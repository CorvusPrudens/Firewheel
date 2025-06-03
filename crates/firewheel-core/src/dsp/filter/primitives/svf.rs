//! Based on https://github.com/MeadowlarkDAW/meadow-dsp/tree/main/meadow-dsp-mit with permission
use std::f32::consts::PI;

use std::f64::consts::FRAC_1_SQRT_2;

use crate::dsp::filter::filter_trait::Filter;

pub const Q_BUTTERWORTH_ORD2: f64 = FRAC_1_SQRT_2;
pub const Q_BUTTERWORTH_ORD4: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_6];
pub const Q_BUTTERWORTH_ORD6: [f64; 3] = [
    0.517_638_090_205_041_5,
    FRAC_1_SQRT_2,
    1.931_851_652_578_136_6,
];
pub const Q_BUTTERWORTH_ORD8: [f64; 4] = [
    0.509_795_579_104_159_2,
    0.601_344_886_935_045_3,
    0.899_976_223_136_415_7,
    2.562_915_447_741_506_4,
];

pub const ORD4_Q_SCALE: f64 = 0.35;
pub const ORD6_Q_SCALE: f64 = 0.2;
pub const ORD8_Q_SCALE: f64 = 0.14;

/// The coefficients for an SVF (state variable filter) model.
#[derive(Default, Clone, Copy)]
pub struct SvfCoeff {
    pub a1: f32,
    pub a2: f32,
    pub a3: f32,

    pub m0: f32,
    pub m1: f32,
    pub m2: f32,
}

impl SvfCoeff {
    pub const NO_OP: Self = Self {
        a1: 0.0,
        a2: 0.0,
        a3: 0.0,
        m0: 1.0,
        m1: 0.0,
        m2: 0.0,
    };

    pub fn lowpass_ord2(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> Self {
        let g = g(cutoff_hz, sample_rate_recip);
        let k = 1.0 / q;

        Self::from_g_and_k(g, k, 0.0, 0.0, 1.0)
    }

    pub fn lowpass_ord4(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> [Self; 2] {
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = scale_q_norm_for_order(q_norm(q), ORD4_Q_SCALE as f32);

        std::array::from_fn(|i| {
            let q = q_norm * Q_BUTTERWORTH_ORD4[i] as f32;
            let k = 1.0 / q;

            Self::from_g_and_k(g, k, 0.0, 0.0, 1.0)
        })
    }

    pub fn lowpass_ord6(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> [Self; 3] {
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = scale_q_norm_for_order(q_norm(q), ORD4_Q_SCALE as f32);

        std::array::from_fn(|i| {
            let q = q_norm * Q_BUTTERWORTH_ORD6[i] as f32;
            let k = 1.0 / q;

            Self::from_g_and_k(g, k, 0.0, 0.0, 1.0)
        })
    }

    pub fn lowpass_ord8(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> [Self; 4] {
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = scale_q_norm_for_order(q_norm(q), ORD8_Q_SCALE as f32);

        std::array::from_fn(|i| {
            let q = q_norm * Q_BUTTERWORTH_ORD8[i] as f32;
            let k = 1.0 / q;

            Self::from_g_and_k(g, k, 0.0, 0.0, 1.0)
        })
    }

    pub fn highpass_ord2(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> Self {
        let g = g(cutoff_hz, sample_rate_recip);
        let k = 1.0 / q;

        Self::from_g_and_k(g, k, 1.0, -k, -1.0)
    }

    pub fn highpass_ord4(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> [Self; 2] {
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = scale_q_norm_for_order(q_norm(q), ORD4_Q_SCALE as f32);

        std::array::from_fn(|i| {
            let q = q_norm * Q_BUTTERWORTH_ORD4[i] as f32;
            let k = 1.0 / q;

            Self::from_g_and_k(g, k, 1.0, -k, -1.0)
        })
    }

    pub fn highpass_ord6(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> [Self; 3] {
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = scale_q_norm_for_order(q_norm(q), ORD6_Q_SCALE as f32);

        std::array::from_fn(|i| {
            let q = q_norm * Q_BUTTERWORTH_ORD6[i] as f32;
            let k = 1.0 / q;

            Self::from_g_and_k(g, k, 1.0, -k, -1.0)
        })
    }

    pub fn highpass_ord8(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> [Self; 4] {
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = scale_q_norm_for_order(q_norm(q), ORD8_Q_SCALE as f32);

        std::array::from_fn(|i| {
            let q = q_norm * Q_BUTTERWORTH_ORD8[i] as f32;
            let k = 1.0 / q;

            Self::from_g_and_k(g, k, 1.0, -k, -1.0)
        })
    }

    pub fn notch(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> Self {
        let g = g(cutoff_hz, sample_rate_recip);
        let k = 1.0 / q;

        Self::from_g_and_k(g, k, 1.0, -k, 0.0)
    }

    pub fn bell(cutoff_hz: f32, q: f32, gain_db: f32, sample_rate_recip: f32) -> Self {
        let a = gain_db_to_a(gain_db);

        let g = g(cutoff_hz, sample_rate_recip);
        let k = 1.0 / (q * a);

        Self::from_g_and_k(g, k, 1.0, k * (a * a - 1.0), 0.0)
    }

    pub fn low_shelf(cutoff_hz: f32, q: f32, gain_db: f32, sample_rate_recip: f32) -> Self {
        let a = gain_db_to_a(gain_db);

        let g = (PI * cutoff_hz * sample_rate_recip).tan() / a.sqrt();
        let k = 1.0 / q;

        Self::from_g_and_k(g, k, 1.0, k * (a - 1.0), a * a - 1.0)
    }

    pub fn high_shelf(cutoff_hz: f32, q: f32, gain_db: f32, sample_rate_recip: f32) -> Self {
        let a = gain_db_to_a(gain_db);

        let g = (PI * cutoff_hz * sample_rate_recip).tan() / a.sqrt();
        let k = 1.0 / q;

        Self::from_g_and_k(g, k, a * a, k * (1.0 - a) * a, 1.0 - a * a)
    }

    pub fn allpass(cutoff_hz: f32, q: f32, sample_rate_recip: f32) -> Self {
        let g = g(cutoff_hz, sample_rate_recip);
        let k = 1.0 / q;

        Self::from_g_and_k(g, k, 1.0, -2.0 * k, 0.0)
    }

    pub fn from_g_and_k(g: f32, k: f32, m0: f32, m1: f32, m2: f32) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        Self {
            a1,
            a2,
            a3,
            m0,
            m1,
            m2,
        }
    }
}

/// The state of an SVF (state variable filter) model.
#[derive(Default, Clone, Copy)]
pub struct SvfState {
    pub ic1eq: f32,
    pub ic2eq: f32,
}

impl Filter for SvfState {
    type Coeffs = SvfCoeff;

    #[inline(always)]
    fn process(&mut self, input: f32, coeff: &Self::Coeffs) -> f32 {
        let v3 = input - self.ic2eq;
        let v1 = coeff.a1 * self.ic1eq + coeff.a2 * v3;
        let v2 = self.ic2eq + coeff.a2 * self.ic1eq + coeff.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        coeff.m0 * input + coeff.m1 * v1 + coeff.m2 * v2
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    #[inline(always)]
    fn is_silent(&self, eps: f32) -> bool {
        self.ic1eq.abs() <= eps && self.ic2eq.abs() <= eps
    }
}

impl<const N: usize> Filter for [SvfState; N] {
    type Coeffs = [SvfCoeff; N];

    #[inline(always)]
    fn process(&mut self, input: f32, coeff: &Self::Coeffs) -> f32 {
        self.iter_mut()
            .zip(coeff.iter())
            .fold(input, |acc, (state, coeff)| state.process(acc, coeff))
    }

    #[inline(always)]
    fn reset(&mut self) {
        for state in self.iter_mut() {
            state.reset();
        }
    }

    #[inline(always)]
    fn is_silent(&self, eps: f32) -> bool {
        self.iter().all(|state| state.is_silent(eps))
    }
}

fn g(cutoff_hz: f32, sample_rate_recip: f32) -> f32 {
    (PI * cutoff_hz * sample_rate_recip).tan()
}

fn q_norm(q: f32) -> f32 {
    q * (1.0 / Q_BUTTERWORTH_ORD2 as f32)
}

fn gain_db_to_a(gain_db: f32) -> f32 {
    10.0f32.powf(gain_db * (1.0 / 40.0))
}

fn scale_q_norm_for_order(q_norm: f32, scale: f32) -> f32 {
    if q_norm > 1.0 {
        1.0 + ((q_norm - 1.0) * scale)
    } else {
        q_norm
    }
}

#[cfg(feature = "portable-simd")]
pub mod simd {
    use std::{
        array,
        simd::{cmp::SimdPartialOrd, f32x4, f32x8, num::SimdFloat},
    };

    use crate::dsp::filter::filter_trait::Filter;

    use super::{SvfCoeff, SvfState};

    /// The coefficients of four SVF (state variable filter) models packed
    /// into an SIMD vector.
    pub struct SvfCoeffx4 {
        pub a1: f32x4,
        pub a2: f32x4,
        pub a3: f32x4,

        pub m0: f32x4,
        pub m1: f32x4,
        pub m2: f32x4,
    }

    impl SvfCoeffx4 {
        pub const fn splat(coeffs: SvfCoeff) -> Self {
            Self {
                a1: f32x4::splat(coeffs.a1),
                a2: f32x4::splat(coeffs.a2),
                a3: f32x4::splat(coeffs.a3),
                m0: f32x4::splat(coeffs.m0),
                m1: f32x4::splat(coeffs.m1),
                m2: f32x4::splat(coeffs.m2),
            }
        }

        pub fn load(coeffs: &[SvfCoeff; 4]) -> Self {
            Self {
                a1: f32x4::from_array(array::from_fn(|i| coeffs[i].a1)),
                a2: f32x4::from_array(array::from_fn(|i| coeffs[i].a2)),
                a3: f32x4::from_array(array::from_fn(|i| coeffs[i].a3)),
                m0: f32x4::from_array(array::from_fn(|i| coeffs[i].m0)),
                m1: f32x4::from_array(array::from_fn(|i| coeffs[i].m1)),
                m2: f32x4::from_array(array::from_fn(|i| coeffs[i].m2)),
            }
        }
    }

    /// The coefficients of eight SVF (state variable filter) models packed
    /// into an SIMD vector.
    pub struct SvfCoeffx8 {
        pub a1: f32x8,
        pub a2: f32x8,
        pub a3: f32x8,

        pub m0: f32x8,
        pub m1: f32x8,
        pub m2: f32x8,
    }

    impl SvfCoeffx8 {
        pub const fn splat(coeffs: SvfCoeff) -> Self {
            Self {
                a1: f32x8::splat(coeffs.a1),
                a2: f32x8::splat(coeffs.a2),
                a3: f32x8::splat(coeffs.a3),
                m0: f32x8::splat(coeffs.m0),
                m1: f32x8::splat(coeffs.m1),
                m2: f32x8::splat(coeffs.m2),
            }
        }

        pub fn load(coeffs: &[SvfCoeff; 8]) -> Self {
            Self {
                a1: f32x8::from_array(array::from_fn(|i| coeffs[i].a1)),
                a2: f32x8::from_array(array::from_fn(|i| coeffs[i].a2)),
                a3: f32x8::from_array(array::from_fn(|i| coeffs[i].a3)),
                m0: f32x8::from_array(array::from_fn(|i| coeffs[i].m0)),
                m1: f32x8::from_array(array::from_fn(|i| coeffs[i].m1)),
                m2: f32x8::from_array(array::from_fn(|i| coeffs[i].m2)),
            }
        }
    }

    /// The state of four SVF (state variable filter) models packed into an
    /// SIMD vector.
    #[derive(Default, Clone, Copy)]
    pub struct SvfStatex4 {
        pub ic1eq: f32x4,
        pub ic2eq: f32x4,
    }

    impl SvfStatex4 {
        pub const fn splat(state: SvfState) -> Self {
            Self {
                ic1eq: f32x4::splat(state.ic1eq),
                ic2eq: f32x4::splat(state.ic2eq),
            }
        }

        pub fn load(&mut self, states: &[SvfState; 4]) -> Self {
            Self {
                ic1eq: f32x4::from_array(array::from_fn(|i| states[i].ic1eq)),
                ic2eq: f32x4::from_array(array::from_fn(|i| states[i].ic2eq)),
            }
        }

        pub fn store(&self, states: &mut [SvfState; 4]) {
            let ic1eq = self.ic1eq.to_array();
            let ic2eq = self.ic2eq.to_array();

            for (i, s) in states.iter_mut().enumerate() {
                s.ic1eq = ic1eq[i];
                s.ic2eq = ic2eq[i];
            }
        }
    }

    impl Filter for SvfStatex4 {
        type Coeffs = SvfCoeffx4;

        #[inline(always)]
        fn process(&mut self, input: f32x4, coeff: &Self::Coeffs) -> f32x4 {
            const V_2: f32x4 = f32x4::from_array([2.0; 4]);

            let v3 = input - self.ic2eq;
            let v1 = coeff.a1 * self.ic1eq + coeff.a2 * v3;
            let v2 = self.ic2eq + coeff.a2 * self.ic1eq + coeff.a3 * v3;
            self.ic1eq = V_2 * v1 - self.ic1eq;
            self.ic2eq = V_2 * v2 - self.ic2eq;

            coeff.m0 * input + coeff.m1 * v1 + coeff.m2 * v2
        }

        #[inline(always)]
        fn reset(&mut self) {
            self.ic1eq = f32x4::splat(0.0);
            self.ic2eq = f32x4::splat(0.0);
        }

        #[inline(always)]
        fn is_silent(&self, eps: f32) -> bool {
            self.ic1eq.abs().simd_le(f32x4::splat(eps)).all()
                && self.ic2eq.abs().simd_le(f32x4::splat(eps)).all()
        }
    }

    /// The state of eight SVF (state variable filter) models packed into an
    /// SIMD vector.
    #[derive(Default, Clone, Copy)]
    pub struct SvfStatex8 {
        pub ic1eq: f32x8,
        pub ic2eq: f32x8,
    }

    impl SvfStatex8 {
        pub const fn splat(state: SvfState) -> Self {
            Self {
                ic1eq: f32x8::splat(state.ic1eq),
                ic2eq: f32x8::splat(state.ic2eq),
            }
        }

        pub fn load(&mut self, states: &[SvfState; 8]) -> Self {
            Self {
                ic1eq: f32x8::from_array(array::from_fn(|i| states[i].ic1eq)),
                ic2eq: f32x8::from_array(array::from_fn(|i| states[i].ic2eq)),
            }
        }

        pub fn store(&self, states: &mut [SvfState; 8]) {
            let ic1eq = self.ic1eq.to_array();
            let ic2eq = self.ic2eq.to_array();

            for (i, s) in states.iter_mut().enumerate() {
                s.ic1eq = ic1eq[i];
                s.ic2eq = ic2eq[i];
            }
        }
    }

    impl Filter for SvfStatex8 {
        type Coeffs = SvfCoeffx8;

        #[inline(always)]
        fn process(&mut self, input: f32x8, coeff: &Self::Coeffs) -> f32x8 {
            const V_2: f32x8 = f32x8::from_array([2.0; 8]);

            let v3 = input - self.ic2eq;
            let v1 = coeff.a1 * self.ic1eq + coeff.a2 * v3;
            let v2 = self.ic2eq + coeff.a2 * self.ic1eq + coeff.a3 * v3;
            self.ic1eq = V_2 * v1 - self.ic1eq;
            self.ic2eq = V_2 * v2 - self.ic2eq;

            coeff.m0 * input + coeff.m1 * v1 + coeff.m2 * v2
        }

        #[inline(always)]
        fn reset(&mut self) {
            self.ic1eq = f32x8::splat(0.0);
            self.ic2eq = f32x8::splat(0.0);
        }

        #[inline(always)]
        fn is_silent(&self, eps: f32) -> bool {
            self.ic1eq.abs().simd_le(f32x8::splat(eps)).all()
                && self.ic2eq.abs().simd_le(f32x8::splat(eps)).all()
        }
    }
}
