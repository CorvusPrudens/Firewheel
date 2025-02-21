//! A set of diff and patch implementations for common leaf types.

use super::{Diff, EventQueue, Patch, PatchError, PathBuilder};
use crate::{
    collector::ArcGc,
    event::{NodeEventType, ParamData, Vec2, Vec3},
};

macro_rules! primitive_diff {
    ($ty:ty, $variant:ident) => {
        impl Diff for $ty {
            fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
                if self != baseline {
                    event_queue.push_param(*self, path);
                }
            }
        }

        impl Patch for $ty {
            fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
                match data {
                    ParamData::$variant(value) => {
                        *self = *value;
                        Ok(())
                    }
                    _ => Err(PatchError::InvalidData),
                }
            }
        }
    };

    ($ty:ty, $cast:ty, $variant:ident) => {
        impl Diff for $ty {
            fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
                if self != baseline {
                    event_queue.push_param(*self as $cast, path);
                }
            }
        }

        impl Patch for $ty {
            fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
                match data {
                    ParamData::$variant(value) => {
                        *self = *value as $ty;
                        Ok(())
                    }
                    _ => Err(PatchError::InvalidData),
                }
            }
        }
    };
}

primitive_diff!(bool, Bool);

primitive_diff!(u8, u32, U32);
primitive_diff!(u16, u32, U32);
primitive_diff!(u32, U32);
primitive_diff!(u64, U64);

primitive_diff!(i8, i32, I32);
primitive_diff!(i16, i32, I32);
primitive_diff!(i32, I32);
primitive_diff!(i64, u64, U64);

primitive_diff!(f32, F32);
primitive_diff!(f64, F64);

// This may be questionable.
impl<A: ?Sized + Send + Sync + 'static> Diff for ArcGc<A> {
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
        if ArcGc::ptr_eq(self, baseline) {
            event_queue.push(NodeEventType::Param {
                data: ParamData::Any(Box::new(Box::new(self.clone()))),
                path: path.build(),
            });
        }
    }
}

impl<A: ?Sized + Send + Sync + 'static> Patch for ArcGc<A> {
    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        if let ParamData::Any(any) = data {
            if let Some(data) = any.downcast_ref::<Self>() {
                *self = data.clone();
                return Ok(());
            }
        }

        Err(PatchError::InvalidData)
    }
}

impl Diff for Vec2 {
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
        if self != baseline {
            event_queue.push_param(*self, path);
        }
    }
}

impl Patch for Vec2 {
    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        *self = data.try_into()?;
        Ok(())
    }
}

impl Diff for Vec3 {
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
        if self != baseline {
            event_queue.push_param(*self, path);
        }
    }
}

impl Patch for Vec3 {
    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        *self = data.try_into()?;
        Ok(())
    }
}
