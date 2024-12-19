use arrayvec::ArrayVec;
use bevy_math::{
    curve::{Ease, EaseFunction},
    prelude::EasingCurve,
    Curve,
};
use downcast_rs::Downcast;
use smallvec::SmallVec;
use std::{any::Any, error::Error, fmt::Debug, hash::Hash};

use crate::{
    clock::{ClockSamples, ClockSeconds},
    dsp::declick::DeclickValues,
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

#[derive(Debug, Clone)]
pub enum ContinuousEvent<T> {
    Immediate(T),
    Deferred {
        value: T,
        time: ClockSeconds,
    },
    Curve {
        curve: EasingCurve<T>,
        start: ClockSeconds,
        end: ClockSeconds,
    },
}

impl<T> ContinuousEvent<T> {
    pub fn start_time(&self) -> Option<ClockSeconds> {
        match self {
            Self::Deferred { time, .. } => Some(*time),
            Self::Curve { start, .. } => Some(*start),
            _ => None,
        }
    }

    pub fn end_time(&self) -> Option<ClockSeconds> {
        match self {
            Self::Deferred { time, .. } => Some(*time),
            Self::Curve { end, .. } => Some(*end),
            _ => None,
        }
    }

    pub fn contains(&self, time: ClockSeconds) -> bool {
        match self {
            Self::Deferred { time: t, .. } => *t == time,
            Self::Curve { start, end, .. } => (*start..=*end).contains(&time),
            _ => false,
        }
    }

    pub fn overlaps(&self, time: ClockSeconds) -> bool {
        match self {
            Self::Curve { start, end, .. } => time > *start && time < *end,
            _ => false,
        }
    }
}

impl<T: Ease + Clone> ContinuousEvent<T> {
    pub fn get(&self, time: ClockSeconds) -> T {
        match self {
            Self::Immediate(i) => i.clone(),
            Self::Deferred { value, .. } => value.clone(),
            Self::Curve { curve, start, end } => {
                let range = end.0 - start.0;
                let progress = time.0 - start.0;

                curve.sample((progress / range) as f32).unwrap()
            }
        }
    }

    pub fn start_value(&self) -> T {
        match self {
            Self::Immediate(i) => i.clone(),
            Self::Deferred { value, .. } => value.clone(),
            Self::Curve { curve, .. } => curve.sample(0.).unwrap(),
        }
    }

    pub fn end_value(&self) -> T {
        match self {
            Self::Immediate(i) => i.clone(),
            Self::Deferred { value, .. } => value.clone(),
            Self::Curve { curve, .. } => curve.sample(1.).unwrap(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Continuous<T> {
    value: T,
    events: ArrayVec<ContinuousEvent<T>, 4>,
    /// The total number of events consumed.
    consumed: usize,
}

impl<T> Continuous<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            events: Default::default(),
            consumed: 0,
        }
    }

    pub fn push(&mut self, event: ContinuousEvent<T>) -> Result<(), ContinuousError> {
        // scan the events to ensure the event doesn't overlap any ranges
        match event {
            ContinuousEvent::Deferred { time, .. } => {
                if self.events.iter().any(|e| e.overlaps(time)) {
                    return Err(ContinuousError::OverlappingRanges);
                }
            }
            ContinuousEvent::Curve { start, end, .. } => {
                if self
                    .events
                    .iter()
                    .any(|e| e.overlaps(start) || e.overlaps(end))
                {
                    return Err(ContinuousError::OverlappingRanges);
                }
            }
            ContinuousEvent::Immediate(_) => {}
        }

        if self.events.remaining_capacity() == 0 {
            self.events.pop_at(0);
        }

        self.events.push(event);
        self.consumed += 1;

        Ok(())
    }

    pub fn is_active(&self, time: ClockSeconds) -> bool {
        self.events
            .iter()
            .any(|e| e.contains(time) && matches!(e, ContinuousEvent::Curve { .. }))
    }
}

#[derive(Debug, Clone)]
pub enum ContinuousError {
    OverlappingRanges,
}

impl<T: Ease + Clone> Continuous<T> {
    pub fn push_curve(
        &mut self,
        end_value: T,
        start: ClockSeconds,
        end: ClockSeconds,
        curve: EaseFunction,
    ) -> Result<(), ContinuousError> {
        let start_value = self.value_at(start);
        let curve = EasingCurve::new(start_value, end_value, curve);

        self.push(ContinuousEvent::Curve { curve, start, end })
    }

    /// Get the value at a point in time.
    pub fn value_at(&self, time: ClockSeconds) -> T {
        if let Some(bounded) = self.events.iter().find(|e| e.contains(time)) {
            return bounded.get(time);
        }

        let mut recent_time = core::f64::MAX;
        let mut recent_value = None;

        for event in &self.events {
            if let Some(end) = event.end_time() {
                let delta = time.0 - end.0;

                if delta >= 0. && delta < recent_time {
                    recent_time = delta;
                    recent_value = Some(event.end_value());
                }
            }
        }

        recent_value.unwrap_or(self.value.clone())
    }

    pub fn get(&self) -> T {
        self.value.clone()
    }
}

#[derive(Debug, Clone)]
pub enum DeferredEvent<T> {
    Immediate(T),
    Deferred { value: T, time: ClockSeconds },
}

#[derive(Debug, Clone)]
pub struct Deferred<T> {
    value: T,
    events: ArrayVec<DeferredEvent<T>, 4>,
    consumed: usize,
}

pub enum ParamData {
    F32(ContinuousEvent<f32>),
    F64(ContinuousEvent<f64>),
    I32(ContinuousEvent<i32>),
    I64(ContinuousEvent<i64>),
    Bool(DeferredEvent<bool>),
    Any(Box<dyn Any + Sync + Send>),
}

pub struct ParamEvent {
    pub data: ParamData,
    pub path: ParamPath,
}

#[derive(Default)]
pub struct ParamEvents(Vec<ParamEvent>);

impl ParamEvents {
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, message: ParamEvent) {
        self.0.push(message);
    }
}

#[derive(Clone, Default)]
pub struct ParamPath(SmallVec<[u16; 8]>);

impl core::ops::Deref for ParamPath {
    type Target = [u16];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ParamPath {
    pub fn with(&self, index: u16) -> Self {
        let mut new = self.0.clone();
        new.push(index);
        Self(new)
    }
}

pub enum PatchError {
    InvalidPath,
    InvalidData,
}

pub use firewheel_macros::AudioParam;

pub trait AudioParam: Sized {
    fn to_messages(&self, cmp: &Self, writer: impl FnMut(ParamEvent), path: ParamPath);

    fn patch(&mut self, data: &mut ParamData, path: &[u16]) -> Result<(), PatchError>;

    fn tick(&mut self, time: ClockSeconds);
}

impl AudioParam for () {
    fn to_messages(&self, _cmp: &Self, _writer: impl FnMut(ParamEvent), _path: ParamPath) {}

    fn patch(&mut self, _data: &mut ParamData, _path: &[u16]) -> Result<(), PatchError> {
        Ok(())
    }

    fn tick(&mut self, _time: ClockSeconds) {}
}

impl AudioParam for f32 {
    fn to_messages(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
        if self != cmp {
            writer(ParamEvent {
                data: ParamData::F32(ContinuousEvent::Immediate(*self)),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &mut ParamData, _: &[u16]) -> Result<(), PatchError> {
        match data {
            ParamData::F32(ContinuousEvent::Immediate(value)) => {
                *self = *value;

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }

    fn tick(&mut self, _time: ClockSeconds) {}
}

impl AudioParam for bool {
    fn to_messages(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
        if self != cmp {
            writer(ParamEvent {
                data: ParamData::Bool(DeferredEvent::Immediate(*self)),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &mut ParamData, _: &[u16]) -> Result<(), PatchError> {
        match data {
            ParamData::Bool(DeferredEvent::Immediate(value)) => {
                *self = *value;

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }

    fn tick(&mut self, _time: ClockSeconds) {}
}

impl AudioParam for Continuous<f32> {
    fn to_messages(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
        let newly_consumed = self.consumed.saturating_sub(cmp.consumed);

        if newly_consumed == 0 {
            return;
        }

        // If more items were added than the buffer can hold, we only have the most recent self.events.len() items.
        let clamped_newly_consumed = newly_consumed.min(self.events.len());

        // Start index for the new items. They are the last 'clamped_newly_consumed' items in the buffer.
        let start = self.events.len() - clamped_newly_consumed;
        let new_items = &self.events[start..];

        for event in new_items.iter() {
            writer(ParamEvent {
                data: ParamData::F32(event.clone()),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &mut ParamData, _: &[u16]) -> Result<(), PatchError> {
        match data {
            ParamData::F32(message) => {
                self.events.push(message.clone());

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }

    fn tick(&mut self, time: ClockSeconds) {
        self.value = self.value_at(time);
    }
}

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

///// An event type associated with an [`AudioNode`].
//pub enum NodeEventType {
//    /// Pause this node and all of its queued delayed events.
//    ///
//    /// Note this event type cannot be delayed.
//    Pause,
//    /// Resume this node and all of its queued delayed events.
//    ///
//    /// Note this event type cannot be delayed.
//    Resume,
//    /// Stop this node and discard all of its queued delayed events.
//    ///
//    /// Note this event type cannot be delayed.
//    Stop,
//    /// Enable/disable this node.
//    ///
//    /// Note the node must implement this event type for this to take
//    /// effect.
//    SetEnabled(bool),
//    /// Set the value of an `f32` parameter.
//    F32Param {
//        /// The unique ID of the paramater.
//        id: u32,
//        /// The parameter value.
//        value: f32,
//        /// Set this to `false` to request the node to immediately jump
//        /// to this new value without smoothing (may cause audible
//        /// clicking or stair-stepping artifacts).
//        smoothing: bool,
//    },
//    /// Set the value of an `f64` parameter.
//    F64Param {
//        /// The unique ID of the paramater.
//        id: u32,
//        /// The parameter value.
//        value: f64,
//        /// Set this to `false` to request the node to immediately jump
//        /// to this new value without smoothing (may cause audible
//        /// clicking or stair-stepping artifacts).
//        smoothing: bool,
//    },
//    /// Set the value of an `i32` parameter.
//    I32Param {
//        /// The unique ID of the paramater.
//        id: u32,
//        /// The parameter value.
//        value: i32,
//        /// Set this to `false` to request the node to immediately jump
//        /// to this new value without smoothing (may cause audible
//        /// clicking or stair-stepping artifacts).
//        smoothing: bool,
//    },
//    /// Set the value of an `u64` parameter.
//    U64Param {
//        /// The unique ID of the paramater.
//        id: u32,
//        /// The parameter value.
//        value: u64,
//        /// Set this to `false` to request the node to immediately jump
//        /// to this new value without smoothing (may cause audible
//        /// clicking or stair-stepping artifacts).
//        smoothing: bool,
//    },
//    /// Set the value of a `bool` parameter.
//    BoolParam {
//        /// The unique ID of the paramater.
//        id: u32,
//        /// The parameter value.
//        value: bool,
//        /// Set this to `false` to request the node to immediately jump
//        /// to this new value without smoothing (may cause audible
//        /// clicking or stair-stepping artifacts).
//        smoothing: bool,
//    },
//    /// Set the value of a parameter containing three
//    /// `f32` elements.
//    Vector3DParam {
//        /// The unique ID of the paramater.
//        id: u32,
//        /// The parameter value.
//        value: [f32; 3],
//        /// Set this to `false` to request the node to immediately jump
//        /// to this new value without smoothing (may cause audible
//        /// clicking or stair-stepping artifacts).
//        smoothing: bool,
//    },
//    /// Play a sample to completion.
//    ///
//    /// (Even though this event is only used by the `OneShotSamplerNode`,
//    /// because it is so common, define it here so the event doesn't have
//    /// to be allocated every time.)
//    PlaySample {
//        /// The sample resource to play.
//        sample: Arc<dyn SampleResource>,
//        /// The normalized volume to play this sample at (where `0.0` is mute
//        /// and `1.0` is unity gain.)
//        ///
//        /// Note, this value cannot be changed while the sample is playing.
//        /// Use a `VolumeNode` for that instead.
//        normalized_volume: f32,
//        /// If `true`, then all other voices currently being played in this
//        /// node will be stopped.
//        stop_other_voices: bool,
//    },
//    /// Custom event type.
//    Custom(Box<dyn Any + Send>),
//    // TODO: Animation (automation) event types.
//}

pub type NodeEventIter<'a> = std::collections::vec_deque::IterMut<'a, EventData>;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_continuous_diff() {
        let a = Continuous::new(0f32);
        let mut b = a.clone();

        b.push_curve(
            2f32,
            ClockSeconds(1.),
            ClockSeconds(2.),
            EaseFunction::Linear,
        )
        .unwrap();

        let mut events = ParamEvents::new();
        b.to_messages(&a, &mut events, Default::default());

        assert!(
            matches!(&events.0.as_slice(), &[ParamEvent { data, .. }] if matches!(data, ParamData::F32(_)))
        )
    }

    #[test]
    fn test_full_diff() {
        let mut a = Continuous::new(0f32);

        for _ in 0..8 {
            a.push_curve(
                2f32,
                ClockSeconds(1.),
                ClockSeconds(2.),
                EaseFunction::Linear,
            )
            .unwrap();
        }

        let mut b = a.clone();

        b.push_curve(
            1f32,
            ClockSeconds(1.),
            ClockSeconds(2.),
            EaseFunction::Linear,
        )
        .unwrap();

        let mut events = ParamEvents::new();
        b.to_messages(&a, &mut events, Default::default());

        assert!(
            matches!(&events.0.as_slice(), &[ParamEvent { data, .. }] if matches!(data, ParamData::F32(d) if d.end_value() == 1.))
        )
    }

    #[test]
    fn test_linear_curve() {
        let mut value = Continuous::new(0f32);

        value
            .push_curve(
                1f32,
                ClockSeconds(0.),
                ClockSeconds(1.),
                EaseFunction::Linear,
            )
            .unwrap();

        value
            .push_curve(
                2f32,
                ClockSeconds(1.),
                ClockSeconds(2.),
                EaseFunction::Linear,
            )
            .unwrap();

        value
            .push(ContinuousEvent::Deferred {
                value: 3.0,
                time: ClockSeconds(2.5),
            })
            .unwrap();

        assert_eq!(value.value_at(ClockSeconds(0.)), 0.);
        assert_eq!(value.value_at(ClockSeconds(0.5)), 0.5);
        assert_eq!(value.value_at(ClockSeconds(1.0)), 1.0);

        assert_eq!(value.value_at(ClockSeconds(1.)), 1.);
        assert_eq!(value.value_at(ClockSeconds(1.5)), 1.5);
        assert_eq!(value.value_at(ClockSeconds(2.0)), 2.0);

        assert_eq!(value.value_at(ClockSeconds(2.25)), 2.0);

        assert_eq!(value.value_at(ClockSeconds(2.5)), 3.0);
    }
}
