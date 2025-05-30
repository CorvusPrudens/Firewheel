use std::{f32::consts::PI, num::NonZero};

use super::filter_trait::Filter;

/// The coefficients for a generic first-order filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
#[derive(Debug, Default, Clone, Copy)]
pub struct FirstOrderCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub a1: f32,
}

impl FirstOrderCoeffs {
    /// Computes the digital first-order filter from a real analog pole for use in a lowpass filter.
    ///       s + a1
    /// `k` is the prewarp factor, use `prewarp_k` to compute it.
    /// This function is not super readable for performance reasons.
    /// More info here: <https://en.wikipedia.org/wiki/Bilinear_transform?useskin=vector#Transformation_for_a_general_first-order_continuous-time_filter>
    // TODO: discuss whether inlining always a good idea, compiler could do optimizations if it knows the value of a at compile time
    #[inline(always)]
    pub fn from_real_pole_lp(a1: f32, k: f32) -> Self {
        let norm = k + a1;
        let norm_recip = 1. / norm;
        // just a common constant so we can reuse results, includes gain normalization at DC
        let c = norm_recip * 0.5 * (a1 - k);
        Self {
            b0: c,
            b1: c,
            a1: (k - a1) * norm_recip,
        }
    }

    /// Computes the digital first-order filter from a real analog pole for use in a highpass filter.
    /// This function assumes that the analog pole was designed for a lowpass filter and transforms it.
    ///       s + a0
    ///    -> 1 / s + a0
    ///     = a0 * s + 1        (this results in the transfer function's numerator gaining an s^2)
    /// `k` is the prewarp factor, use `prewarp_k` to compute it.
    /// This function is not super readable for performance reasons.
    /// More info here: <https://en.wikipedia.org/wiki/Bilinear_transform?useskin=vector#Transformation_for_a_general_first-order_continuous-time_filter>
    // TODO: discuss whether inlining always a good idea, compiler could do optimizations if it knows the value of a at compile time
    #[inline(always)]
    pub fn from_real_pole_hp(a0: f32, k: f32) -> Self {
        let norm = a0 * k + 1.;
        let norm_recip = 1. / norm;
        // just some common constants so we can reuse results, `c` includes gain normalization at nyquist
        let digital_a1 = (a0 * k - 1.) * norm_recip;
        let c = 0.5 - 0.5 * digital_a1;
        Self {
            b0: c,
            b1: -c,
            a1: (a0 * k - 1.) * norm_recip,
        }
    }
}

/// A first-order filter
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
/// `w` is the memory, storing `b1 x[n-1] - a1 y[n-1]` for use in the next iteration
#[derive(Default, Clone, Copy)]
pub struct FirstOrderFilter {
    w: f32,
}

impl Filter for FirstOrderFilter {
    type Coeffs = FirstOrderCoeffs;

    fn reset(&mut self) {
        self.w = 0.0;
    }

    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        // Using transposed direct form II
        let y = coeffs.b0 * x + self.w;
        self.w = coeffs.b1 * x - coeffs.a1 * y;
        y
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.w.abs() <= eps
    }
}

/// The coefficients for a biquad filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] + b2 x[n-2] - a1 y[n-1] - a2 y[n-2]`
#[derive(Debug, Default, Clone, Copy)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    /// Computes the digital biquad from a pair of conjugate analog poles for use in a lowpass filter.
    ///       (s - (re(p) + i * im(p))) * (s + (re(p) - i * im(p)))
    ///     = s^2 - 2 * re(p) * s + (re(p)^2 + im(p)^2)
    ///    := s^2 + a1 * s + a2
    /// The biquad is normalized for gain at 0 Hz (DC).
    /// `k` is the prewarp factor, use `prewarp_k` to compute it.
    /// This function is not super readable for performance reasons.
    /// More info here: <https://en.wikipedia.org/wiki/Bilinear_transform?useskin=vector#General_second-order_biquad_transformation>
    // TODO: discuss whether inlining always a good idea, compiler could do optimizations if it knows the values of a1 or a2 at compile time
    #[inline(always)]
    pub fn from_conjugate_poles_lp(a1: f32, a2: f32, k: f32) -> Self {
        let k2 = k * k;
        let norm_recip = 1. / (k2 - a1 * k + a2);
        // just a common constant so we can reuse results, includes the normalization at DC
        let c = 0.25 + norm_recip * (0.75 * k2 + 0.25 * (a1 * k - a2));
        Self {
            b0: c,
            b1: 2. * c,
            b2: c,
            a1: (k2 - a2) * 2. * norm_recip,
            a2: (k2 + a1 * k + a2) * norm_recip,
        }
    }

    /// Computes the digital biquad from a pair of conjugate analog poles for use in a highpass filter.
    ///       s^2 + a1 * s + a2
    ///    -> 1 + a1 / s + a2 / s^2
    ///     = s^2 + a1 * s + a2        (this results in the numerator transfer function gaining an s^2)
    /// The effect is that the zeroes are placed at DC instead of nyquist.
    /// The biquad is normalized for gain at nyquist (sample_rate / 2).
    /// `k` is the prewarp factor, use `prewarp_k` to compute it.
    /// This function is not super readable for performance reasons.
    /// More info here: <https://en.wikipedia.org/wiki/Bilinear_transform?useskin=vector#General_second-order_biquad_transformation>
    // TODO: discuss whether inlining always a good idea, compiler could do optimizations if it knows the values of a1 or a2 at compile time
    #[inline(always)]
    pub fn from_conjugate_poles_hp(a1: f32, a2: f32, k: f32) -> Self {
        let k2 = k * k;
        let norm_recip = 1. / (k2 - a1 * k + a2);
        // just a common constant so we can reuse results, includes the normalization at nyquist
        let c = 0.25 + norm_recip * (0.75 * a2 + 0.25 * a1 * k - 0.25 * k2);
        Self {
            b0: c,
            b1: -2. * c,
            b2: c,
            a1: (k2 - a2) * 2. * norm_recip,
            a2: (k2 + a1 * k + a2) * norm_recip,
        }
    }
}

// A biquad filter
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] + b2 x[n-2] - a1 y[n-1] - a2 y[n-2]`
/// `s1` stores `b1 x[n-1] - a1 y[n-1] + s2` for use in the next iteration
/// `s2` stores `b2 x[n-2] - a2 y[n-2]` for use in the iteration
#[derive(Default, Clone, Copy)]
pub struct Biquad {
    s1: f32,
    s2: f32,
}

impl Filter for Biquad {
    type Coeffs = BiquadCoeffs;

    fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        // Using transposed direct form II
        // For more info see <https://en.wikipedia.org/wiki/Digital_biquad_filter?useskin=vector#Transposed_direct_form_2>
        let y = coeffs.b0 * x + self.s1;
        self.s1 = self.s2 + coeffs.b1 * x - coeffs.a1 * y;
        self.s2 = coeffs.b2 * x - coeffs.a2 * y;
        y
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.s1.abs() <= eps && self.s2.abs() <= eps
    }
}

impl<const N: usize> Filter for [Biquad; N] {
    type Coeffs = [BiquadCoeffs; N];

    fn reset(&mut self) {
        for biquad in self.iter_mut() {
            biquad.reset();
        }
    }

    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        self.iter_mut()
            .zip(coeffs)
            .fold(x, |acc, (biquad, coeffs)| biquad.process(acc, coeffs))
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.iter().all(|biquad| biquad.is_silent(eps))
    }
}

/// Computes the prewarp factor `K` needed for the bilinear transform.
pub fn prewarp_k(frequency_hz: f32, sample_rate: NonZero<u32>) -> f32 {
    let sample_rate = sample_rate.get() as f32;
    (PI * frequency_hz / sample_rate).tan()
}
