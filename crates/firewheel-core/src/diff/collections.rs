//! A set of diff and patch implementations for common collections.

use super::{Diff, EventQueue, Patch, PatchError, PathBuilder};
use crate::event::ParamData;

macro_rules! sequence_diff {
    ($gen:ident, $ty:ty) => {
        impl<$gen: Diff> Diff for $ty {
            fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
                for (i, item) in self.iter().enumerate() {
                    item.diff(&baseline[i], path.with(i as u32), event_queue);
                }
            }
        }

        impl<$gen: Patch> Patch for $ty {
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
}

impl<T: Patch, const LEN: usize> Patch for [T; LEN] {
    fn patch(&mut self, data: &ParamData, path: &[u32]) -> Result<(), PatchError> {
        let first = path.first().ok_or(PatchError::InvalidPath)?;
        let target = self
            .get_mut(*first as usize)
            .ok_or(PatchError::InvalidPath)?;

        target.patch(data, &path[1..])
    }
}

macro_rules! tuple_diff {
    ($($gen:ident, $base:ident, $index:literal),*) => {
        #[allow(non_snake_case, unused_variables)]
        impl<$($gen: Diff),*> Diff for ($($gen,)*) {
            fn diff<E: EventQueue>(&self, baseline: &Self, path: PathBuilder, event_queue: &mut E) {
                let ($($gen,)*) = self;
                let ($($base,)*) = baseline;

                $(
                    $gen.diff($base, path.with($index), event_queue);
                )*
            }
        }
    };
}

tuple_diff!();
tuple_diff!(A0, A1, 0);
tuple_diff!(A0, A1, 0, B0, B1, 1);
tuple_diff!(A0, A1, 0, B0, B1, 1, C0, C1, 2);
tuple_diff!(A0, A1, 0, B0, B1, 1, C0, C1, 2, D0, D1, 3);
tuple_diff!(A0, A1, 0, B0, B1, 1, C0, C1, 2, D0, D1, 3, E0, E1, 4);
tuple_diff!(A0, A1, 0, B0, B1, 1, C0, C1, 2, D0, D1, 3, E0, E1, 4, F0, F1, 5);
tuple_diff!(A0, A1, 0, B0, B1, 1, C0, C1, 2, D0, D1, 3, E0, E1, 4, F0, F1, 5, G0, G1, 6);
tuple_diff!(A0, A1, 0, B0, B1, 1, C0, C1, 2, D0, D1, 3, E0, E1, 4, F0, F1, 5, G0, G1, 6, H0, H1, 7);
