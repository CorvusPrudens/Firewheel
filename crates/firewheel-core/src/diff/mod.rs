use crate::{
    collector::ArcGc,
    event::{NodeEventList, NodeEventType, ParamData},
};

use smallvec::SmallVec;

mod collections;
mod leaf;
mod memo;

pub use memo::Memo;

pub use firewheel_macros::{Diff, Patch};

/// A trait for diffing and patching.
///
/// This trait allows a type to perform diffing on itself,
/// generating events that another instance can use to patch
/// itself.
///
/// Fields are distinguished by their [`ParamPath`]. Since
/// every non-sharing struct can be represented as a tree,
/// a path of indeces can be used to distinguish any
/// arbitrarily nested field.
pub trait Diff {
    /// Compare `self` to `baseline` and generate events to resolve any differences.
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E);
}

pub trait Patch {
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

    /// Patch a set of parameters with incoming events.
    ///
    /// Returns `true` if any parameters have changed.
    ///
    /// This is usefule as a convenience method for extracting the path
    /// and data components from a [`NodeEventType`]. Errors produced
    /// here are ignored.
    fn patch_event(&mut self, event: &NodeEventType) -> bool {
        if let NodeEventType::Param { data, path } = event {
            // NOTE: It may not be ideal to ignore errors.
            // Would it be possible to log these in debug mode?
            self.patch(data, path).is_ok()
        } else {
            false
        }
    }

    /// Patch a set of parameters with a list of incoming events.
    ///
    /// Returns `true` if any parameters have changed.
    ///
    /// This is usefule as a convenience method for patching parameters
    /// directly from a [`NodeEventList`]. Errors produced here are ignored.
    fn patch_list(&mut self, mut event_list: NodeEventList) -> bool {
        let mut changed = false;

        event_list.for_each(|e| {
            changed |= self.patch_event(e);
        });

        changed
    }
}

/// A path of indeces that uniquely describes an arbitrarily nested field.
pub enum ParamPath {
    Single(u32),
    Multi(ArcGc<SmallVec<[u32; 4]>>),
}

impl core::ops::Deref for ParamPath {
    type Target = [u32];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Single(single) => core::slice::from_ref(single),
            Self::Multi(multi) => multi.as_ref(),
        }
    }
}

// NOTE: Using a `SmallVec` instead of a `Box<[u32]>` yields
// around an 8% performance uplift for cases where the path
// is in the range 2..=4.
//
// Beyond this range, the performance drops off around 13%.
//
// Since this avoids extra allocations in the common < 5
// scenario, this seems like a reasonable tradeoff.

/// A simple builder for [`ParamPath`].
#[derive(Debug, Default, Clone)]
pub struct PathBuilder(SmallVec<[u32; 4]>);

impl PathBuilder {
    /// Clone the path and append the index.
    pub fn with(&self, index: u32) -> Self {
        let mut new = self.0.clone();
        new.push(index);
        Self(new)
    }

    /// Convert this path builder into a [`ParamPath`].
    pub fn build(self) -> ParamPath {
        if self.0.len() == 1 {
            ParamPath::Single(self.0[0])
        } else {
            ParamPath::Multi(ArcGc::new(self.0.clone()))
        }
    }
}

/// An event queue for diffing.
pub trait EventQueue {
    /// Push an event to the queue.
    fn push(&mut self, data: NodeEventType);

    /// Push an event to the queue.
    ///
    /// This is a convenience method for constructing a [`NodeEventType`]
    /// from param data and a path.
    #[inline(always)]
    fn push_param(&mut self, data: impl Into<ParamData>, path: PathBuilder) {
        self.push(NodeEventType::Param {
            data: data.into(),
            path: path.build(),
        });
    }
}

impl EventQueue for Vec<NodeEventType> {
    fn push(&mut self, data: NodeEventType) {
        self.push(data);
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
