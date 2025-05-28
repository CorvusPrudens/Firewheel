use super::{filter_trait::Filter, primitives::*};

/// A cascade of biquads + a first order filter for filters `2N + 1` of poles (and zeroes)
#[derive(Clone, Copy)]
pub struct Cascade<const N: usize> {
    first_order: Option<FirstOrder>,
    biquads: [Biquad; N],
}

impl<const N: usize> Default for Cascade<N> {
    fn default() -> Self {
        Self {
            first_order: Default::default(),
            biquads: [Biquad::default(); N],
        }
    }
}

pub struct CascadeCoeff<const N: usize> {
    first_order_coeff: FirstOrderCoeff,
    biquad_coeffs: [BiquadCoeff; N],
}

impl<const N: usize> Filter for Cascade<N> {
    type Coeff = CascadeCoeff<N>;

    fn reset(&mut self) {
        if let Some(first_order) = &mut self.first_order {
            first_order.reset();
        }
        for biquad in self.biquads.iter_mut() {
            biquad.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.biquads.process(
            self.first_order
                .map(|mut first_order| first_order.process(x, coeffs.first_order_coeff))
                .unwrap_or(x),
            coeffs.biquad_coeffs,
        )
    }
}

/// Cascades for `M` filters of order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
pub struct ChainedCascade<const M: usize, const N: usize> {
    cascades: [Cascade<N>; M],
}

pub type ChainedCascadeCoeff<const M: usize, const N: usize> = [<Cascade<N> as Filter>::Coeff; M];

impl<const M: usize, const N: usize> Default for ChainedCascade<M, N> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const M: usize, const N: usize> Filter for ChainedCascade<M, N> {
    type Coeff = ChainedCascadeCoeff<M, N>;

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
