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
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        ScratchBuffers,
    },
    sync_wrapper::SyncWrapper,
    StreamInfo,
};
use fixed_resample::{ReadStatus, ResamplingChannelConfig};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamReaderConfig {
    /// The configuration of the input to output channel.
    pub channel_config: ResamplingChannelConfig,
}

impl Default for StreamReaderConfig {
    fn default() -> Self {
        Self {
            channel_config: ResamplingChannelConfig::default(),
        }
    }
}

pub struct StreamReaderHandle {
    /// The configuration of the stream.
    ///
    /// Changing this will have no effect until a new stream is started.
    pub config: StreamReaderConfig,

    channels: NonZeroChannelCount,
    active_state: Option<ActiveState>,
    shared_state: Arc<SharedState>,
}

impl StreamReaderHandle {
    pub fn new(config: StreamReaderConfig, channels: NonZeroChannelCount) -> Self {
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
            channels: self.channels,
        }
    }

    /// Returns `true` if there is there is currently an active stream on this node.
    pub fn is_active(&self) -> bool {
        self.active_state.is_some() && self.shared_state.stream_active.load(Ordering::Relaxed)
    }

    /// Returns `true` if an underflow occured (due to the output stream
    /// running faster than the input stream).
    ///
    /// If this happens excessively in Release mode, you may want to consider
    /// increasing [`StreamReaderConfig::channel_config.latency_seconds`].
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
    /// increasing [`StreamReaderConfig::channel_config.capacity_seconds`]. For
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

    /// Begin the output audio stream on this node.
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
    ) -> Result<NewOutputStreamEvent, ()> {
        if self.is_active() {
            return Err(());
        }

        self.shared_state.reset();

        let (prod, cons) = fixed_resample::resampling_channel::<f32>(
            output_stream_sample_rate.get(),
            sample_rate.get(),
            self.channels.get().get() as usize,
            self.config.channel_config,
        );

        self.active_state = Some(ActiveState { cons, sample_rate });
        self.shared_state
            .stream_active
            .store(true, Ordering::Relaxed);

        Ok(NewOutputStreamEvent { prod: Some(prod) })
    }

    /// The total number of frames (not samples) that can currently be read from
    /// the stream.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to receive samples, then this will return `0`.
    pub fn available_frames(&self) -> usize {
        if self.is_ready() {
            self.active_state
                .as_ref()
                .map(|s| s.cons.available_frames())
                .unwrap_or(0)
        } else {
            0
        }
    }

    /// The amount of data in seconds that is currently available to read.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to receive samples, then this will return `0.0`.
    pub fn available_seconds(&self) -> f64 {
        if self.is_ready() {
            self.active_state
                .as_ref()
                .map(|s| s.cons.available_seconds())
                .unwrap_or(0.0)
        } else {
            0.0
        }
    }

    /// The amount of data in seconds that is currently occupied in the channel.
    ///
    /// This value will be in the range `[0.0, ResamplingCons::capacity_seconds()]`.
    ///
    /// This can also be used to detect when an extra packet of data should be read or
    /// discarded to correct for jitter.
    ///
    /// If there is no active stream, then this will return `None`.
    pub fn occupied_seconds(&self) -> Option<f64> {
        self.active_state
            .as_ref()
            .map(|s| s.cons.occupied_seconds())
    }

    /// Returns the number of input frames (samples in a single channel) from the producer
    /// (not output frames from this consumer) that are currently occupied in the channel.
    ///
    /// If there is no active stream, then this will return `None`.
    pub fn occupied_input_frames(&self) -> Option<usize> {
        self.active_state
            .as_ref()
            .map(|s| s.cons.occupied_input_frames())
    }

    /// The value of [`ResamplingChannelConfig::latency_seconds`] that was passed when
    /// this channel was created.
    pub fn latency_seconds(&self) -> f64 {
        self.config.channel_config.latency_seconds
    }

    /// The capacity of the channel in seconds.
    ///
    /// If there is no active stream, then this will return `None`.
    pub fn capacity_seconds(&self) -> Option<f64> {
        self.active_state
            .as_ref()
            .map(|s| s.cons.capacity_seconds())
    }

    /// The capacity of the channel in input frames (samples in a single channel) from
    /// the producer (not output frames from this consumer).
    ///
    /// If there is no active stream, then this will return `None`.
    pub fn capacity_input_frames(&self) -> Option<usize> {
        self.active_state
            .as_ref()
            .map(|s| s.cons.capacity_input_frames())
    }

    /// The number of channels in this node.
    pub fn num_channels(&self) -> NonZeroChannelCount {
        self.channels
    }

    /// The sample rate of the active stream.
    ///
    /// Returns `None` if there is no active stream.
    pub fn sample_rate(&self) -> Option<NonZeroU32> {
        self.active_state.as_ref().map(|s| s.sample_rate)
    }

    /// Read from the channel and write the results into the given output buffer
    /// in interleaved format.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to send samples, then the output will be filled with zeros
    /// and `None` will be returned.
    pub fn read_interleaved(&mut self, output: &mut [f32]) -> Option<ReadStatus> {
        if !self.is_ready() {
            output.fill(0.0);
            return None;
        }

        Some(
            self.active_state
                .as_mut()
                .unwrap()
                .cons
                .read_interleaved(output),
        )
    }

    /// Read from the channel and write the results into the given output buffer in
    /// de-interleaved format.
    ///
    /// * `output` - The channels to write data to.
    /// * `range` - The range in each slice in `output` to write to.
    ///
    /// If there is no active stream, the stream is paused, or the processor end
    /// is not ready to send samples, then the output will be filled with zeros
    /// and `None` will be returned.
    pub fn read<Vin: AsMut<[f32]>>(
        &mut self,
        output: &mut [Vin],
        range: Range<usize>,
    ) -> Option<ReadStatus> {
        if !self.is_ready() {
            for ch in output.iter_mut() {
                ch.as_mut()[range.clone()].fill(0.0);
            }
            return None;
        }

        Some(self.active_state.as_mut().unwrap().cons.read(output, range))
    }

    /// Discard all data currently in the channel.
    ///
    /// Note, you should typically wait for [`StreamReaderHandle::occupied_seconds`]
    /// to be `>=` [`StreamReaderHandle::latency_seconds`] (or for
    /// [`StreamReaderHandle::available_frames`] to be `>=` to the equivalant of
    /// [`StreamReaderHandle::latency_seconds`]) before reading from the channel again.
    ///
    /// Returns the number of input frames that were discarded.
    pub fn discard_all(&mut self) -> usize {
        if let Some(state) = &mut self.active_state {
            state.cons.discard_input_frames(usize::MAX)
        } else {
            0
        }
    }

    /// If the value of [`StreamReaderHandle::occupied_seconds()`] is greater than the
    /// given threshold in seconds, then discard the number of input frames needed to
    /// bring the value back down to [`StreamReaderHandle::latency_seconds()`] to avoid
    /// excessive overflows and reduce perceived audible glitchiness.
    ///
    /// Returns the number of input frames from the producer (not output frames from
    /// this consumer) that were discarded.
    ///
    /// If `threshold_seconds` is less than [`StreamReaderHandle::latency_seconds()`],
    /// then this will do nothing.
    pub fn discard_jitter(&mut self, threshold_seconds: f64) -> usize {
        if let Some(state) = &mut self.active_state {
            state.cons.discard_jitter(threshold_seconds)
        } else {
            0
        }
    }

    /// Returns `true` if the processor end of the stream is ready to start sending
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

impl Drop for StreamReaderHandle {
    fn drop(&mut self) {
        self.stop_stream();
    }
}

#[derive(Clone)]
pub struct Constructor {
    shared_state: Arc<SharedState>,
    channels: NonZeroChannelCount,
}

impl AudioNodeConstructor for Constructor {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "stream_output",
            channel_config: ChannelConfig {
                num_inputs: self.channels.get(),
                num_outputs: ChannelCount::ZERO,
            },
            uses_events: true,
        }
    }

    fn processor(&mut self, _stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(Processor {
            prod: None,
            shared_state: Arc::clone(&self.shared_state),
        })
    }
}

struct ActiveState {
    cons: fixed_resample::ResamplingCons<f32>,
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
    prod: Option<fixed_resample::ResamplingProd<f32>>,
    shared_state: Arc<SharedState>,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: ScratchBuffers,
    ) -> ProcessStatus {
        events.for_each(|event| {
            if let NodeEventType::Custom(event) = event {
                if let Some(out_stream_event) = event
                    .downcast_mut::<SyncWrapper<NewOutputStreamEvent>>()
                    .and_then(SyncWrapper::get_mut)
                {
                    // Swap the memory so that the old channel will be properly
                    // dropped outside of the audio thread.
                    std::mem::swap(&mut self.prod, &mut out_stream_event.prod);
                }
            }
        });

        if !self.shared_state.stream_active.load(Ordering::Relaxed)
            || self.shared_state.paused.load(Ordering::Relaxed)
        {
            return ProcessStatus::Bypass;
        }

        let Some(prod) = &mut self.prod else {
            return ProcessStatus::Bypass;
        };

        // Notify the input stream that the output stream has begun
        // reading data.
        self.shared_state
            .channel_started
            .store(true, Ordering::Relaxed);

        let pushed_frames = prod.push(inputs, 0..proc_info.frames);

        if pushed_frames < proc_info.frames {
            self.shared_state
                .overflow_occurred
                .store(true, Ordering::Relaxed);
        }

        ProcessStatus::Bypass
    }

    fn stream_stopped(&mut self) {
        self.shared_state
            .stream_active
            .store(false, Ordering::Relaxed);
        self.prod = None;
    }
}

pub struct NewOutputStreamEvent {
    prod: Option<fixed_resample::ResamplingProd<f32>>,
}

impl From<NewOutputStreamEvent> for NodeEventType {
    fn from(value: NewOutputStreamEvent) -> Self {
        NodeEventType::Custom(Box::new(SyncWrapper::new(value)))
    }
}
