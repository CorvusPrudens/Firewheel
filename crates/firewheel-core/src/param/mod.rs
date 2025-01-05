use crate::clock::ClockSeconds;
use bevy_math::prelude::{Curve, Ease, EaseFunction, EasingCurve};
use core::any::Any;
use fixed_vec::FixedVec;
use smallvec::SmallVec;

mod fixed_vec;
pub mod range;
pub mod smoother;

/// Derive [`AudioParam`] for structs composed of
/// types that also implement [`AudioParam`].
///
/// [`AudioParam`] cannot be derived on enums.
pub use firewheel_macros::AudioParam;

/// A set of audio parameters.
///
/// This trait allows a type to perform diffing on itself,
/// generating events that another instance can use to patch
/// itself.
///
/// Fields are distinguished by their [`ParamPath`]. Since
/// every non-cyclic struct can be represented as a tree,
/// a path of indeces can be used to distinguish any
/// arbitrarily nested field. This is similar to techniques used
/// in [reactive_stores](https://docs.rs/reactive_stores/latest/reactive_stores/)
/// and [Xilem](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html).
pub trait AudioParam: Sized {
    /// Compare `self` to `cmp` and generate events to resolve any differences.
    fn diff(&self, cmp: &Self, writer: impl FnMut(ParamEvent), path: ParamPath);

    /// Patch `self` according to the incoming data.
    /// This will generally be called from within
    /// the audio thread.
    ///
    /// `data` is intentionally made a shared reference.
    /// This should make accidental syscalls due to
    /// additional allocations or drops more difficult.
    /// If you find yourself reaching for interior
    /// mutability, consider whether you're building
    /// realtime-appropriate behavior.
    fn patch(&mut self, data: &ParamData, path: &[u32]) -> Result<(), PatchError>;

    /// Update `self` according to `time`, if necessary.
    fn tick(&mut self, time: ClockSeconds) {
        // Suppress the unused warning
        let _ = time;
    }
}

/// An parameter synchronization event.
#[derive(Debug)]
pub struct ParamEvent {
    pub data: ParamData,
    pub path: ParamPath,
}

/// The payload for a [`ParamEvent`].
#[derive(Debug)]
pub enum ParamData {
    F32(TimelineEvent<f32>),
    I32(TimelineEvent<i32>),
    I64(TimelineEvent<i64>),
    Bool(DeferredEvent<bool>),
    Any(Box<dyn Any + Sync + Send>),
}

/// A path of indeces that uniquely describes an arbitrarily nested field.
#[derive(Clone, Debug, Default)]
pub struct ParamPath(SmallVec<[u32; 4]>);

impl core::ops::Deref for ParamPath {
    type Target = [u32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ParamPath {
    pub fn with(&self, index: u32) -> Self {
        let mut new = self.0.clone();
        new.push(index);
        Self(new)
    }
}

/// An error encountered when patching a type
/// from [`ParamData`].
#[derive(Debug, Clone)]
pub enum PatchError {
    /// The provided path does not match any children.
    InvalidPath,
    /// The data supplied for the path did not match the expected type.
    InvalidData,
}

/// A parameter expressed as a timeline of events.
///
/// This allows parameters to vary smoothly at audio-rate
/// with minimal cross-thread communication.
#[derive(Debug, Clone)]
pub struct Timeline<T> {
    value: T,
    events: FixedVec<TimelineEvent<T>>,
    /// The total number of events consumed.
    consumed: usize,
}

impl<T> Timeline<T> {
    /// Create a new instance of [`Timeline`] with an initial value.
    pub fn new(value: T) -> Self {
        Self {
            value,
            events: Default::default(),
            consumed: 0,
        }
    }

    /// Returns whether the value is changing at `time`.
    pub fn is_active(&self, time: ClockSeconds) -> bool {
        self.events
            .iter()
            .any(|e| e.contains(time) && matches!(e, TimelineEvent::Curve { .. }))
    }

    /// Returns whether this node will change within the time range.
    pub fn active_within(&self, start: ClockSeconds, end: ClockSeconds) -> bool {
        self.events.iter().any(|e| {
            e.start_time()
                .is_some_and(|t| (start.0..end.0).contains(&t.0))
                || e.end_time()
                    .is_some_and(|t| (start.0..end.0).contains(&t.0))
        })
    }

    /// Remove all events from the timeline.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[derive(Debug, Clone)]
pub enum TimelineError {
    OverlappingRanges,
}

impl<T: Ease + Clone> Timeline<T> {
    /// Push an event to the timeline, popping off the oldest one if the
    /// queue is full.
    pub fn push(&mut self, event: TimelineEvent<T>) -> Result<(), TimelineError> {
        // scan the events to ensure the event doesn't overlap any ranges
        match &event {
            TimelineEvent::Deferred { time, .. } => {
                if self.events.iter().any(|e| e.overlaps(*time)) {
                    return Err(TimelineError::OverlappingRanges);
                }
            }
            TimelineEvent::Curve { start, end, .. } => {
                if self
                    .events
                    .iter()
                    .any(|e| e.overlaps(*start) || e.overlaps(*end))
                {
                    return Err(TimelineError::OverlappingRanges);
                }
            }
            TimelineEvent::Immediate(i) => {
                self.clear();
                self.value = i.clone();
            }
        }

        self.events.push(event);
        self.consumed += 1;

        Ok(())
    }

    /// Set the value immediately.
    pub fn set(&mut self, value: T) {
        // This push cannot fail.
        self.push(TimelineEvent::Immediate(value)).unwrap();
    }

    /// Push a curve event with absolute timestamps.
    pub fn push_curve(
        &mut self,
        end_value: T,
        start: ClockSeconds,
        end: ClockSeconds,
        curve: EaseFunction,
    ) -> Result<(), TimelineError> {
        let start_value = self.value_at(start);
        let curve = EasingCurve::new(start_value, end_value, curve);

        self.push(TimelineEvent::Curve { curve, start, end })
    }

    /// Get the value at a point in time.
    pub fn value_at(&self, time: ClockSeconds) -> T {
        if let Some(bounded) = self.events.iter().find(|e| e.contains(time)) {
            return bounded.get(time);
        }

        let mut recent_time = f64::MAX;
        let mut recent_value = None;

        for event in self.events.iter() {
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

    /// Get the current value without respect to time.
    ///
    /// This depends on regular calls to [`AudioParam::tick`]
    /// for accuracy.
    pub fn get(&self) -> T {
        self.value.clone()
    }
}

#[derive(Debug, Clone)]
pub enum TimelineEvent<T> {
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

impl<T> TimelineEvent<T> {
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

impl<T: Ease + Clone> TimelineEvent<T> {
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
pub enum DeferredEvent<T> {
    Immediate(T),
    Deferred { value: T, time: ClockSeconds },
}

impl<T> From<DeferredEvent<T>> for TimelineEvent<T> {
    fn from(value: DeferredEvent<T>) -> Self {
        match value {
            DeferredEvent::Immediate(i) => TimelineEvent::Immediate(i),
            DeferredEvent::Deferred { value, time } => TimelineEvent::Deferred { value, time },
        }
    }
}

impl<T> DeferredEvent<T>
where
    T: Clone,
{
    pub fn end_time(&self) -> Option<ClockSeconds> {
        match self {
            Self::Immediate(_) => None,
            Self::Deferred { time, .. } => Some(*time),
        }
    }

    pub fn value(&self) -> T {
        match self {
            Self::Immediate(i) => i.clone(),
            Self::Deferred { value, .. } => value.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Deferred<T> {
    value: T,
    events: FixedVec<DeferredEvent<T>>,
    consumed: usize,
}

impl<T> Deferred<T>
where
    T: Clone,
{
    /// Construct a new [`Deferred`] with an initial value.
    pub fn new(value: T) -> Self {
        Self {
            value,
            events: Default::default(),
            consumed: 0,
        }
    }

    /// Get the value at a point in time.
    pub fn value_at(&self, time: ClockSeconds) -> T {
        let mut recent_time = f64::MAX;
        let mut recent_value = None;

        for event in self.events.iter() {
            if let Some(end) = event.end_time() {
                let delta = time.0 - end.0;

                if delta >= 0. && delta < recent_time {
                    recent_time = delta;
                    recent_value = Some(event.value());
                }
            }
        }

        recent_value.unwrap_or(self.value.clone())
    }

    /// Get the current value without respect to time.
    ///
    /// This depends on regular calls to [`AudioParam::tick`]
    /// for accuracy.
    pub fn get(&self) -> T {
        self.value.clone()
    }

    pub fn push(&mut self, event: DeferredEvent<T>) {
        if matches!(event, DeferredEvent::Immediate(_)) {
            self.events.clear();
        }

        self.events.push(event);
        self.consumed += 1;
    }

    pub fn active_within(&self, start: ClockSeconds, end: ClockSeconds) -> bool {
        self.events.iter().any(|e| {
            e.end_time()
                .is_some_and(|t| (start.0..end.0).contains(&t.0))
        })
    }
}

impl AudioParam for () {
    fn diff(&self, _cmp: &Self, _writer: impl FnMut(ParamEvent), _path: ParamPath) {}

    fn patch(&mut self, _data: &ParamData, _path: &[u32]) -> Result<(), PatchError> {
        Ok(())
    }
}

impl AudioParam for f32 {
    fn diff(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
        if self != cmp {
            writer(ParamEvent {
                data: ParamData::F32(TimelineEvent::Immediate(*self)),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::F32(TimelineEvent::Immediate(value)) => {
                *self = *value;

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }
}

impl AudioParam for bool {
    fn diff(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
        if self != cmp {
            writer(ParamEvent {
                data: ParamData::Bool(DeferredEvent::Immediate(*self)),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::Bool(DeferredEvent::Immediate(value)) => {
                *self = *value;

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }
}

impl AudioParam for Timeline<f32> {
    fn diff(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
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

    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::F32(message) => {
                // There's not much error handling that can be
                // done in the audio thread.
                let _ = self.push(message.clone());

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }

    fn tick(&mut self, time: ClockSeconds) {
        self.value = self.value_at(time);
    }
}

impl AudioParam for Deferred<bool> {
    fn diff(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
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
                data: ParamData::Bool(event.clone()),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::Bool(message) => {
                self.push(message.clone());

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }

    fn tick(&mut self, time: ClockSeconds) {
        self.value = self.value_at(time);
    }
}

impl AudioParam for Deferred<i32> {
    fn diff(&self, cmp: &Self, mut writer: impl FnMut(ParamEvent), path: ParamPath) {
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
                data: ParamData::I32(event.clone().into()),
                path: path.clone(),
            });
        }
    }

    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::I32(message) => match message {
                TimelineEvent::Deferred { value, time } => {
                    self.push(DeferredEvent::Deferred {
                        value: *value,
                        time: *time,
                    });

                    Ok(())
                }
                TimelineEvent::Immediate(i) => {
                    self.push(DeferredEvent::Immediate(*i));

                    Ok(())
                }
                _ => Err(PatchError::InvalidData),
            },
            _ => Err(PatchError::InvalidData),
        }
    }

    fn tick(&mut self, time: ClockSeconds) {
        self.value = self.value_at(time);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_continuous_diff() {
        let a = Timeline::new(0f32);
        let mut b = a.clone();

        b.push_curve(
            2f32,
            ClockSeconds(1.),
            ClockSeconds(2.),
            EaseFunction::Linear,
        )
        .unwrap();

        let mut events = Vec::new();
        b.diff(&a, |event| events.push(event), Default::default());

        assert!(
            matches!(&events.as_slice(), &[ParamEvent { data, .. }] if matches!(data, ParamData::F32(_)))
        )
    }

    #[test]
    fn test_full_diff() {
        let mut a = Timeline::new(0f32);

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

        let mut events = Vec::new();
        b.diff(&a, |event| events.push(event), Default::default());

        assert!(
            matches!(&events.as_slice(), &[ParamEvent { data, .. }] if matches!(data, ParamData::F32(d) if d.end_value() == 1.))
        )
    }

    #[test]
    fn test_linear_curve() {
        let mut value = Timeline::new(0f32);

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
            .push(TimelineEvent::Deferred {
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
