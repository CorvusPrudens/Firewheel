use std::{fmt::Debug, hash::Hash, ops::Range};

use crate::{
    channel_config::ChannelConfig,
    clock::{ClockSamples, ClockSeconds, MusicalTime, MusicalTransport},
    dsp::declick::DeclickValues,
    event::NodeEventList,
    SilenceMask, StreamInfo,
};

/// A globally unique identifier for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeID(pub thunderdome::Index);

impl NodeID {
    pub const DANGLING: Self = Self(thunderdome::Index::DANGLING);
}

impl Default for NodeID {
    fn default() -> Self {
        Self::DANGLING
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioNodeInfo {
    /// A unique name for this type of node, used for debugging purposes.
    pub debug_name: &'static str,

    /// The channel configuration of this node.
    pub channel_config: ChannelConfig,

    /// Set to `true` if this node type uses events, `false` otherwise.
    ///
    /// Setting to `false` will help the system save some memory by not
    /// allocating an event buffer for this node.
    pub uses_events: bool,
}

pub trait AudioNodeConstructor {
    type Configuration: Default;

    /// Get information about this node.
    ///
    /// This method is only called once after the node is added to the audio graph.
    fn info(&self, configuration: &Self::Configuration) -> AudioNodeInfo;

    /// Construct a processor for this node.
    fn processor(
        &self,
        configuration: &Self::Configuration,
        stream_info: &StreamInfo,
    ) -> impl AudioNodeProcessor;
}

/// A type-erased [`AudioNodeConstructor`].
pub trait AudioNode {
    fn info(&self) -> AudioNodeInfo;
    fn processor(&self, stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor>;
}

/// Pairs constructors with their configurations.
///
/// This is useful for type-erasing an [`AudioNodeConstructor`].
pub struct Constructor<T, C> {
    constructor: T,
    configuration: C,
}

impl<T: AudioNodeConstructor> Constructor<T, T::Configuration> {
    pub fn new(constructor: T, configuration: Option<T::Configuration>) -> Self {
        Self {
            constructor,
            configuration: configuration.unwrap_or_default(),
        }
    }
}

impl<T: AudioNodeConstructor> AudioNode for Constructor<T, T::Configuration> {
    fn info(&self) -> AudioNodeInfo {
        self.constructor.info(&self.configuration)
    }

    fn processor(&self, stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(self.constructor.processor(&self.configuration, stream_info))
    }
}

/// The trait describing the realtime processor counterpart to an
/// audio node.
pub trait AudioNodeProcessor: 'static + Send {
    /// Process the given block of audio. Only process data in the
    /// buffers up to `samples`.
    ///
    /// The node *MUST* either return `ProcessStatus::ClearAllOutputs`
    /// or fill all output buffers with data.
    ///
    /// If any output buffers contain all zeros up to `samples` (silent),
    /// then mark that buffer as silent in [`ProcInfo::out_silence_mask`].
    ///
    /// * `inputs` - The input buffers.
    /// * `outputs` - The output buffers,
    /// * `events` - A list of events for this node to process.
    /// * `proc_info` - Additional information about the process.
    /// * `scratch_buffers` - A list of extra scratch buffers that can be
    /// used for processing. This removes the need for nodes to allocate
    /// their own scratch buffers. Each buffer has a length of
    /// [`StreamInfo::max_block_frames`]. These buffers are shared across
    /// all nodes, so assume that they contain junk data.
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventList,
        proc_info: &ProcInfo,
        scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus;

    /// Called when the audio stream has been stopped.
    fn stream_stopped(&mut self) {}

    /// Called when a new audio stream has been started after a previous
    /// call to [`AudioNodeProcessor::stream_stopped`].
    ///
    /// Note, this method gets called on the main thread, not the audio
    /// thread. So it is safe to allocate/deallocate here.
    fn new_stream(&mut self, stream_info: &StreamInfo) {
        let _ = stream_info;
    }
}

pub const NUM_SCRATCH_BUFFERS: usize = 8;

/// Additional information for processing audio
pub struct ProcInfo<'a> {
    /// The number of samples (in a single channel of audio) in this
    /// processing block.
    ///
    /// Not to be confused with video frames.
    pub frames: usize,

    /// An optional optimization hint on which input channels contain
    /// all zeros (silence). The first bit (`0b1`) is the first channel,
    /// the second bit is the second channel, and so on.
    pub in_silence_mask: SilenceMask,

    /// An optional optimization hint on which output channels contain
    /// all zeros (silence). The first bit (`0b1`) is the first channel,
    /// the second bit is the second channel, and so on.
    pub out_silence_mask: SilenceMask,

    /// The current interval of time of the internal clock in units of
    /// seconds. The start of the range is the instant of time at the
    /// first sample in the block (inclusive), and the end of the range
    /// is the instant of time at the end of the block (exclusive).
    ///
    /// This uses the clock from the OS's audio API so it should be quite
    /// accurate, and it correctly accounts for any output underflows that
    /// may occur.
    pub clock_seconds: Range<ClockSeconds>,

    /// The total number of samples (in a single channel of audio) that
    /// have been processed since the start of the audio stream.
    ///
    /// This value can be used for more accurate timing than
    /// [`ProcInfo::clock_seconds`], but note it does *NOT* account for any
    /// output underflows that may occur.
    pub clock_samples: ClockSamples,

    /// Information about the musical transport.
    ///
    /// This will be `None` if no musical transport is currently active,
    /// or if the current transport is currently paused.
    pub transport_info: Option<TransportInfo<'a>>,

    /// Flags indicating the current status of the audio stream
    pub stream_status: StreamStatus,

    /// A buffer of values that linearly ramp up/down between `0.0` and `1.0`
    /// which can be used to implement efficient declicking when
    /// pausing/resuming/stopping.
    pub declick_values: &'a DeclickValues,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransportInfo<'a> {
    /// The current transport.
    pub transport: &'a MusicalTransport,

    /// The current interval of time of the internal clock in units of
    /// musical time. The start of the range is the instant of time at the
    /// first sample in the block (inclusive), and the end of the range
    /// is the instant of time at the end of the block (exclusive).
    ///
    /// This will be `None` if no musical clock is currently present.
    pub musical_clock: Range<MusicalTime>,

    /// Whether or not the transport is currently paused.
    pub paused: bool,
}

bitflags::bitflags! {
    /// Flags indicating the current status of the audio stream
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct StreamStatus: u32 {
        /// Some input data was discarded because of an overflow condition
        /// at the audio driver.
        const INPUT_OVERFLOW = 0b01;

        /// The output buffer ran low, likely producing a break in the
        /// output sound.
        const OUTPUT_UNDERFLOW = 0b10;
    }
}

/// The status of processing buffers in an audio node.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// No output buffers were modified. If this is returned, then
    /// the engine will automatically clear all output buffers
    /// for you as efficiently as possible.
    #[default]
    ClearAllOutputs,
    /// No output buffers were modified. If this is returned, then
    /// the engine will automatically copy the input buffers to
    /// their corresponding output buffers for you as efficiently
    /// as possible.
    Bypass,
    /// All output buffers were filled with data.
    OutputsModified { out_silence_mask: SilenceMask },
}

impl ProcessStatus {
    /// All output buffers were filled with non-silence.
    pub const fn outputs_not_silent() -> Self {
        Self::OutputsModified {
            out_silence_mask: SilenceMask::NONE_SILENT,
        }
    }

    /// All output buffers were filled with data.
    pub const fn outputs_modified(out_silence_mask: SilenceMask) -> Self {
        Self::OutputsModified { out_silence_mask }
    }
}

/// The configuration for a "dummy" node, a node which does nothing.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DummyConfig {
    pub channel_config: ChannelConfig,
}

impl AudioNode for DummyConfig {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "dummy",
            channel_config: self.channel_config,
            uses_events: false,
        }
    }

    fn processor(&self, _stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(DummyProcessor)
    }
}

pub struct DummyProcessor;

impl AudioNodeProcessor for DummyProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        _proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        ProcessStatus::Bypass
    }
}
