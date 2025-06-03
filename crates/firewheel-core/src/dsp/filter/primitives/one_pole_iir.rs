//! Based on https://github.com/MeadowlarkDAW/meadow-dsp/tree/main/meadow-dsp-mit with permission
use std::f32::consts::PI;

use crate::dsp::filter::filter_trait::Filter;

/// The coefficients for a single-pole IIR filter.
#[derive(Default, Clone, Copy, PartialEq)]
pub struct OnePoleIirCoeff {
    pub a0: f32,
    pub b1: f32,

    pub m0: f32,
    pub m1: f32,
}

impl OnePoleIirCoeff {
    pub const NO_OP: Self = Self {
        a0: 0.0,
        b1: 0.0,
        m0: 1.0,
        m1: 0.0,
    };

    pub fn lowpass(cutoff_hz: f32, sample_rate_recip: f32) -> Self {
        let b1 = ((-2.0 * PI) * cutoff_hz * sample_rate_recip).exp();
        let a0 = 1.0 - b1;

        Self {
            a0,
            b1,
            m0: 0.0,
            m1: 1.0,
        }
    }

    pub fn highpass(cutoff_hz: f32, sample_rate_recip: f32) -> Self {
        let b1 = ((-2.0 * PI) * cutoff_hz * sample_rate_recip).exp();
        let a0 = 1.0 - b1;

        Self {
            a0,
            b1,
            m0: 1.0,
            m1: -1.0,
        }
    }
}

/// The state of a single-pole IIR filter.
#[derive(Default, Clone, Copy, PartialEq)]
pub struct OnePoleIirState {
    pub z1: f32,
}

impl Filter for OnePoleIirState {
    type Coeffs = OnePoleIirCoeff;

    #[inline(always)]
    fn process(&mut self, input: f32, coeff: &Self::Coeffs) -> f32 {
        self.z1 = (coeff.a0 * input) + (coeff.b1 * self.z1);
        coeff.m0 * input + coeff.m1 * self.z1
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.z1 = 0.0;
    }

    #[inline(always)]
    fn is_silent(&self, eps: f32) -> bool {
        self.z1.abs() <= eps
    }
}

#[cfg(feature = "portable-simd")]
pub mod simd {
    use std::{
        array,
        simd::{cmp::SimdPartialOrd, f32x4, f32x8, num::SimdFloat},
    };

    use crate::dsp::filter::filter_trait::Filter;

    use super::{OnePoleIirCoeff, OnePoleIirState};

    /// The coefficients of four one-pole IIR filters packed into an SIMD vector.
    #[derive(Default, Clone, Copy)]
    pub struct OnePoleIirCoeffx4 {
        pub a0: f32x4,
        pub b1: f32x4,

        pub m0: f32x4,
        pub m1: f32x4,
    }

    impl OnePoleIirCoeffx4 {
        pub const fn splat(coeffs: OnePoleIirCoeff) -> Self {
            Self {
                a0: f32x4::splat(coeffs.a0),
                b1: f32x4::splat(coeffs.b1),
                m0: f32x4::splat(coeffs.m0),
                m1: f32x4::splat(coeffs.m1),
            }
        }

        pub fn load(coeffs: &[OnePoleIirCoeff; 4]) -> Self {
            Self {
                a0: f32x4::from_array(array::from_fn(|i| coeffs[i].a0)),
                b1: f32x4::from_array(array::from_fn(|i| coeffs[i].b1)),
                m0: f32x4::from_array(array::from_fn(|i| coeffs[i].m0)),
                m1: f32x4::from_array(array::from_fn(|i| coeffs[i].m1)),
            }
        }
    }

    /// The coefficients of eight one-pole IIR filters packed into an SIMD vector.
    #[derive(Default, Clone, Copy)]
    pub struct OnePoleIirCoeffx8 {
        pub a0: f32x8,
        pub b1: f32x8,

        pub m0: f32x8,
        pub m1: f32x8,
    }

    impl OnePoleIirCoeffx8 {
        pub const fn splat(coeffs: OnePoleIirCoeff) -> Self {
            Self {
                a0: f32x8::splat(coeffs.a0),
                b1: f32x8::splat(coeffs.b1),
                m0: f32x8::splat(coeffs.m0),
                m1: f32x8::splat(coeffs.m1),
            }
        }

        pub fn load(coeffs: &[OnePoleIirCoeff; 8]) -> Self {
            Self {
                a0: f32x8::from_array(array::from_fn(|i| coeffs[i].a0)),
                b1: f32x8::from_array(array::from_fn(|i| coeffs[i].b1)),
                m0: f32x8::from_array(array::from_fn(|i| coeffs[i].m0)),
                m1: f32x8::from_array(array::from_fn(|i| coeffs[i].m1)),
            }
        }
    }

    /// The state of four single-pole IIR filters packed into an SIMD vector.
    #[derive(Default, Clone, Copy)]
    pub struct OnePoleIirStatex4 {
        z1: f32x4,
    }

    impl OnePoleIirStatex4 {
        pub const fn splat(state: OnePoleIirState) -> Self {
            Self {
                z1: f32x4::splat(state.z1),
            }
        }

        pub fn load(states: &[OnePoleIirState; 4]) -> Self {
            Self {
                z1: f32x4::from_array(array::from_fn(|i| states[i].z1)),
            }
        }
    }

    impl Filter for OnePoleIirStatex4 {
        type Coeffs = OnePoleIirCoeffx4;

        #[inline(always)]
        fn process(&mut self, input: f32x4, coeff: &Self::Coeffs) -> f32x4 {
            self.z1 = (coeff.a0 * input) + (coeff.b1 * self.z1);
            coeff.m0 * input + coeff.m1 * self.z1
        }

        #[inline(always)]
        fn reset(&mut self) {
            self.z1 = f32x4::splat(0.0);
        }

        fn is_silent(&self, eps: f32) -> bool {
            self.z1.abs().simd_le(f32x4::splat(eps)).all()
        }
    }

    /// The state of eight single-pole IIR filters packed into an SIMD vector.
    #[derive(Default, Clone, Copy)]
    pub struct OnePoleIirStatex8 {
        z1: f32x8,
    }

    impl OnePoleIirStatex8 {
        pub const fn splat(state: OnePoleIirState) -> Self {
            Self {
                z1: f32x8::splat(state.z1),
            }
        }

        pub fn load(states: &[OnePoleIirState; 8]) -> Self {
            Self {
                z1: f32x8::from_array(array::from_fn(|i| states[i].z1)),
            }
        }
    }

    impl Filter for OnePoleIirStatex8 {
        type Coeffs = OnePoleIirCoeffx8;

        #[inline(always)]
        fn process(&mut self, input: f32x8, coeff: &Self::Coeffs) -> f32x8 {
            self.z1 = (coeff.a0 * input) + (coeff.b1 * self.z1);
            coeff.m0 * input + coeff.m1 * self.z1
        }

        #[inline(always)]
        fn reset(&mut self) {
            self.z1 = f32x8::splat(0.0);
        }

        fn is_silent(&self, eps: f32) -> bool {
            self.z1.abs().simd_le(f32x8::splat(eps)).all()
        }
    }
}
