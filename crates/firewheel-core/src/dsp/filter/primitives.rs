use std::f32::consts::TAU;

/// The coefficients for a generic first-order filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] - a1 y[n-1]`
pub struct FirstOrderCoeff {
    pub b0: f32,
    pub b1: f32,
    pub a1: f32,
}

#[derive(Default, Clone, Copy)]
pub struct FirstOrder {
    m: f32,
}

impl FirstOrder {
    pub fn reset(&mut self) {
        self.m = 0.0;
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    pub fn process(&mut self, x: f32, coeff: FirstOrderCoeff) -> f32 {
        let y = self.m + coeff.b0 * x;
        self.m = coeff.b1 * x - coeff.a1 * y;
        y
    }
}

/// The coefficients for a biquad filter.
/// This filter has the form: `y[n] = b0 x[n] + b1 x[n-1] + b2 x[n-2] - a1 y[n-1] - a2 y[n-2]`
pub struct BiquadCoeff {
    pub a1: f32,
    pub a2: f32,
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
}

#[derive(Default, Clone, Copy)]
pub struct Biquad {
    d1: f32,
    d2: f32,
}

impl Biquad {
    pub fn reset(&mut self) {
        self.d1 = 0.0;
        self.d2 = 0.0;
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    pub fn process(&mut self, x: f32, coeff: BiquadCoeff) -> f32 {
        // Using transposed direct from II
        let y = coeff.b0 * x + self.d1;
        self.d1 = coeff.b1 * x + coeff.a1 * y + self.d2;
        self.d2 = coeff.b2 * x + coeff.a2 * y;
        y
    }
}
