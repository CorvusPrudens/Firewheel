use super::{
    filter_trait::Filter,
    primitives::*,
    spec::{OddOrderSteepness, Steepness},
};

type CascadeEvenCoeff<const N: usize> = [BiquadCoeff; N];

/// A cascade of biquads for filters with `2N` poles (and zeroes)
#[derive(Clone, Copy)]
pub struct CascadeEven<const N: usize> {
    biquads: [Biquad; N],
}

impl<const N: usize> Filter for CascadeEven<N> {
    type Coeff = CascadeEvenCoeff<N>;

    fn reset(&mut self) {
        for biquad in self.biquads.iter_mut() {
            biquad.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        coeffs
            .into_iter()
            .zip(self.biquads.iter_mut())
            .fold(x, |acc, (coeff, biquad)| biquad.process(acc, coeff))
    }
}

impl<const N: usize> Default for CascadeEven<N> {
    fn default() -> Self {
        Self {
            biquads: [Biquad::default(); N],
        }
    }
}

/// A cascade of biquads + a first order filter for filters `2N + 1` of poles (and zeroes)
#[derive(Default, Clone, Copy)]
pub struct CascadeOdd<const N: usize> {
    first_order: FirstOrder,
    biquads: CascadeEven<N>,
}

pub struct CascadeOddCoeff<const N: usize> {
    first_order_coeff: FirstOrderCoeff,
    biquad_coeffs: [BiquadCoeff; N],
}

impl<const N: usize> Filter for CascadeOdd<N> {
    type Coeff = CascadeOddCoeff<N>;

    fn reset(&mut self) {
        self.first_order.reset();
        self.biquads.reset();
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.biquads.process(
            self.first_order.process(x, coeffs.first_order_coeff),
            coeffs.biquad_coeffs,
        )
    }
}

impl<const N: usize> CascadeOddCoeff<N> {
    #[inline]
    pub fn new_butterworth(cutoff_hz: f32, sample_rate_recip: f32) -> CascadeOddCoeff<N> {
        todo!()
    }
}

/// Cascades for `M` filters of order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
pub struct ChainedCascade<const M: usize, C: Filter> {
    cascades: [C; M],
}

impl<const M: usize, C: Filter + Copy + Default> Default for ChainedCascade<M, C> {
    fn default() -> Self {
        Self {
            cascades: [C::default(); M],
        }
    }
}

impl<const M: usize, C: Filter + Copy> Filter for ChainedCascade<M, C> {
    type Coeff = [C::Coeff; M];

    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        coeffs
            .into_iter()
            .zip(self.cascades.iter_mut())
            .fold(x, |acc, (coeff, cascade)| cascade.process(acc, coeff))
    }
}
