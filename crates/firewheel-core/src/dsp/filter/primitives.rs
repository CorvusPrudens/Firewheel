use super::filter_trait::Filter;

/// The coefficients for a generic first-order filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
#[derive(Default, Clone, Copy)]
pub struct FirstOrderCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub a1: f32,
}

/// A first-order filter
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
#[derive(Default, Clone, Copy)]
pub struct FirstOrderFilter {
    m: f32,
    pub coeffs: FirstOrderCoeffs,
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
