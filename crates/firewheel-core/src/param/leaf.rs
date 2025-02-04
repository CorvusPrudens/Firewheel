//! A set of implementations for common leaf types.

use super::{Diff, EventQueue, PatchError, PathBuilder};
use crate::{
    collector::ArcGc,
    event::{NodeEventType, ParamData},
};
use smallvec::SmallVec;

macro_rules! primitive_diff {
    ($ty:ty, $variant:ident) => {
        impl Diff for $ty {
            fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
                if self != baseline {
                    event_queue.push_param(*self, path);
                }
            }

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

    fn patch(&mut self, data: &ParamData, _: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::Any(any) => {
                if let Some(data) = any.downcast_ref::<Self>() {
                    *self = data.clone();
                    return Ok(());
                }
            }
            _ => {}
        }

        Err(PatchError::InvalidData)
    }
}

macro_rules! sequence_diff {
    ($gen:ident, $ty:ty) => {
        impl<$gen: Diff> Diff for $ty {
            fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
                for (i, item) in self.iter().enumerate() {
                    item.diff(&baseline[i], path.with(i as u32), event_queue);
                }
            }

            fn patch(&mut self, data: &ParamData, path: &[u32]) -> Result<(), PatchError> {
                let first = path.first().ok_or(PatchError::InvalidPath)?;
                let target = self
                    .get_mut(*first as usize)
                    .ok_or(PatchError::InvalidPath)?;

                target.patch(data, &path[1..])
            }
        }
    };
}

sequence_diff!(T, Vec<T>);
sequence_diff!(T, Box<[T]>);
sequence_diff!(T, [T]);

impl<T: Diff, const LEN: usize> Diff for [T; LEN] {
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
        for (i, item) in self.iter().enumerate() {
            item.diff(&baseline[i], path.with(i as u32), event_queue);
        }
    }

    fn patch(&mut self, data: &ParamData, path: &[u32]) -> Result<(), PatchError> {
        let first = path.first().ok_or(PatchError::InvalidPath)?;
        let target = self
            .get_mut(*first as usize)
            .ok_or(PatchError::InvalidPath)?;

        target.patch(data, &path[1..])
    }
}

#[cfg(feature = "bevy")]
impl Diff for bevy_math::prelude::Vec2 {
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
        if self != baseline {
            event_queue.push_param(*self, path);
        }
    }

    fn patch(&mut self, data: &ParamData, path: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::Vector2D([x, y]) => {
                self.x = *x;
                self.y = *y;

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }
}

#[cfg(feature = "bevy")]
impl Diff for bevy_math::prelude::Vec3 {
    fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
        if self != baseline {
            event_queue.push_param(*self, path);
        }
    }

    fn patch(&mut self, data: &ParamData, path: &[u32]) -> Result<(), PatchError> {
        match data {
            ParamData::Vector3D([x, y, z]) => {
                self.x = *x;
                self.y = *y;
                self.z = *z;

                Ok(())
            }
            _ => Err(PatchError::InvalidData),
        }
    }
}
