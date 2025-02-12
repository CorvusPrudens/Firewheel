use std::{
    num::NonZeroU32,
    ops::Range,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount, NonZeroChannelCount},
    dsp::declick::{Declicker, FadeType},
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    SilenceMask, StreamInfo,
};
use fixed_resample::{ReadStatus, ResamplingChannelConfig};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamWriterConfig {
    /// The configuration of the input to output channel.
    pub channel_config: ResamplingChannelConfig,

    /// If the input stream is running faster than the output stream by this
    /// amount in seconds, then discard samples to reduce the percieved
    /// glitchiness due to excessive overflows.
    ///
    /// This can happen if there are a lot of underruns occuring in the
    /// output audio thread.
    ///
    /// If this is `None`, then the threshold will be the entire capacity of
    /// the channel.
    ///
    /// By default this is set to `None`.
    pub discard_jitter_threshold_seconds: Option<f64>,

    /// Whether or not to check for silence in the input stream. Highly
    /// recommened to set this to `true` to improve audio graph performance
    /// when there is no input on the microphone.
    ///
    /// By default this is set to `true`.
    pub check_for_silence: bool,
}

impl Default for StreamWriterConfig {
    fn default() -> Self {
        Self {
            channel_config: ResamplingChannelConfig::default(),
            discard_jitter_threshold_seconds: None,
            check_for_silence: true,
        }
    }
}

pub struct StreamWriterHandle {
    /// The configuration of the stream.
    ///
    /// Changing this will have no effect until a new stream is started.
    pub config: StreamWriterConfig,

    channels: NonZeroChannelCount,
    active_state: Option<ActiveState>,
    shared_state: Arc<SharedState>,
}

impl StreamWriterHandle {
    pub fn new(config: StreamWriterConfig, channels: NonZeroChannelCount) -> Self {
        Self {
            config,
            channels,
            active_state: None,
            shared_state: Arc::new(SharedState::new()),
        }
    }

    pub fn constructor(&self) -> Constructor {
        Constructor {
            shared_state: Arc::clone(&self.shared_state),
            config: self.config.clone(),
            channels: self.channels,
        }
    }

    /// The number of channels in this node.
    pub fn channels(&self) -> NonZeroChannelCount {
        self.channels
    }

    /// The sample rate of the active stream.
    ///
    /// Returns `None` if there is no active stream.
    pub fn sample_rate(&self) -> Option<NonZeroU32> {
        self.active_state.as_ref().map(|s| s.sample_rate)
    }

    /// Returns `true` if there is there is currently an active stream on this node.
    pub fn is_active(&self) -> bool {
        self.active_state.is_some() && self.shared_state.stream_active.load(Ordering::Relaxed)
    }

    /// Returns `true` if an underflow occured (due to the output stream
    /// running faster than the input stream).
    ///
    /// If this happens excessively in Release mode, you may want to consider
    /// increasing [`StreamWriterConfig::channel_config.latency_seconds`].
    ///
    /// (Calling this will also reset the flag indicating whether an
    /// underflow occurred.)
    pub fn underflow_occurred(&self) -> bool {
        self.shared_state
            .underflow_occurred
            .swap(false, Ordering::Relaxed)
    }

    /// Returns `true` if an overflow occured (due to the input stream
    /// running faster than the output stream).
    ///
    /// If this happens excessively in Release mode, you may want to consider
    /// increasing [`StreamWriterConfig::channel_config.capacity_seconds`]. For
    /// example, if you are streaming data from a network, you may want to
    /// increase the capacity to several seconds.
    ///
    /// (Calling this will also reset the flag indicating whether an
    /// overflow occurred.)
    pub fn overflow_occurred(&self) -> bool {
        self.shared_state
            .overflow_occurred
            .swap(false, Ordering::Relaxed)
    }

    /// An number describing the current amount of jitter in seconds between the input and
    /// output streams. A value of 0.0 means the two channels are perfectly synced, a value
    /// less than 0.0 means the input channel is slower than the input channel, and a value
    /// greater than 0.0 means the input channel is faster than the output channel.
    ///
    /// This value can be used to correct for jitter and avoid underflows/overflows. For
    /// example, if this value goes below a certain threshold, then you can push an extra
    /// packet of data to correct for the jitter.
    ///
    /// This number will be in the range `[-latency_seconds, capacity_seconds - latency_seconds]`,
    /// where `latency_seconds` and `capacity_seconds` are the values passed in
    /// `ResamplingChannelConfig` when this channel was constructed.
    ///
    /// Note, it is typical for the jitter value to be around plus or minus the size of a
    /// packet of pushed/read data even when the streams are perfectly in sync).
    ///
    /// Returns `None` if there is no active stream.
    pub fn jitter_seconds(&self) -> Option<f64> {
        self.active_state.as_ref().map(|s| s.prod.jitter_seconds())
    }

    /// The total number of frames (not samples) that can currently be pushed to the stream.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to receive samples, then this will return `0`.
    pub fn available_frames(&self) -> usize {
        if self.is_ready() {
            self.active_state
                .as_ref()
                .map(|s| s.prod.available_frames())
                .unwrap_or(0)
        } else {
            0
        }
    }

    /// Begin the input audio stream on this node.
    ///
    /// The returned event must be sent to the node's processor for this to take effect.
    ///
    /// * `sample_rate` - The sample rate of this node.
    /// * `output_stream_sample_rate` - The sample rate of the active output audio stream.
    ///
    /// If there is already an active stream running on this node, then this will return
    /// an error.
    pub fn start_stream(
        &mut self,
        sample_rate: NonZeroU32,
        output_stream_sample_rate: NonZeroU32,
    ) -> Result<NewInputStreamEvent, ()> {
        if self.is_active() {
            return Err(());
        }

        self.shared_state.reset();

        let (prod, cons) = fixed_resample::resampling_channel::<f32>(
            sample_rate.get(),
            output_stream_sample_rate.get(),
            self.channels.get().get() as usize,
            self.config.channel_config,
        );

        self.active_state = Some(ActiveState { prod, sample_rate });
        self.shared_state
            .stream_active
            .store(true, Ordering::Relaxed);

        Ok(NewInputStreamEvent { cons: Some(cons) })
    }

    /// Push the given data in interleaved format.
    ///
    /// Returns the number of frames (not samples) that were successfully pushed.
    /// If this number is less than the number of frames in `data`, then it means
    /// an overflow has occured.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to receive samples, then no data will be sent and this will
    /// return `0`.
    pub fn push_interleaved(&mut self, data: &[f32]) -> usize {
        if !self.is_ready() {
            return 0;
        }

        self.active_state
            .as_mut()
            .unwrap()
            .prod
            .push_interleaved(data)
    }

    /// Push the given data in de-interleaved format.
    ///
    /// * `data` - The channels of data to push to the channel.
    /// * `range` - The range in each slice in `input` to read data from.
    ///
    /// Returns the number of frames (not samples) that were successfully pushed.
    /// If this number is less than the number of frames in `data`, then it means
    /// an overflow has occured.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to receive samples, then no data will be sent and this will
    /// return `0`.
    pub fn push<Vin: AsRef<[f32]>>(&mut self, data: &[Vin], range: Range<usize>) -> usize {
        if !self.is_ready() {
            return 0;
        }

        self.active_state.as_mut().unwrap().prod.push(data, range)
    }

    /// Returns `true` if the processor end of the stream is ready to start receiving
    /// data.
    pub fn is_ready(&self) -> bool {
        self.active_state.is_some()
            && self.shared_state.channel_started.load(Ordering::Relaxed)
            && !self.shared_state.paused.load(Ordering::Relaxed)
    }

    /// Pause any active audio streams.
    pub fn pause_stream(&mut self) {
        if self.is_active() {
            self.shared_state.paused.store(true, Ordering::Relaxed);
        }
    }

    /// Resume any active audio streams after pausing.
    pub fn resume(&mut self) {
        self.shared_state.paused.store(false, Ordering::Relaxed);
    }

    // Stop any active audio input streams.
    pub fn stop_stream(&mut self) {
        self.active_state = None;
        self.shared_state.reset();
    }
}

impl Drop for StreamWriterHandle {
    fn drop(&mut self) {
        self.stop_stream();
    }
}

#[derive(Clone)]
pub struct Constructor {
    shared_state: Arc<SharedState>,
    config: StreamWriterConfig,
    channels: NonZeroChannelCount,
}

impl AudioNodeConstructor for Constructor {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "stream_input",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: self.channels.get(),
            },
            uses_events: true,
        }
    }

    fn processor(&mut self, _stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(Processor {
            cons: None,
            shared_state: Arc::clone(&self.shared_state),
            discard_jitter_threshold_seconds: self.config.discard_jitter_threshold_seconds,
            check_for_silence: self.config.check_for_silence,
            pause_declicker: Declicker::SettledAt0,
        })
    }
}

struct ActiveState {
    prod: fixed_resample::ResamplingProd<f32>,
    sample_rate: NonZeroU32,
}

struct SharedState {
    stream_active: AtomicBool,
    channel_started: AtomicBool,
    paused: AtomicBool,
    underflow_occurred: AtomicBool,
    overflow_occurred: AtomicBool,
}

impl SharedState {
    fn new() -> Self {
        Self {
            stream_active: AtomicBool::new(false),
            channel_started: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            underflow_occurred: AtomicBool::new(false),
            overflow_occurred: AtomicBool::new(false),
        }
    }

    fn reset(&self) {
        self.stream_active.store(false, Ordering::Relaxed);
        self.channel_started.store(false, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
        self.underflow_occurred.store(false, Ordering::Relaxed);
        self.overflow_occurred.store(false, Ordering::Relaxed);
    }
}

struct Processor {
    cons: Option<fixed_resample::ResamplingCons<f32>>,
    shared_state: Arc<SharedState>,
    discard_jitter_threshold_seconds: Option<f64>,
    check_for_silence: bool,
    pause_declicker: Declicker,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        events.for_each(|event| {
            if let NodeEventType::Custom(event) = event {
                if let Some(in_stream_event) = event.downcast_mut::<NewInputStreamEvent>() {
                    // Swap the memory so that the old channel will be properly
                    // dropped outside of the audio thread.
                    std::mem::swap(&mut self.cons, &mut in_stream_event.cons);
                }
            }
        });

        let enabled = self.shared_state.stream_active.load(Ordering::Relaxed)
            && !self.shared_state.paused.load(Ordering::Relaxed);

        self.pause_declicker
            .fade_to_enabled(enabled, proc_info.declick_values);

        if self.pause_declicker.disabled() {
            return ProcessStatus::ClearAllOutputs;
        }

        let Some(cons) = &mut self.cons else {
            self.pause_declicker.reset_to_0();
            return ProcessStatus::ClearAllOutputs;
        };

        // Notify the input stream that the output stream has begun
        // reading data.
        self.shared_state
            .channel_started
            .store(true, Ordering::Relaxed);

        if let Some(threshold) = self.discard_jitter_threshold_seconds {
            let num_discarded_samples = cons.discard_jitter(threshold);
            if num_discarded_samples > 0 {
                self.shared_state
                    .overflow_occurred
                    .store(true, Ordering::Relaxed);
            }
        }

        match cons.read(outputs, 0..proc_info.frames) {
            ReadStatus::Ok => {}
            ReadStatus::Underflow => {
                self.shared_state
                    .underflow_occurred
                    .store(true, Ordering::Relaxed);
            }
            ReadStatus::WaitingForFrames => {
                self.pause_declicker.reset_to_target();
                return ProcessStatus::outputs_modified(SilenceMask::new_all_silent(outputs.len()));
            }
        }

        if !self.pause_declicker.is_settled() {
            self.pause_declicker.process(
                outputs,
                0..proc_info.frames,
                proc_info.declick_values,
                1.0,
                FadeType::EqualPower3dB,
            );
        }

        let mut silence_mask = SilenceMask::NONE_SILENT;
        if self.check_for_silence {
            let resampler_channels = cons.num_channels().get();

            for (ch_i, ch) in outputs.iter().enumerate() {
                if ch_i >= resampler_channels {
                    // `cons.read()` clears any extra channels
                    silence_mask.set_channel(ch_i, true);
                } else {
                    let mut all_silent = true;
                    for &s in ch[..proc_info.frames].iter() {
                        if s != 0.0 {
                            all_silent = false;
                            break;
                        }
                    }

                    if all_silent {
                        silence_mask.set_channel(ch_i, true);
                    }
                }
            }
        }

        ProcessStatus::outputs_modified(silence_mask)
    }

    fn stream_stopped(&mut self) {
        self.shared_state
            .stream_active
            .store(false, Ordering::Relaxed);
        self.cons = None;
        self.pause_declicker.reset_to_0();
    }
}

pub struct NewInputStreamEvent {
    cons: Option<fixed_resample::ResamplingCons<f32>>,
}

impl Into<NodeEventType> for NewInputStreamEvent {
    fn into(self) -> NodeEventType {
        NodeEventType::Custom(Box::new(self))
    }
}
