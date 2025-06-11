use firewheel_core::diff::{Diff, Patch};

#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct HighShelfFilterNode {
    pub cutoff_hz: f32,
    pub q: f32,
    pub gain_db: f32,
}

impl Default for HighShelfFilterNode {
    fn default() -> Self {
        Self {
            cutoff_hz: 1.,
            q: 1.,
            gain_db: 0.,
        }
    }
}
