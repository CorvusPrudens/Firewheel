use firewheel_core::diff::{Diff, Patch};

#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct HighpassFilterNode {
    pub order: u32,
    pub cutoff_hz: f32,
    pub q: f32,
}

impl Default for HighpassFilterNode {
    fn default() -> Self {
        Self {
            order: 2,
            cutoff_hz: 1.,
            q: 1.,
        }
    }
}
