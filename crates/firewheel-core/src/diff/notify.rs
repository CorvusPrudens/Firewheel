use crate::{
    diff::{Diff, Patch},
    event::ParamData,
};
use core::sync::atomic::{AtomicU64, Ordering};

static NOTIFY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct Notify<T> {
    value: T,
    counter: u64,
}

impl<T> Notify<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            counter: NOTIFY_COUNTER.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl<T> core::ops::Deref for Notify<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> core::ops::DerefMut for Notify<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.counter = NOTIFY_COUNTER.fetch_add(1, Ordering::Relaxed);

        &mut self.value
    }
}

impl<T: Clone + Send + Sync + 'static> Diff for Notify<T> {
    fn diff<E: super::EventQueue>(
        &self,
        baseline: &Self,
        path: super::PathBuilder,
        event_queue: &mut E,
    ) {
        if self.counter != baseline.counter {
            event_queue.push_param(ParamData::any(self.clone()), path);
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Patch for Notify<T> {
    type Patch = Self;

    fn patch(data: &ParamData, _: &[u32]) -> Result<Self::Patch, super::PatchError> {
        data.downcast_ref()
            .ok_or(super::PatchError::InvalidData)
            .cloned()
    }

    fn apply(&mut self, patch: Self::Patch) {
        *self = patch;
    }
}
