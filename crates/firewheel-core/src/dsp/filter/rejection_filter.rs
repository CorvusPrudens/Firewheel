use super::{
    cascade::{Cascade, CascadeCoeff, ChainedCascade, ChainedCascadeCoeff},
    filter_trait::Filter,
};

#[derive(Default)]
pub struct Lowpass<const N: usize> {
    cascade: Cascade<N>,
}
type LowpassCoeffs<const N: usize> = CascadeCoeff<N>;

#[derive(Default)]
pub struct Highpass<const N: usize> {
    cascade: Cascade<N>,
}
type HighpassCoeffs<const N: usize> = CascadeCoeff<N>;

#[derive(Default)]
pub struct Bandpass<const N: usize> {
    cascades: ChainedCascade<2, N>,
}
type BandpassCoeffs<const N: usize> = ChainedCascadeCoeff<2, N>;

#[derive(Default)]
pub struct Bandstop<const N: usize> {
    cascades: ChainedCascade<2, N>,
}
type BandstopCoeffs<const N: usize> = ChainedCascadeCoeff<2, N>;

impl<const N: usize> Filter for Lowpass<N> {
    type Coeff = LowpassCoeffs<N>;

    fn reset(&mut self) {
        self.cascade.reset();
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.cascade.process(x, coeffs)
    }
}
impl<const N: usize> Filter for Highpass<N> {
    type Coeff = HighpassCoeffs<N>;

    fn reset(&mut self) {
        self.cascade.reset();
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.cascade.process(x, coeffs)
    }
}
impl<const N: usize> Filter for Bandpass<N> {
    type Coeff = BandpassCoeffs<N>;

    fn reset(&mut self) {
        self.cascades.reset();
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.cascades.process(x, coeffs)
    }
}
impl<const N: usize> Filter for Bandstop<N> {
    type Coeff = BandstopCoeffs<N>;

    fn reset(&mut self) {
        self.cascades.reset();
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.cascades.process(x, coeffs)
    }
}
