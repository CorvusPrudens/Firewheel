use downcast_rs::Downcast;
use std::{any::Any, error::Error, fmt::Debug, hash::Hash};

use crate::{
    clock::{ClockSamples, ClockSeconds},
    dsp::declick::DeclickValues,
    param::ParamEvent,
    ChannelConfig, ChannelCount, SilenceMask, StreamInfo,
};

/// A globally unique identifier for a node.
#[derive(Clone, Copy)]
pub struct NodeID {
    pub idx: thunderdome::Index,
    pub debug_name: &'static str,
}

impl NodeID {
    pub const DANGLING: Self = Self {
        idx: thunderdome::Index::DANGLING,
        debug_name: "dangling",
    };
}

impl Default for NodeID {
    fn default() -> Self {
        Self::DANGLING
    }
}

impl PartialEq for NodeID {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

impl Eq for NodeID {}

impl Ord for NodeID {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.idx.cmp(&other.idx)
    }
}

impl PartialOrd for NodeID {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for NodeID {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
    }
}

impl Debug for NodeID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}-{}",
            self.debug_name,
            self.idx.slot(),
            self.idx.generation()
        )
    }
}

/// The trait describing an audio node in an audio graph.
///
/// # Audio Node Lifecycle:
///
/// 1. The user constructs a new node instance using a custom constructor
/// defined by the node.
/// 2. The host calls [`AudioNode::info`] and [`AudioNode::debug_name`] to
/// get information from the node.
/// 3. The host checks the channel configuration with the info, and then
/// calls [`AudioNode::channel_config_supported`] for a final check on the
/// channel configuration. If the channel configuration is invalid, then
/// the node will be discarded (dropped).
/// 4. The host calls [`AudioNode::activate`]. If successful, then the
/// [`AudioNodeProcessor`] counterpart is sent to the audio thread for
/// processing (there may be a delay before processing starts). If the
/// node returns an error and the node was just added to the graph, then
/// the node will be discarded (dropped).
/// 5. Activated state:
///     * In this state, the user may get a mutable reference to the node
/// via its [`NodeID`] and then downcasting. Note that the user can only
/// access the node mutably this way when it is in the activated state,
/// so there is no need to check for this activated state and return an
/// error in the Node's custom methods.
///     * If the node specified that it wants updates via
/// [`AudioNodeInfo::updates`], then the host will call
/// [`AudioNode::update`] periodically (i.e. once every frame).
/// 6. The host deactivates the node by calling [`AudioNode::deactivate`].
/// If the audio stream did not crash, then the processor counterpart
/// is returned for any additional cleanup.
/// 7. Here, the node may either be activated again or dropped.
pub trait AudioNode: 'static + Downcast {
    /// The name of this type of audio node for debugging purposes.
    fn debug_name(&self) -> &'static str;

    /// Return information about this audio node.
    fn info(&self) -> AudioNodeInfo;

    /// Return `Ok` if the given channel configuration is supported, or
    /// an error if it is not.
    ///
    /// Note that the host already checks if `num_inputs` and `num_outputs`
    /// is within the range given in [`AudioNode::info`], so there is no
    /// need for the node to check that here.
    fn channel_config_supported(
        &self,
        channel_config: ChannelConfig,
    ) -> Result<(), Box<dyn Error>> {
        let _ = channel_config;
        Ok(())
    }

    /// Activate the audio node for processing.
    ///
    /// Note the host will call [`AudioNode::channel_config_supported`] with
    /// the given number of inputs and outputs before calling this method, and
    /// it will only call this method if that method returned `Ok`.
    fn activate(
        &mut self,
        stream_info: &StreamInfo,
        channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn Error>>;

    /// Called when the processor counterpart has been deactivated
    /// and dropped.
    ///
    /// If the audio graph counterpart has gracefully shut down, then
    /// the processor counterpart is returned.
    fn deactivate(&mut self, processor: Option<Box<dyn AudioNodeProcessor>>) {
        let _ = processor;
    }

    /// A method that gets called periodically (i.e. once every frame).
    ///
    /// This method will only be called if [`AudioNodeInfo::updates`]
    /// was set to `true`.
    fn update(&mut self) {}
}

downcast_rs::impl_downcast!(AudioNode);

/// Information about an [`AudioNode`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioNodeInfo {
    /// The minimum number of input buffers this node supports
    pub num_min_supported_inputs: ChannelCount,
    /// The maximum number of input buffers this node supports
    pub num_max_supported_inputs: ChannelCount,

    /// The minimum number of output buffers this node supports
    pub num_min_supported_outputs: ChannelCount,
    /// The maximum number of output buffers this node supports
    pub num_max_supported_outputs: ChannelCount,

    /// Whether or not the number of input channels must match the
    /// number of output channels.
    pub equal_num_ins_and_outs: bool,

    /// The defaul channel configuration for this node
    pub default_channel_config: ChannelConfig,

    /// Whether or not to call the `update` method on this node.
    ///
    /// If you do not need this, set this to `false` to save
    /// some performance overhead.
    ///
    /// By default this is set to `false`.
    pub updates: bool,

    /// Whether or not this node reads any events in
    /// [`AudioNodeProcessor::process`].
    ///
    /// Setting this to `false` will skip allocating an event
    /// buffer for this node.
    ///
    /// By default this is set to `true`.
    pub uses_events: bool,
}

impl Default for AudioNodeInfo {
    fn default() -> Self {
        Self {
            num_min_supported_inputs: ChannelCount::default(),
            num_max_supported_inputs: ChannelCount::default(),
            num_min_supported_outputs: ChannelCount::default(),
            num_max_supported_outputs: ChannelCount::default(),
            default_channel_config: ChannelConfig::default(),
            equal_num_ins_and_outs: false,
            updates: false,
            uses_events: true,
        }
    }
}

/// The trait describing the realtime processor counterpart to an
/// [`AudioNode`].
pub trait AudioNodeProcessor: 'static + Send {
    /// Process the given block of audio. Only process data in the
    /// buffers up to `samples`.
    ///
    /// The node *MUST* either return `ProcessStatus::ClearAllOutputs`
    /// or fill all output buffers with data.
    ///
    /// If any output buffers contain all zeros up to `samples` (silent),
    /// then mark that buffer as silent in [`ProcInfo::out_silence_mask`].
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus;
}

pub const NUM_SCRATCH_BUFFERS: usize = 16;

/// Additional information for processing audio
pub struct ProcInfo<'a, 'b> {
    /// The number of samples (in a single channel of audio) in this
    /// processing block.
    pub samples: usize,

    /// The reciprocal of the sample rate, or the
    /// duration in seconds of each frame.
    pub sample_rate_recip: f64,

    /// An optional optimization hint on which input channels contain
    /// all zeros (silence). The first bit (`0b1`) is the first channel,
    /// the second bit is the second channel, and so on.
    pub in_silence_mask: SilenceMask,

    /// An optional optimization hint on which output channels contain
    /// all zeros (silence). The first bit (`0b1`) is the first channel,
    /// the second bit is the second channel, and so on.
    pub out_silence_mask: SilenceMask,

    /// The current time of the internal clock in units of seconds.
    ///
    /// This uses the clock from the OS's audio API so it should be quite
    /// accurate. This value has also been adjusted to match the clock in
    /// the main thread.
    ///
    /// This value correctly accounts for any output underflows that may
    /// occur.
    pub clock_seconds: ClockSeconds,

    /// The total number of samples that have been processed since the
    /// start of the audio stream.
    ///
    /// This value can be used for more accurate timing than
    /// [`ProcInfo::clock_secs`], but note it does *NOT* account for any
    /// output underflows that may occur.
    pub clock_samples: ClockSamples,

    /// Flags indicating the current status of the audio stream
    pub stream_status: StreamStatus,

    /// A list of extra scratch buffers that can be used for processing.
    /// This removes the need for nodes to allocate their own scratch
    /// buffers.
    ///
    /// Each buffer has a length of [`StreamInfo::max_block_samples`].
    ///
    /// These buffers are shared across all nodes, so assume that they
    /// contain junk data.
    pub scratch_buffers: &'a mut [&'b mut [f32]; NUM_SCRATCH_BUFFERS],

    /// A buffer of values that linearly ramp up/down between `0.0` and `1.0`
    /// which can be used to implement efficient declicking when
    /// pausing/resuming/stopping.
    pub declick_values: &'a DeclickValues,
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

#[derive(Debug)]
pub enum EventData {
    /// Pause this node and all of its queued delayed events.
    Pause,
    /// Resume this node and all of its queued delayed events.
    Resume,
    /// Stop this node and discard all of its queued delayed events.
    Stop,
    /// Enable/disable this node.
    ///
    /// Note the node must implement this event type for this to take
    /// effect.
    SetEnabled(bool),
    /// A parameter event.
    ///
    /// Each node can freely interpret the data according to its parameters.
    Parameter(ParamEvent),
    /// A custom event.
    ///
    /// This is useful for one-shot events like playing samples.
    Custom(Box<dyn Any + Sync + Send>),
}

/// An event sent to an [`AudioNode`].
pub struct NodeEvent {
    /// The ID of the node that should receive the event.
    pub node_id: NodeID,
    /// The type of event.
    pub event: EventData,
}

pub type NodeEventIter<'a> = std::collections::vec_deque::IterMut<'a, EventData>;
