use std::f32::consts::PI;

use super::filter_trait::Filter;

/// The coefficients for a generic first-order filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
#[derive(Default, Clone, Copy)]
pub struct FirstOrderCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub a1: f32,
}

impl FirstOrderCoeffs {
    /// Computes the digital first-order filter from a real analog pole.
    ///       s + a
    /// `k` is the prewarp factor, use `prewarp_k` to compute it.
    // TODO: discuss whether inlining always a good idea, compiler could do optimizations if it knows the value of a at compile time
    #[inline(always)]
    pub fn from_real_pole(a: f32, k: f32) -> Self {
        let norm = a + k;
        let norm_recip = 1. / norm;
        Self {
            b0: norm_recip,
            b1: norm_recip,
            a1: (a - k) * norm_recip,
        }
    }
}

/// A first-order filter
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
#[derive(Default, Clone, Copy)]
pub struct FirstOrderFilter {
    m: f32,
    pub coeffs: FirstOrderCoeffs,
}

impl FirstOrderFilter {
    pub fn with_coeffs(coeffs: FirstOrderCoeffs) -> Self {
        Self { m: 0., coeffs }
    }
}

impl Filter for FirstOrderFilter {
    fn reset(&mut self) {
        self.m = 0.0;
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.m + self.coeffs.b0 * x;
        self.m = self.coeffs.b1 * x - self.coeffs.a1 * y;
        y
    }
}

/// The coefficients for a biquad filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] + b2 x[n-2] - a1 y[n-1] - a2 y[n-2]`
#[derive(Default, Clone, Copy)]
pub struct BiquadCoeffs {
    pub a1: f32,
    pub a2: f32,
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
}

impl BiquadCoeffs {
    /// Computes the digital biquad from a pair of conjugate analog poles.
    ///       s^2 + a1 * s + a2
    /// `k` is the prewarp factor, use `prewarp_k` to compute it.
    // TODO: discuss whether inlining always a good idea, compiler could do optimizations if it knows the values of a1 or a2 at compile time
    #[inline(always)]
    pub fn from_conjugate_pole(a1: f32, a2: f32, k: f32) -> Self {
        let k2 = k * k;
        let norm = k * a1 + k2 * a2;
        let norm_recip = 1. / norm;
        Self {
            a1: -2. * k2 * a2 * norm_recip,
            a2: (1. - k * a1 + k2 * a2) * norm_recip,
            b0: norm_recip,
            b1: 2. * norm_recip,
            b2: norm_recip,
        }
    }
}

// A biquad filter
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] + b2 x[n-2] - a1 y[n-1] - a2 y[n-2]`
#[derive(Default, Clone, Copy)]
pub struct Biquad {
    d1: f32,
    d2: f32,
    pub coeffs: BiquadCoeffs,
}

impl Filter for Biquad {
    fn reset(&mut self) {
        self.d1 = 0.0;
        self.d2 = 0.0;
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32) -> f32 {
        // Using transposed direct from II
        let y = self.coeffs.b0 * x + self.d1;
        self.d1 = self.coeffs.b1 * x + self.coeffs.a1 * y + self.d2;
        self.d2 = self.coeffs.b2 * x + self.coeffs.a2 * y;
        y
    }
}

impl<const N: usize> Filter for [Biquad; N] {
    fn reset(&mut self) {
        for biquad in self.iter_mut() {
            biquad.reset();
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.iter_mut().fold(x, |acc, biquad| biquad.process(acc))
    }
}

/// Computes the prewarp factor `K` needed for the bilinear transform.
pub fn prewarp_k(frequency: f32, sample_rate: f32) -> f32 {
    (2.0 * sample_rate) * (PI * frequency / sample_rate).tan()
}
