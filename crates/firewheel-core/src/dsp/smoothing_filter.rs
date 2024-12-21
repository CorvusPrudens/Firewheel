use std::num::NonZeroU32;

pub const DEFAULT_SMOOTH_SECONDS: f32 = 5.0 / 1_000.0;
pub const DEFAULT_SETTLE_EPSILON: f32 = 0.00001f32;

/// The coefficients for a simple smoothing/declicking filter where:
///
/// `out[n] = (target_value * a) + (out[n-1] * b)`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coeff {
    pub a: f32,
    pub b: f32,
}

impl Coeff {
    pub fn new(sample_rate: NonZeroU32, smooth_secs: f32) -> Self {
        assert!(smooth_secs > 0.0);

        let b = (-1.0f32 / (smooth_secs * sample_rate.get() as f32)).exp();
        let a = 1.0f32 - b;

        Self { a, b }
    }
}

#[inline(always)]
pub fn process_sample(filter_state: f32, target: f32, coeff: Coeff) -> f32 {
    (target * coeff.a) + (filter_state * coeff.b)
}

#[inline(always)]
pub fn process_sample_a(filter_state: f32, target_times_a: f32, coeff_b: f32) -> f32 {
    target_times_a + (filter_state * coeff_b)
}

pub fn process_into_buffer(
    buffer: &mut [f32],
    filter_state: f32,
    target: f32,
    coeff: Coeff,
) -> f32 {
    let target_times_a = target * coeff.a;

    buffer[0] = process_sample_a(filter_state, target_times_a, coeff.b);

    if buffer.len() > 1 {
        for i in 1..buffer.len() {
            buffer[i] = process_sample_a(buffer[i - 1], target_times_a, coeff.b);
        }
    }

    *buffer.last().unwrap()
}

pub fn has_settled(filter_state: f32, target: f32, settle_epsilon: f32) -> bool {
    (filter_state - target).abs() < settle_epsilon
}
