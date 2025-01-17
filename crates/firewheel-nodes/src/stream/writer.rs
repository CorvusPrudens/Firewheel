use std::{
    num::NonZeroU32,
    ops::Range,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    collector::ArcGc,
    dsp::declick::{Declicker, FadeType},
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    SilenceMask,
};
use rtrb::CopyToUninit;

use super::ActiveStreamNodeInfo;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamWriterConfig {
    /// The sample rate of the stream.
    ///
    /// By default this is set to `44100`.
    pub sample_rate: NonZeroU32,

    /// The latency of the stream in seconds.
    ///
    /// Lower values will improve latency, but may increase the likelihood
    /// of dropped samples.
    ///
    /// By default this is set to `0.04` (40ms).
    pub latency_seconds: f32,

    /// The size of the buffer in seconds.
    ///
    /// This must be at least twice as large as `latency_seconds`.
    ///
    /// Lower values will use less memory, but may increase the likelihood
    /// of dropped samples and will limit how many samples you can push
    /// to the buffer at once.
    ///
    /// By default this is set to `2.0` (2 seconds).
    pub buffer_size_seconds: f32,

    pub declick_pause_resume: bool,
    pub declick_underflows: bool,
}

impl Default for StreamWriterConfig {
    fn default() -> Self {
        Self {
            sample_rate: NonZeroU32::new(44100).unwrap(),
            latency_seconds: 0.04,
            buffer_size_seconds: 2.0,
            declick_pause_resume: true,
            declick_underflows: true,
        }
    }
}

#[repr(u32)]
#[derive(Clone)]
pub enum StreamWriterEvent {
    /// Pause the stream.
    Pause = 0,
    /// Resume the stream.
    ///
    /// Note, you should always make sure that the buffer is filled with at least
    /// [`ActiveStreamNodeInfo::latency_frames`] frames before sending this event,
    /// or else underflows may occur.
    Resume,
    /// Stop the stream and discard all future samples in the buffer.
    Stop,
}

impl Into<NodeEventType> for StreamWriterEvent {
    fn into(self) -> NodeEventType {
        NodeEventType::U32Param {
            id: 0,
            value: self as u32,
        }
    }
}

impl StreamWriterEvent {
    fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::Pause),
            1 => Some(Self::Resume),
            2 => Some(Self::Stop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum WriteError {
    #[error("An audio stream is not currently active")]
    NotActive,
    #[error("The buffer is full (overflow)")]
    BufferFull,
}

#[derive(Clone)]
pub struct StreamNodeWriter<const NUM_CHANNELS: usize> {
    config: StreamWriterConfig,
    active_state: Arc<Mutex<Option<WriterActiveState>>>,
    sample_rate: NonZeroU32,
    dropped_frames_counter: ArcGc<AtomicU64>,
}

impl<const NUM_CHANNELS: usize> StreamNodeWriter<NUM_CHANNELS> {
    pub fn new(config: StreamWriterConfig, sample_rate: NonZeroU32) -> Self {
        assert_ne!(NUM_CHANNELS, 0);
        assert!(NUM_CHANNELS <= 64);
        assert!(config.latency_seconds > 0.0);
        assert!(config.buffer_size_seconds >= config.latency_seconds * 2.0);

        let active_state = Arc::new(Mutex::new(None));

        Self {
            config,
            active_state,
            sample_rate,
            dropped_frames_counter: ArcGc::new(AtomicU64::new(0)),
        }
    }

    pub fn config(&self) -> &StreamWriterConfig {
        &self.config
    }

    pub fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames_counter.load(Ordering::Relaxed)
    }

    pub fn is_active(&self) -> bool {
        self.poll_abandoned()
    }

    pub fn stream_info(&self) -> Option<ActiveStreamNodeInfo> {
        self.active_state.lock().unwrap().as_ref().map(|s| s.info)
    }

    pub fn get_event(command: StreamWriterEvent) -> NodeEventType {
        command.into()
    }

    /// The maximum number of frames (samples in a single channel of audio)
    /// that can be pushed to [`StreamNodeWriter::write`].
    pub fn available_frames(&self) -> usize {
        self.active_state
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| {
                if s.buffer_tx.is_abandoned() {
                    None
                } else {
                    Some(s.buffer_tx.slots() / NUM_CHANNELS)
                }
            })
            .unwrap_or(0)
    }

    /// Write the interleaved data into the buffer.
    ///
    /// On success, returns the number of frames (samples in a single channel of audio)
    /// that were written to the buffer.
    ///
    /// # Panics
    ///
    /// Panics if `data.len()` is not a multiple of the number of channels.
    pub fn write(&mut self, data: &[f32]) -> Result<usize, WriteError> {
        assert_eq!(data.len() % NUM_CHANNELS, 0);

        if !self.poll_abandoned() {
            return Err(WriteError::NotActive);
        }

        let mut state = self.active_state.lock().unwrap();
        let Some(state) = state.as_mut() else {
            return Err(WriteError::NotActive);
        };

        if state.buffer_tx.is_full() {
            return Err(WriteError::BufferFull);
        }

        let slots = data.len().min(state.buffer_tx.slots());

        let mut writer = state.buffer_tx.write_chunk_uninit(slots).unwrap();
        let (slice1, slice2) = writer.as_mut_slices();

        data[0..slice1.len()].copy_to_uninit(slice1);
        if slice2.len() > 0 {
            data[slice1.len()..slice1.len() + slice2.len()].copy_to_uninit(slice2);
        }

        // # SAFETY:
        // All slots in the writer have been initialized with data.
        unsafe {
            writer.commit_all();
        }

        Ok(slots / NUM_CHANNELS)
    }

    fn poll_abandoned(&self) -> bool {
        let mut state = self.active_state.lock().unwrap();

        if let Some(state_mut) = state.as_mut() {
            if state_mut.buffer_tx.is_abandoned() {
                *state = None;
            }
        }

        state.is_some()
    }
}

impl<const NUM_CHANNELS: usize> AudioNodeConstructor for StreamNodeWriter<NUM_CHANNELS> {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "input_stream",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: ChannelCount::new(NUM_CHANNELS as u32).unwrap(),
            },
            uses_events: true,
        }
    }

    fn processor(
        &mut self,
        stream_info: &firewheel_core::StreamInfo,
    ) -> Box<dyn AudioNodeProcessor> {
        let latency_frames =
            (self.config.latency_seconds * stream_info.sample_rate.get() as f32).round() as usize;
        let capacity_frames = (self.config.buffer_size_seconds
            * stream_info.sample_rate.get() as f32)
            .round() as usize;

        let (buffer_tx, buffer_rx) = rtrb::RingBuffer::new(capacity_frames * NUM_CHANNELS);
        // Sanity check, our DSP won't work if this is not true.
        assert_eq!(
            buffer_tx.buffer().capacity(),
            capacity_frames * NUM_CHANNELS
        );

        let mut active_state = self.active_state.lock().unwrap();
        *active_state = Some(WriterActiveState {
            buffer_tx,
            info: ActiveStreamNodeInfo {
                stream_sample_rate: stream_info.sample_rate,
                latency_frames,
                capacity_frames,
            },
        });

        let (pause_declicker, resume_declicker) = if self.config.declick_pause_resume {
            (Some(Declicker::SettledAt0), Some(Declicker::SettledAt0))
        } else {
            (None, None)
        };

        let underflow_declicker = if self.config.declick_underflows {
            Some(Declicker::SettledAt0)
        } else {
            None
        };

        Box::new(StreamNodeWriterProcessor::<NUM_CHANNELS> {
            buffer_rx,
            paused: false,
            queue_resume_declick: true,
            pause_declicker,
            resume_declicker,
            underflow_declicker,
            last_samples: [0.0; NUM_CHANNELS],
            last_pause_samples: [0.0; NUM_CHANNELS],
            last_underflow_samples: [0.0; NUM_CHANNELS],
            dropped_frames_counter: ArcGc::clone(&self.dropped_frames_counter),
        })
    }
}

struct WriterActiveState {
    buffer_tx: rtrb::Producer<f32>,
    info: ActiveStreamNodeInfo,
}

struct StreamNodeWriterProcessor<const NUM_CHANNELS: usize> {
    buffer_rx: rtrb::Consumer<f32>,
    paused: bool,
    queue_resume_declick: bool,
    pause_declicker: Option<Declicker>,
    resume_declicker: Option<Declicker>,
    underflow_declicker: Option<Declicker>,
    last_samples: [f32; NUM_CHANNELS],
    last_pause_samples: [f32; NUM_CHANNELS],
    last_underflow_samples: [f32; NUM_CHANNELS],
    dropped_frames_counter: ArcGc<AtomicU64>,
}

impl<const NUM_CHANNELS: usize> AudioNodeProcessor for StreamNodeWriterProcessor<NUM_CHANNELS> {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        events.for_each(|event| {
            if let NodeEventType::U32Param { id, value } = event {
                if *id != 0 {
                    return;
                }

                let Some(event) = StreamWriterEvent::from_u32(*value) else {
                    return;
                };

                match event {
                    StreamWriterEvent::Pause => {
                        if !self.paused {
                            self.paused = true;

                            if let Some(pause_declicker) = &mut self.pause_declicker {
                                pause_declicker.reset_to_1();
                                pause_declicker.fade_to_0(&proc_info.declick_values);
                            }

                            self.last_pause_samples = self.last_samples;
                        }
                    }
                    StreamWriterEvent::Resume => {
                        if self.paused {
                            self.paused = false;
                            self.queue_resume_declick = true;
                        }
                    }
                    StreamWriterEvent::Stop => {
                        // Discard all future samples from the buffer.
                        let slots = self.buffer_rx.slots();
                        let chunk = self.buffer_rx.read_chunk(slots).unwrap();
                        chunk.commit_all();

                        if !self.paused {
                            self.paused = true;

                            if let Some(pause_declicker) = &mut self.pause_declicker {
                                pause_declicker.reset_to_1();
                                pause_declicker.fade_to_0(&proc_info.declick_values);
                            }

                            self.last_pause_samples = self.last_samples;
                        }
                    }
                }
            }
        });

        if self.paused
            && self
                .pause_declicker
                .map(|d| d == Declicker::SettledAt0)
                .unwrap_or(true)
        {
            return ProcessStatus::ClearAllOutputs;
        }

        let mut declick_to_zero =
            |range: Range<usize>,
             outputs: &mut [&mut [f32]],
             declicker: &mut Declicker,
             last_samples: &[f32],
             silence_mask: &mut SilenceMask| {
                // Apply a filter to the last played samples to declick.
                //
                // While this is not technically the best way to declick, because this
                // is a stream, we don't always have access to future samples in order
                // to declick the proper way.

                declicker.process(
                    &mut [&mut scratch_buffers[0]],
                    0..range.end - range.start,
                    proc_info.declick_values,
                    1.0,
                    FadeType::EqualPower3dB,
                );

                let mut sm = SilenceMask::NONE_SILENT;

                for (ch_i, (out_ch, &last_sample)) in
                    outputs.iter_mut().zip(last_samples.iter()).enumerate()
                {
                    if last_sample == 0.0 {
                        sm.set_channel(ch_i, true);
                    } else {
                        for (os, &gain) in out_ch[range.clone()]
                            .iter_mut()
                            .zip(scratch_buffers[0].iter())
                        {
                            *os += last_sample * gain;
                        }
                    }
                }

                *silence_mask = SilenceMask(silence_mask.0 & sm.0);
            };

        let mut silence_mask = SilenceMask::new_all_silent(NUM_CHANNELS);

        let mut underflow_declick_start_frame = 0;

        if !self.paused {
            // Fill the output with samples.

            if self.queue_resume_declick {
                self.queue_resume_declick = false;

                if let Some(resume_declicker) = &mut self.resume_declicker {
                    resume_declicker.reset_to_0();
                    resume_declicker.fade_to_1(proc_info.declick_values);
                }
            }

            let slot_frames = self.buffer_rx.slots() / NUM_CHANNELS;
            let fill_frames = proc_info.frames.min(slot_frames);

            if fill_frames > 0 {
                let reader = self
                    .buffer_rx
                    .read_chunk(fill_frames * NUM_CHANNELS)
                    .unwrap();

                let (slice1, slice2) = reader.as_slices();

                let silence_mask_1 = firewheel_core::dsp::interleave::deinterleave(
                    outputs,
                    0,
                    slice1,
                    NUM_CHANNELS,
                    true,
                );

                silence_mask = SilenceMask(silence_mask.0 & silence_mask_1.0);

                if !slice2.is_empty() {
                    let silence_mask_2 = firewheel_core::dsp::interleave::deinterleave(
                        outputs,
                        slice1.len() / NUM_CHANNELS,
                        slice2,
                        NUM_CHANNELS,
                        true,
                    );

                    silence_mask = SilenceMask(silence_mask.0 & silence_mask_2.0);
                }

                reader.commit_all();

                if let Some(resume_declicker) = &mut self.resume_declicker {
                    if !resume_declicker.is_settled() {
                        resume_declicker.process(
                            outputs,
                            0..fill_frames,
                            proc_info.declick_values,
                            1.0,
                            FadeType::EqualPower3dB,
                        );
                    }
                }

                for (last_sample, ch) in self.last_samples.iter_mut().zip(outputs.iter()) {
                    *last_sample = ch[fill_frames - 1];
                }
            }

            if fill_frames < proc_info.frames {
                // Underflow occured.

                let dropped_frames = self.dropped_frames_counter.load(Ordering::Relaxed);
                self.dropped_frames_counter.store(
                    dropped_frames + (proc_info.frames - fill_frames) as u64,
                    Ordering::Relaxed,
                );

                self.queue_resume_declick = true;

                for b in outputs.iter_mut() {
                    b[fill_frames..].fill(0.0);
                }

                if slot_frames != 0 {
                    if let Some(underflow_declicker) = &mut self.underflow_declicker {
                        underflow_declicker.reset_to_1();
                        underflow_declicker.fade_to_0(&proc_info.declick_values);
                    }

                    self.last_underflow_samples = self.last_samples;

                    underflow_declick_start_frame = fill_frames;
                }
            }
        } else {
            for (ch_i, b) in outputs.iter_mut().enumerate() {
                if !proc_info.out_silence_mask.is_channel_silent(ch_i) {
                    b.fill(0.0);
                }
            }
        }

        if let Some(pause_declicker) = &mut self.pause_declicker {
            if !pause_declicker.is_settled() {
                declick_to_zero(
                    0..proc_info.frames,
                    outputs,
                    pause_declicker,
                    &self.last_pause_samples,
                    &mut silence_mask,
                );
            }
        }

        if let Some(underflow_declicker) = &mut self.underflow_declicker {
            if !underflow_declicker.is_settled() {
                declick_to_zero(
                    underflow_declick_start_frame..proc_info.frames,
                    outputs,
                    underflow_declicker,
                    &self.last_underflow_samples,
                    &mut silence_mask,
                );
            }
        }

        ProcessStatus::OutputsModified {
            out_silence_mask: silence_mask,
        }
    }
}
