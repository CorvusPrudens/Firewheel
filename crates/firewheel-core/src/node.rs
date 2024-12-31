use std::{any::Any, fmt::Debug, hash::Hash, sync::Arc};

use crate::{
    clock::{ClockSamples, ClockSeconds, EventDelay},
    dsp::declick::DeclickValues,
    sample_resource::SampleResource,
    SilenceMask,
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

pub const NUM_SCRATCH_BUFFERS: usize = 8;

/// Additional information for processing audio
pub struct ProcInfo<'a, 'b> {
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

    /// The current time of the internal clock in units of seconds.
    ///
    /// This uses the clock from the OS's audio API so it should be quite
    /// accurate.
    ///
    /// This value correctly accounts for any output underflows that may
    /// occur.
    pub clock_seconds: ClockSeconds,

    /// The total number of samples (in a single channel of audio) that
    /// have been processed since the start of the audio stream.
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

/// An event sent to an [`AudioNode`].
pub struct NodeEvent {
    /// The ID of the node that should receive the event.
    pub node_id: NodeID,
    /// The delay of this event.
    ///
    /// Note the following event types will ignore this value and execute
    /// the event immediately instead:
    /// * [`NodeEventType::Pause`]
    /// * [`NodeEventType::Resume`]
    /// * [`NodeEventType::Stop`]
    pub delay: EventDelay,
    /// The type of event.
    pub event: NodeEventType,
}

/// An event type associated with an [`AudioNode`].
pub enum NodeEventType {
    /// Pause this node and all of its pending delayed events.
    ///
    /// Note this event type cannot be delayed.
    Pause,
    /// Resume this node and all of its pending delayed events.
    ///
    /// Note this event type cannot be delayed.
    Resume,
    /// Stop this node and discard all of its pending delayed events.
    ///
    /// Note this event type cannot be delayed.
    Stop,
    /// Tell the node to start/restart its playback sequencue.
    ///
    /// Note the node must implement this event type for this to take
    /// effect.
    StartOrRestart,
    /// Enable/disable this node.
    ///
    /// Note the node must implement this event type for this to take
    /// effect.
    SetEnabled(bool),
    /// Tell the node to discard any stored data (like samples).
    ///
    /// Note the node must implement this event type for this to take
    /// effect.
    DiscardData,
    /// Set the value of an `f32` parameter.
    F32Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: f32,
        /// Set this to `false` to request the node to immediately jump
        /// to this new value without smoothing (may cause audible
        /// clicking or stair-stepping artifacts).
        smoothing: bool,
    },
    /// Set the value of an `f64` parameter.
    F64Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: f64,
        /// Set this to `false` to request the node to immediately jump
        /// to this new value without smoothing (may cause audible
        /// clicking or stair-stepping artifacts).
        smoothing: bool,
    },
    /// Set the value of an `i32` parameter.
    I32Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: i32,
        /// Set this to `false` to request the node to immediately jump
        /// to this new value without smoothing (may cause audible
        /// clicking or stair-stepping artifacts).
        smoothing: bool,
    },
    /// Set the value of an `u64` parameter.
    U64Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: u64,
        /// Set this to `false` to request the node to immediately jump
        /// to this new value without smoothing (may cause audible
        /// clicking or stair-stepping artifacts).
        smoothing: bool,
    },
    /// Set the value of a `bool` parameter.
    BoolParam {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: bool,
        /// Set this to `false` to request the node to immediately jump
        /// to this new value without smoothing (may cause audible
        /// clicking or stair-stepping artifacts).
        smoothing: bool,
    },
    /// Set the value of a parameter containing three
    /// `f32` elements.
    Vector3DParam {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: [f32; 3],
        /// Set this to `false` to request the node to immediately jump
        /// to this new value without smoothing (may cause audible
        /// clicking or stair-stepping artifacts).
        smoothing: bool,
    },
    /// Use the given sample. This will stop any currently playing samples
    /// and reset the playhead to the beginning. Send a `StartOrRestart`
    /// event to begin playback.
    ///
    /// (Even though this event is only used by the `SamplerNode`, because it
    /// is so common, define it here so that the event doesn't have to be
    /// allocated every time.)
    NewSample {
        /// The sample resource.
        sample: Arc<dyn SampleResource>,
        /// The normalized volume to play this sample at (where `0.0` is mute
        /// and `1.0` is unity gain.)
        ///
        /// Note, this value cannot be changed while the sample is playing.
        /// Use a `VolumeNode` or a `StereoVolumePanNOde` node for that
        /// instead.
        normalized_volume: f32,
        /// How many times this sample should be repeated for each
        /// `StartOrRestart` command.
        repeat_mode: RepeatMode,
    },
    /// Custom event type.
    Custom(Box<dyn Any + Send>),
    /// Reserved for use by [`NodeHandle`] only.
    _Dropped,
    // TODO: Animation (automation) event types.
}

pub type NodeEventIter<'a> = std::collections::vec_deque::IterMut<'a, NodeEventType>;

/// How many times a sample/sequence should be repeated for each `StartOrRestart` command.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    /// Play the sample/sequence once and then stop.
    #[default]
    PlayOnce,
    /// Repeat the sample/sequence the given number of times.
    RepeatMultiple { num_times_to_repeat: u16 },
    /// Repeat the sample/sequence endlessly.
    RepeatEndlessly,
}

impl RepeatMode {
    pub fn do_loop(&self, num_times_looped_back: u64) -> bool {
        match self {
            Self::PlayOnce => false,
            &Self::RepeatMultiple {
                num_times_to_repeat,
            } => num_times_looped_back < num_times_to_repeat as u64,
            Self::RepeatEndlessly => true,
        }
    }
}

// TODO: Option to use crossbeam.
pub type EventQueueSender = std::sync::mpsc::Sender<NodeEvent>;

/// A handle to a node in the audio graph.
///
/// When this handle is dropped, it will automatically send an event through
/// the event queue to notify the audio graph to remove this node.
#[derive(Debug, Clone)]
pub struct NodeHandle {
    pub id: NodeID,
    pub event_queue_sender: EventQueueSender,
}

impl NodeHandle {
    /// Queue an event to send to this node's processor counterpart.
    ///
    /// Note that [`FirewheelCtx::update()`] must be called to
    /// flush the events.
    pub fn queue_event(&mut self, event: NodeEventType, delay: EventDelay) {
        self.event_queue_sender
            .send(NodeEvent {
                node_id: self.id,
                delay,
                event,
            })
            .unwrap();
    }
}

impl Drop for NodeHandle {
    fn drop(&mut self) {
        let _ = self.event_queue_sender.send(NodeEvent {
            node_id: self.id,
            delay: EventDelay::Immediate,
            event: NodeEventType::_Dropped,
        });
    }
}
