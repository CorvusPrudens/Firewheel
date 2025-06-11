//! Based on https://github.com/MeadowlarkDAW/meadow-dsp/tree/main/meadow-dsp-mit with permission
use std::f32::consts::PI;

use crate::dsp::filter::{
    filter_trait::Filter, primitives::butterworth_coeffs::butterworth_coeffs, spec::FilterOrder,
};

/// The coefficients for an SVF (state variable filter) model.
#[derive(Default, Clone, Copy, Debug)]
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

    pub fn lowpass(
        order: FilterOrder,
        cutoff_hz: f32,
        q: f32,
        sample_rate_recip: f32,
        out: &mut [Self],
    ) {
        let num_svf = order / 2;
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = q.powf(1. / (num_svf as f32));

        let constants = butterworth_coeffs(order);

        for i in 0..num_svf {
            let q = q_norm * (constants[i] as f32);
            let k = 1.0 / q;

            out[i] = Self::from_g_and_k(g, k, 0.0, 0.0, 1.0);
        }
    }

    pub fn highpass(
        order: FilterOrder,
        cutoff_hz: f32,
        q: f32,
        sample_rate_recip: f32,
        out: &mut [Self],
    ) {
        let num_svf = order / 2;
        let g = g(cutoff_hz, sample_rate_recip);
        let q_norm = q.powf(1. / (num_svf as f32));

        let constants = butterworth_coeffs(order);

        for i in 0..num_svf {
            let q = q_norm * (constants[i] as f32);
            let k = 1.0 / q;

            out[i] = Self::from_g_and_k(g, k, 1.0, -k, -1.0);
        }
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
    fn is_silent(&self) -> bool {
        self.ic1eq.abs() <= Self::SILENT_THRESHOLD && self.ic2eq.abs() <= Self::SILENT_THRESHOLD
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
    fn is_silent(&self) -> bool {
        self.iter().all(|state| state.is_silent())
    }
}

impl<'a> Filter for &'a mut [SvfState] {
    type Coeffs = &'a [SvfCoeff];

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
    fn is_silent(&self) -> bool {
        self.iter().all(|state| state.is_silent())
    }
}

fn g(cutoff_hz: f32, sample_rate_recip: f32) -> f32 {
    (PI * cutoff_hz * sample_rate_recip).tan()
}

fn gain_db_to_a(gain_db: f32) -> f32 {
    10.0f32.powf(gain_db * (1.0 / 40.0))
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
        fn is_silent(&self) -> bool {
            self.ic1eq
                .abs()
                .simd_le(f32x4::splat(Self::SILENT_THRESHOLD))
                .all()
                && self
                    .ic2eq
                    .abs()
                    .simd_le(f32x4::splat(Self::SILENT_THRESHOLD))
                    .all()
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
        fn is_silent(&self) -> bool {
            self.ic1eq
                .abs()
                .simd_le(f32x8::splat(Self::SILENT_THRESHOLD))
                .all()
                && self
                    .ic2eq
                    .abs()
                    .simd_le(f32x8::splat(Self::SILENT_THRESHOLD))
                    .all()
        }
    }
}
