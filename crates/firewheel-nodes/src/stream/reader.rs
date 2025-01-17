use std::{
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    collector::ArcGc,
    dsp::declick::{Declicker, FadeType},
    event::NodeEventList,
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    SilenceMask,
};

use super::ActiveStreamNodeInfo;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamReaderConfig {
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
}

impl Default for StreamReaderConfig {
    fn default() -> Self {
        Self {
            sample_rate: NonZeroU32::new(44100).unwrap(),
            latency_seconds: 0.04,
            buffer_size_seconds: 2.0,
            declick_pause_resume: true,
        }
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum ReadError {
    #[error("An audio stream is not currently active")]
    NotActive,
    #[error("The buffer has not been filled with the initial frames yet")]
    NotReadyYet,
}

#[derive(Clone)]
pub struct StreamNodeReader<const NUM_CHANNELS: usize> {
    config: StreamReaderConfig,
    active_state: Arc<Mutex<Option<ReaderActiveState>>>,
    sample_rate: NonZeroU32,
    shared_state: ArcGc<ReaderSharedState>,
    wait_for_initial_fill: bool,
    dropped_frames_due_to_underflow: u64,
    stopped: bool,
}

impl<const NUM_CHANNELS: usize> StreamNodeReader<NUM_CHANNELS> {
    pub fn new(config: StreamReaderConfig, sample_rate: NonZeroU32) -> Self {
        assert_ne!(NUM_CHANNELS, 0);
        assert!(NUM_CHANNELS <= NUM_SCRATCH_BUFFERS);
        assert!(config.latency_seconds > 0.0);
        assert!(config.buffer_size_seconds >= config.latency_seconds * 2.0);

        let active_state = Arc::new(Mutex::new(None));

        Self {
            config,
            active_state,
            sample_rate,
            shared_state: ArcGc::new(ReaderSharedState::new(true)),
            wait_for_initial_fill: true,
            dropped_frames_due_to_underflow: 0,
            stopped: true,
        }
    }

    pub fn config(&self) -> &StreamReaderConfig {
        &self.config
    }

    pub fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    pub fn dropped_frames_due_to_overflow(&self) -> u64 {
        self.shared_state
            .dropped_frames_counter
            .load(Ordering::Relaxed)
    }

    pub fn dropped_frames_due_to_underflow(&self) -> u64 {
        self.dropped_frames_due_to_underflow
    }

    pub fn paused(&self) -> bool {
        self.shared_state.paused.load(Ordering::Relaxed)
    }

    pub fn pause(&mut self) {
        if self.stopped {
            return;
        }

        if self.config.declick_pause_resume {
            self.shared_state
                .finished_pausing
                .store(false, Ordering::Relaxed);
        }

        self.shared_state.paused.store(true, Ordering::Relaxed);
    }

    pub fn start_or_resume(&mut self) -> Result<(), ReadError> {
        if !self.poll_abandoned() {
            return Err(ReadError::NotActive);
        }

        if self.stopped {
            self.stopped = false;

            // Clear any remaining samples in the buffer from the last stream.
            let mut state = self.active_state.lock().unwrap();
            let state = state.as_mut().unwrap();
            let slots = state.buffer_rx.slots();
            let reader = state.buffer_rx.read_chunk(slots).unwrap();
            reader.commit_all();

            self.wait_for_initial_fill = true;
        }

        self.shared_state.paused.store(false, Ordering::Relaxed);

        Ok(())
    }

    pub fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;

            self.shared_state
                .finished_pausing
                .store(false, Ordering::Relaxed);
            self.shared_state.paused.store(true, Ordering::Relaxed);
        }
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn finished_pausing(&self) -> bool {
        self.shared_state.finished_pausing.load(Ordering::Relaxed)
    }

    pub fn stream_info(&self) -> Option<ActiveStreamNodeInfo> {
        self.active_state.lock().unwrap().as_ref().map(|s| s.info)
    }

    /// The number of frames (samples in a single channel of audio) available to read
    /// in [`StreamNodeReader::read`].
    pub fn available_frames(&mut self) -> usize {
        self.poll_abandoned();

        self.active_state
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| {
                if s.buffer_rx.is_abandoned() {
                    None
                } else {
                    let frames = s.buffer_rx.slots() / NUM_CHANNELS;

                    if self.wait_for_initial_fill && frames >= s.latency_frames {
                        Some(frames)
                    } else {
                        None
                    }
                }
            })
            .unwrap_or(0)
    }

    pub fn is_ready(&mut self) -> bool {
        self.available_frames() > 0
    }

    /// Read the available data and write it to the given interleaved buffer.
    ///
    /// On success, returns the number of frames (samples in a single channel of audio)
    /// that were written to the buffer.
    pub fn read(&mut self, buffer: &mut [f32]) -> Result<usize, ReadError> {
        if !self.poll_abandoned() {
            return Err(ReadError::NotActive);
        }

        let mut state = self.active_state.lock().unwrap();
        let Some(state) = state.as_mut() else {
            return Err(ReadError::NotActive);
        };

        let buffer_frames = buffer.len() / NUM_CHANNELS;

        let slots = state.buffer_rx.slots();
        let slot_frames = slots / NUM_CHANNELS;
        let read_slots = (buffer_frames * NUM_CHANNELS).min(slots);
        let read_frames = read_slots / NUM_CHANNELS;

        if self.wait_for_initial_fill && slot_frames < state.latency_frames {
            return Err(ReadError::NotReadyYet);
        }
        self.wait_for_initial_fill = false;

        let reader = state.buffer_rx.read_chunk(read_slots).unwrap();
        let (slice1, slice2) = reader.as_slices();

        buffer[..slice1.len()].copy_from_slice(slice1);
        if slice2.len() > 0 {
            buffer[slice1.len()..slice1.len() + slice2.len()].copy_from_slice(slice2);
        }

        reader.commit_all();

        if read_frames < buffer_frames {
            self.dropped_frames_due_to_underflow += (buffer_frames - read_frames) as u64;
        }

        Ok(read_frames)
    }

    fn poll_abandoned(&mut self) -> bool {
        let mut state = self.active_state.lock().unwrap();

        if let Some(state_mut) = state.as_mut() {
            if state_mut.buffer_rx.is_abandoned() {
                *state = None;
            }
        }

        if state.is_none() {
            self.stopped = true;
        }

        state.is_some()
    }
}

impl<const NUM_CHANNELS: usize> AudioNodeConstructor for StreamNodeReader<NUM_CHANNELS> {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "output_stream",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::new(NUM_CHANNELS as u32).unwrap(),
                num_outputs: ChannelCount::ZERO,
            },
            uses_events: false,
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
        *active_state = Some(ReaderActiveState {
            buffer_rx,
            info: ActiveStreamNodeInfo {
                stream_sample_rate: stream_info.sample_rate,
                latency_frames,
                capacity_frames,
            },
            latency_frames,
        });

        let paused = self.shared_state.paused.load(Ordering::Relaxed);
        let declicker = if paused {
            Declicker::SettledAt0
        } else {
            Declicker::SettledAt1
        };

        Box::new(StreamNodeReaderProcessor::<NUM_CHANNELS> {
            buffer_tx,
            declicker,
            shared_state: ArcGc::clone(&self.shared_state),
            declick_pause_resume: self.config.declick_pause_resume,
            paused,
        })
    }
}

struct ReaderActiveState {
    buffer_rx: rtrb::Consumer<f32>,
    info: ActiveStreamNodeInfo,
    latency_frames: usize,
}

struct ReaderSharedState {
    dropped_frames_counter: AtomicU64,
    paused: AtomicBool,
    finished_pausing: AtomicBool,
}

impl ReaderSharedState {
    fn new(paused: bool) -> Self {
        Self {
            dropped_frames_counter: AtomicU64::new(0),
            paused: AtomicBool::new(paused),
            finished_pausing: AtomicBool::new(true),
        }
    }
}

struct StreamNodeReaderProcessor<const NUM_CHANNELS: usize> {
    buffer_tx: rtrb::Producer<f32>,
    declicker: Declicker,
    shared_state: ArcGc<ReaderSharedState>,
    declick_pause_resume: bool,
    paused: bool,
}

impl<const NUM_CHANNELS: usize> AudioNodeProcessor for StreamNodeReaderProcessor<NUM_CHANNELS> {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        proc_info: &ProcInfo,
        scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        let paused = self.shared_state.paused.load(Ordering::Relaxed);
        if self.paused != paused {
            self.paused = paused;

            if paused {
                if self.declick_pause_resume {
                    self.declicker.fade_to_0(proc_info.declick_values);
                    self.shared_state
                        .finished_pausing
                        .store(false, Ordering::Relaxed);
                } else {
                    self.declicker.reset_to_0();
                    self.shared_state
                        .finished_pausing
                        .store(true, Ordering::Relaxed);
                }
            } else {
                if self.declick_pause_resume {
                    self.declicker.fade_to_1(proc_info.declick_values);
                } else {
                    self.declicker.reset_to_1();
                }
            }
        }

        if !self.paused && !self.shared_state.finished_pausing.load(Ordering::Relaxed) {
            self.shared_state
                .finished_pausing
                .store(true, Ordering::Relaxed);
        }

        match self.declicker {
            Declicker::SettledAt0 => {}
            Declicker::SettledAt1 => {
                write_samples::<_, NUM_CHANNELS>(
                    inputs,
                    proc_info.frames,
                    &mut self.buffer_tx,
                    proc_info.in_silence_mask,
                    &self.shared_state,
                );
            }
            _ => {
                let declick_frames = self.declicker.frames_left().min(proc_info.frames);

                for (in_ch, sc_ch) in inputs[..NUM_CHANNELS]
                    .iter()
                    .zip(scratch_buffers.iter_mut())
                {
                    sc_ch[..declick_frames].copy_from_slice(&in_ch[..declick_frames]);
                }

                self.declicker.process(
                    scratch_buffers,
                    0..declick_frames,
                    proc_info.declick_values,
                    1.0,
                    FadeType::EqualPower3dB,
                );

                write_samples::<_, NUM_CHANNELS>(
                    scratch_buffers,
                    declick_frames,
                    &mut self.buffer_tx,
                    proc_info.in_silence_mask,
                    &self.shared_state,
                );

                if self.declicker == Declicker::SettledAt0 {
                    self.shared_state
                        .finished_pausing
                        .store(true, Ordering::Relaxed);
                }
            }
        }

        ProcessStatus::Bypass
    }
}

fn write_samples<V: AsRef<[f32]>, const NUM_CHANNELS: usize>(
    inputs: &[V],
    frames: usize,
    buffer_tx: &mut rtrb::Producer<f32>,
    in_silence_mask: SilenceMask,
    shared_state: &ArcGc<ReaderSharedState>,
) {
    let slots = (frames * NUM_CHANNELS).min(buffer_tx.slots());

    let mut write_slots = buffer_tx.write_chunk(slots).unwrap();

    let (slice1, slice2) = write_slots.as_mut_slices();

    firewheel_core::dsp::interleave::interleave(
        inputs,
        0,
        slice1,
        NUM_CHANNELS,
        Some(in_silence_mask),
    );

    if !slice2.is_empty() {
        firewheel_core::dsp::interleave::interleave(
            inputs,
            slice1.len() / NUM_CHANNELS,
            slice2,
            NUM_CHANNELS,
            Some(in_silence_mask),
        );
    }

    write_slots.commit_all();

    if slots / NUM_CHANNELS < frames {
        let dropped_frames = shared_state.dropped_frames_counter.load(Ordering::Relaxed);
        shared_state.dropped_frames_counter.store(
            dropped_frames + (frames - (slots / NUM_CHANNELS)) as u64,
            Ordering::Relaxed,
        );
    }
}
