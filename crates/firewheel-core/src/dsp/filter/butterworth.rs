use super::{rejection_filter::Lowpass, spec::Steepness};

pub fn new<S: Steepness>(freq: f32, sample_rate_recip: f32) -> Lowpass<S::ORDER> {}
