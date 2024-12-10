use std::ops::Range;

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
                    samples_left: declick_values.fade_samples(),
                }
            }
            Self::FadingTo1 { samples_left } => {
                let samples_left = if *samples_left <= declick_values.fade_samples() {
                    declick_values.fade_samples() - *samples_left
                } else {
                    declick_values.fade_samples()
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
                    samples_left: declick_values.fade_samples(),
                }
            }
            Self::FadingTo0 { samples_left } => {
                let samples_left = if *samples_left <= declick_values.fade_samples() {
                    declick_values.fade_samples() - *samples_left
                } else {
                    declick_values.fade_samples()
                };

                *self = Self::FadingTo0 { samples_left }
            }
            _ => {}
        }
    }

    pub fn reset_to_0(&mut self) {
        *self = Self::SettledAt0;
    }

    pub fn reset_to_1(&mut self) {
        *self = Self::SettledAt0;
    }

    pub fn process<V: AsMut<[f32]>>(
        &mut self,
        buffers: &mut [V],
        buffer_range: Range<usize>,
        declick_values: &DeclickValues,
    ) {
        let mut fade_buffers = |declick_samples_left: &mut usize, values: &[f32]| -> usize {
            let buffer_samples = buffer_range.end - buffer_range.start;
            let process_samples = buffer_samples.min(*declick_samples_left);
            let start_frame = values.len() - *declick_samples_left;

            for b in buffers.iter_mut() {
                let b = &mut b.as_mut()[buffer_range.clone()];

                for (s, &g) in b[..process_samples]
                    .iter_mut()
                    .zip(values[start_frame..start_frame + process_samples].iter())
                {
                    *s *= g;
                }
            }

            *declick_samples_left -= process_samples;

            process_samples
        };

        match self {
            Self::SettledAt0 => {
                for b in buffers.iter_mut() {
                    let b = &mut b.as_mut();
                    b[buffer_range.clone()].fill(0.0);
                }
            }
            Self::FadingTo0 { samples_left } => {
                let samples_processed =
                    fade_buffers(samples_left, &declick_values.fade_1_to_0_values);

                if samples_processed < buffer_range.end - buffer_range.start {
                    for b in buffers.iter_mut() {
                        let b = &mut b.as_mut()[buffer_range.clone()];

                        b[samples_processed..].fill(0.0);
                    }
                }

                if *samples_left == 0 {
                    *self = Self::SettledAt0;
                }
            }
            Self::FadingTo1 { samples_left } => {
                fade_buffers(samples_left, &declick_values.fade_0_to_1_values);

                if *samples_left == 0 {
                    *self = Self::SettledAt1;
                }
            }
            _ => {}
        }
    }
}

/// A buffer of values that linearly ramp up/down between `0.0` and `1.0`.
///
/// This approach is more SIMD-friendly than using a smoothing filter or
/// incrementing the gain per-sample.
pub struct DeclickValues {
    pub fade_0_to_1_values: Vec<f32>,
    pub fade_1_to_0_values: Vec<f32>,
}

impl DeclickValues {
    pub const DEFAULT_FADE_SECONDS: f32 = 3.0 / 1_000.0;

    pub fn new(fade_seconds: f32, sample_rate: u32) -> Self {
        let fade_samples = (fade_seconds * sample_rate as f32).round() as usize;
        let fade_samples_recip = (fade_samples as f32).recip();

        let mut fade_0_to_1_values = Vec::new();
        let mut fade_1_to_0_values = Vec::new();

        fade_0_to_1_values.reserve_exact(fade_samples);
        fade_1_to_0_values.reserve_exact(fade_samples);

        fade_0_to_1_values = (0..fade_samples)
            .map(|i| i as f32 * fade_samples_recip)
            .collect();
        fade_1_to_0_values = (0..fade_samples)
            .rev()
            .map(|i| i as f32 * fade_samples_recip)
            .collect();

        Self {
            fade_0_to_1_values,
            fade_1_to_0_values,
        }
    }

    pub fn fade_samples(&self) -> usize {
        self.fade_0_to_1_values.len()
    }
}
