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
pub fn process_sample_a(filter_state: f32, target_times_a: f32, coeff: Coeff) -> f32 {
    target_times_a + (filter_state * coeff.b)
}

pub fn process_into_buffer(
    buffer: &mut [f32],
    filter_state: f32,
    target: f32,
    coeff: Coeff,
) -> f32 {
    let target_times_a = target * coeff.a;

    buffer[0] = process_sample_a(filter_state, target_times_a, coeff);

    if buffer.len() > 1 {
        for i in 1..buffer.len() {
            buffer[i] = process_sample_a(buffer[i - 1], target_times_a, coeff);
        }
    }

    *buffer.last().unwrap()
}

pub fn has_settled(filter_state: f32, target: f32, settle_epsilon: f32) -> bool {
    (filter_state - target).abs() < settle_epsilon
}

pub fn process_into_buffers_simd2(
    buffers: [&mut [f32]; 2],
    filter_states: [f32; 2],
    targets: [f32; 2],
    coeff: Coeff,
) -> [f32; 2] {
    let frames = buffers[0].len().min(buffers[1].len());

    let target_times_a = [targets[0] * coeff.a, targets[1] * coeff.a];

    let mut frame = [
        process_sample_a(filter_states[0], target_times_a[0], coeff),
        process_sample_a(filter_states[1], target_times_a[1], coeff),
    ];

    buffers[0][0] = frame[0];
    buffers[1][0] = frame[1];

    if frames > 1 {
        for i in 1..frames {
            frame = [
                process_sample_a(frame[0], target_times_a[0], coeff),
                process_sample_a(frame[1], target_times_a[1], coeff),
            ];

            buffers[0][i] = frame[0];
            buffers[1][i] = frame[1];
        }
    }

    frame
}

pub fn process_into_buffers_simd3(
    buffers: [&mut [f32]; 3],
    filter_states: [f32; 3],
    targets: [f32; 3],
    coeff: Coeff,
) -> [f32; 3] {
    let frames = buffers[0].len().min(buffers[1].len()).min(buffers[2].len());

    let target_times_a = [
        targets[0] * coeff.a,
        targets[1] * coeff.a,
        targets[2] * coeff.a,
    ];

    let mut frame = [
        process_sample_a(filter_states[0], target_times_a[0], coeff),
        process_sample_a(filter_states[1], target_times_a[1], coeff),
        process_sample_a(filter_states[2], target_times_a[2], coeff),
    ];

    buffers[0][0] = frame[0];
    buffers[1][0] = frame[1];
    buffers[2][0] = frame[2];

    if frames > 1 {
        for i in 1..frames {
            frame = [
                process_sample_a(frame[0], target_times_a[0], coeff),
                process_sample_a(frame[1], target_times_a[1], coeff),
                process_sample_a(frame[2], target_times_a[2], coeff),
            ];

            buffers[0][i] = frame[0];
            buffers[1][i] = frame[1];
            buffers[2][i] = frame[2];
        }
    }

    frame
}

pub fn process_into_buffers_simd4(
    buffers: [&mut [f32]; 4],
    filter_states: [f32; 4],
    targets: [f32; 4],
    coeff: Coeff,
) -> [f32; 4] {
    let frames = buffers[0]
        .len()
        .min(buffers[1].len())
        .min(buffers[2].len())
        .min(buffers[3].len());

    let target_times_a = [
        targets[0] * coeff.a,
        targets[1] * coeff.a,
        targets[2] * coeff.a,
        targets[3] * coeff.a,
    ];

    let mut frame = [
        process_sample_a(filter_states[0], target_times_a[0], coeff),
        process_sample_a(filter_states[1], target_times_a[1], coeff),
        process_sample_a(filter_states[2], target_times_a[2], coeff),
        process_sample_a(filter_states[3], target_times_a[3], coeff),
    ];

    buffers[0][0] = frame[0];
    buffers[1][0] = frame[1];
    buffers[2][0] = frame[2];
    buffers[3][0] = frame[3];

    if frames > 1 {
        for i in 1..frames {
            frame = [
                process_sample_a(frame[0], target_times_a[0], coeff),
                process_sample_a(frame[1], target_times_a[1], coeff),
                process_sample_a(frame[2], target_times_a[2], coeff),
                process_sample_a(frame[3], target_times_a[3], coeff),
            ];

            buffers[0][i] = frame[0];
            buffers[1][i] = frame[1];
            buffers[2][i] = frame[2];
            buffers[3][i] = frame[3];
        }
    }

    frame
}
