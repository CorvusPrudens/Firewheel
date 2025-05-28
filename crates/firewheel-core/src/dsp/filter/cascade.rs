use super::{filter_trait::Filter, primitives::*};

/// A cascade of `N` biquads + an optional first order filter
#[derive(Clone, Copy)]
pub struct FilterCascade<const N: usize> {
    first_order: Option<FirstOrderFilter>,
    biquads: [Biquad; N],
}

impl<const ORDER: usize> Default for FilterCascade<ORDER> {
    fn default() -> Self {
        Self {
            first_order: Default::default(),
            biquads: [Biquad::default(); ORDER],
        }
    }
}

pub struct CascadeCoeff<const ORDER: usize> {
    first_order_coeff: FirstOrderCoeff,
    biquad_coeffs: [BiquadCoeff; ORDER],
}

impl<const ORDER: usize> Filter for FilterCascade<ORDER> {
    type Coeff = CascadeCoeff<ORDER>;

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

/// A cascade of up to `N` biquads + an optional first order filter
/// Supports redesigning of filters with different steepness up to `2N + 1` but also uses space for `M * N` filters regardless of current design
#[derive(Clone, Copy)]
pub struct FilterCascadeUpTo<const N: usize> {
    first_order: Option<FirstOrderFilter>,
    num_biquads: usize,
    biquads: [Biquad; N],
}

impl<const ORDER: usize> Default for FilterCascadeUpTo<ORDER> {
    fn default() -> Self {
        Self {
            first_order: Default::default(),
            num_biquads: Default::default(),
            biquads: [Biquad::default(); ORDER],
        }
    }
}

impl<const ORDER: usize> Filter for FilterCascadeUpTo<ORDER> {
    type Coeff = CascadeCoeff<ORDER>;

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
        coeffs
            .biquad_coeffs
            .into_iter()
            .zip(self.biquads.iter_mut())
            .take(self.num_biquads)
            .fold(
                self.first_order
                    .map(|mut first_order| first_order.process(x, coeffs.first_order_coeff))
                    .unwrap_or(x),
                |acc, (coeff, biquad)| biquad.process(acc, coeff),
            )
    }
}

/// Cascades for `M` filters of order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
pub struct ChainedCascade<const M: usize, const ORDER: usize> {
    cascades: [FilterCascade<ORDER>; M],
}

pub type ChainedCascadeCoeff<const M: usize, const ORDER: usize> =
    [<FilterCascade<ORDER> as Filter>::Coeff; M];

impl<const M: usize, const N: usize> Default for ChainedCascade<M, N> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const M: usize, const ORDER: usize> Filter for ChainedCascade<M, ORDER> {
    type Coeff = ChainedCascadeCoeff<M, ORDER>;

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

/// Cascades for `M` filters of up to order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
/// Supports redesigning of filters with different steepness up to `N` but also uses space for `M * N` filters regardless of current design
pub struct ChainedCascadeUpTo<const M: usize, const ORDER: usize> {
    cascades: [FilterCascadeUpTo<ORDER>; M],
}

pub type ChainedCascadeUpToCoeff<const M: usize, const ORDER: usize> =
    [<FilterCascadeUpTo<ORDER> as Filter>::Coeff; M];

impl<const M: usize, const ORDER: usize> Default for ChainedCascadeUpTo<M, ORDER> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const M: usize, const ORDER: usize> Filter for ChainedCascadeUpTo<M, ORDER> {
    type Coeff = ChainedCascadeUpToCoeff<M, ORDER>;

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
