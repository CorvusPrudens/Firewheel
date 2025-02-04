use crate::{
    collector::ArcGc,
    event::{NodeEventType, ParamData},
};
use smallvec::SmallVec;

mod leaf;
pub mod range;
pub mod smoother;

pub use firewheel_macros::Diff;

/// A trait for diffing and patching.
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
pub trait Diff {
    /// Compare `self` to `baseline` and generate events to resolve any differences.
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E);

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
}

/// A convenience trait for types that implement `Diff`.
pub trait PatchParams: Diff {
    /// Patch a set of parameters with incoming events.
    fn patch_params(&mut self, event: &NodeEventType) {
        if let NodeEventType::Param { data, path } = event {
            // NOTE: It may not be ideal to ignore errors.
            // Would it be possible to log these in debug mode?
            let _ = self.patch(data, &path);
        }
    }
}

impl<T: Diff> PatchParams for T {}

/// A path of indeces that uniquely describes an arbitrarily nested field.
pub enum ParamPath {
    Single(u32),
    Multi(ArcGc<Box<[u32]>>),
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
            ParamPath::Multi(ArcGc::new(self.0.as_ref().into()))
        }
    }
}

pub trait EventQueue {
    fn push(&mut self, data: NodeEventType);

    #[inline(always)]
    fn push_param(&mut self, data: impl Into<ParamData>, path: PathBuilder) {
        self.push(NodeEventType::Param {
            data: data.into(),
            path: path.build(),
        });
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
