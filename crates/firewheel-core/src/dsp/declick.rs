use std::f32::consts::FRAC_PI_2;
use std::{num::NonZeroU32, ops::Range};

/// A struct when can be used to linearly ramp up/down between `0.0`
/// and `1.0` to declick audio streams.
///
/// This approach is more SIMD-friendly than using a smoothing filter
/// or incrementing a gain value per-sample.
///
/// Used in conjunction with [`DeclickValues`].
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declicker {
    SettledAt0,
    #[default]
    SettledAt1,
    FadingTo0 {
        samples_left: usize,
    },
    FadingTo1 {
        samples_left: usize,
    },
}

impl Declicker {
    pub fn is_settled(&self) -> bool {
        *self == Self::SettledAt0 || *self == Self::SettledAt1
    }

    pub fn fade_to_0(&mut self, declick_values: &DeclickValues) {
        match self {
            Self::SettledAt1 => {
                *self = Self::FadingTo0 {
                    samples_left: declick_values.frames(),
                }
            }
            Self::FadingTo1 { samples_left } => {
                let samples_left = if *samples_left <= declick_values.frames() {
                    declick_values.frames() - *samples_left
                } else {
                    declick_values.frames()
                };

                *self = Self::FadingTo0 { samples_left }
            }
            _ => {}
        }
    }

    pub fn fade_to_1(&mut self, declick_values: &DeclickValues) {
        match self {
            Self::SettledAt0 => {
                *self = Self::FadingTo1 {
                    samples_left: declick_values.frames(),
                }
            }
            Self::FadingTo0 { samples_left } => {
                let samples_left = if *samples_left <= declick_values.frames() {
                    declick_values.frames() - *samples_left
                } else {
                    declick_values.frames()
                };

                *self = Self::FadingTo1 { samples_left }
            }
            _ => {}
        }
    }

    pub fn reset_to_0(&mut self) {
        *self = Self::SettledAt0;
    }

    pub fn reset_to_1(&mut self) {
        *self = Self::SettledAt1;
    }

    pub fn process<V: AsMut<[f32]>>(
        &mut self,
        buffers: &mut [V],
        range_in_buffer: Range<usize>,
        declick_values: &DeclickValues,
        gain: f32,
        fade_type: FadeType,
    ) {
        let mut fade_buffers = |declick_samples_left: &mut usize, values: &[f32]| -> usize {
            let buffer_samples = range_in_buffer.end - range_in_buffer.start;
            let process_samples = buffer_samples.min(*declick_samples_left);
            let start_frame = values.len() - *declick_samples_left;

            if gain == 1.0 {
                for b in buffers.iter_mut() {
                    let b = &mut b.as_mut()
                        [range_in_buffer.start..range_in_buffer.start + process_samples];

                    for (s, &g) in b
                        .iter_mut()
                        .zip(values[start_frame..start_frame + process_samples].iter())
                    {
                        *s *= g;
                    }
                }
            } else {
                for b in buffers.iter_mut() {
                    let b = &mut b.as_mut()
                        [range_in_buffer.start..range_in_buffer.start + process_samples];

                    for (s, &g) in b
                        .iter_mut()
                        .zip(values[start_frame..start_frame + process_samples].iter())
                    {
                        *s *= g * gain;
                    }
                }
            }

            *declick_samples_left -= process_samples;

            process_samples
        };

        match self {
            Self::SettledAt0 => {
                for b in buffers.iter_mut() {
                    let b = &mut b.as_mut();
                    b[range_in_buffer.clone()].fill(0.0);
                }
            }
            Self::FadingTo0 { samples_left } => {
                let values = match fade_type {
                    FadeType::Linear => &declick_values.linear_1_to_0_values,
                    FadeType::EqualPower3dB => &declick_values.circular_1_to_0_values,
                };

                let samples_processed = fade_buffers(samples_left, values);

                if samples_processed < range_in_buffer.end - range_in_buffer.start {
                    for b in buffers.iter_mut() {
                        let b = &mut b.as_mut()
                            [range_in_buffer.start + samples_processed..range_in_buffer.end];
                        b.fill(0.0);
                    }
                }

                if *samples_left == 0 {
                    *self = Self::SettledAt0;
                }
            }
            Self::FadingTo1 { samples_left } => {
                let values = match fade_type {
                    FadeType::Linear => &declick_values.linear_0_to_1_values,
                    FadeType::EqualPower3dB => &declick_values.circular_0_to_1_values,
                };

                let samples_processed = fade_buffers(samples_left, values);

                if samples_processed < range_in_buffer.end - range_in_buffer.start && gain != 1.0 {
                    for b in buffers.iter_mut() {
                        let b = &mut b.as_mut()
                            [range_in_buffer.start + samples_processed..range_in_buffer.end];
                        for s in b.iter_mut() {
                            *s *= gain;
                        }
                    }
                }

                if *samples_left == 0 {
                    *self = Self::SettledAt1;
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeType {
    /// Linear fade.
    Linear,
    /// Equal power fade (circular).
    EqualPower3dB,
}

/// A buffer of values that linearly ramp up/down between `0.0` and `1.0`.
///
/// This approach is more SIMD-friendly than using a smoothing filter or
/// incrementing the gain per-sample.
pub struct DeclickValues {
    pub linear_0_to_1_values: Vec<f32>,
    pub linear_1_to_0_values: Vec<f32>,
    pub circular_0_to_1_values: Vec<f32>,
    pub circular_1_to_0_values: Vec<f32>,
}

impl DeclickValues {
    pub const DEFAULT_FADE_SECONDS: f32 = 10.0 / 1_000.0;

    pub fn new(frames: NonZeroU32) -> Self {
        let frames = frames.get() as usize;
        let frames_recip = (frames as f32).recip();

        let mut linear_0_to_1_values = Vec::new();
        let mut linear_1_to_0_values = Vec::new();
        let mut circular_0_to_1_values = Vec::new();
        let mut circular_1_to_0_values = Vec::new();

        linear_0_to_1_values.reserve_exact(frames);
        linear_1_to_0_values.reserve_exact(frames);
        circular_0_to_1_values.reserve_exact(frames);
        circular_1_to_0_values.reserve_exact(frames);

        linear_0_to_1_values = (0..frames).map(|i| i as f32 * frames_recip).collect();
        linear_1_to_0_values = (0..frames).rev().map(|i| i as f32 * frames_recip).collect();

        circular_0_to_1_values = linear_0_to_1_values
            .iter()
            .map(|x| (x * FRAC_PI_2).sin())
            .collect();
        circular_1_to_0_values = circular_0_to_1_values.iter().rev().copied().collect();

        Self {
            linear_0_to_1_values,
            linear_1_to_0_values,
            circular_0_to_1_values,
            circular_1_to_0_values,
        }
    }

    pub fn frames(&self) -> usize {
        self.linear_0_to_1_values.len()
    }
}
