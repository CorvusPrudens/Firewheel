use super::{filter_trait::Filter, primitives::*, spec::FilterOrder};

/// A cascade of `N` biquads + an optional first order filter
#[derive(Clone, Copy)]
pub struct FilterCascade<const ORDER: FilterOrder> {
    first_order: Option<FirstOrderFilter>,
    biquads: [Biquad; ORDER],
}

pub struct FilterCascadeCoeffs<const ORDER: FilterOrder> {
    pub first_order: Option<FirstOrderCoeffs>,
    pub biquads: [BiquadCoeffs; ORDER],
}

impl<const ORDER: FilterOrder> Default for FilterCascade<ORDER> {
    fn default() -> Self {
        Self {
            first_order: Default::default(),
            biquads: [Biquad::default(); ORDER],
        }
    }
}

impl<const ORDER: FilterOrder> Filter for FilterCascade<ORDER> {
    type Coeffs = FilterCascadeCoeffs<ORDER>;

    fn reset(&mut self) {
        if let Some(first_order) = &mut self.first_order {
            first_order.reset();
        }
        for biquad in self.biquads.iter_mut() {
            biquad.reset();
        }
    }

    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        // Unwrapping coeffs.first_order_coeffs is okay because it is the caller's responsibility
        // to ensure that FirstOrderCoeffs are available if the filter needs them
        self.biquads.process(
            self.first_order
                .map(|mut first_order| first_order.process(x, &coeffs.first_order.unwrap()))
                .unwrap_or(x),
            &coeffs.biquads,
        )
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.first_order
            .is_none_or(|first_order| first_order.is_silent(eps))
            && self.biquads.is_silent(eps)
    }
}

/// A cascade of up to `N` biquads + an optional first order filter
/// Supports redesigning of filters with different steepness up to `N` but also uses space for `N` filters regardless of current design
#[derive(Clone, Copy)]
pub struct FilterCascadeUpTo<const ORDER: FilterOrder> {
    pub first_order: Option<FirstOrderFilter>,
    pub biquads: [Biquad; ORDER],
    pub num_biquads: usize,
}

impl<const ORDER: FilterOrder> Default for FilterCascadeUpTo<ORDER> {
    fn default() -> Self {
        Self {
            first_order: Default::default(),
            num_biquads: Default::default(),
            biquads: [Biquad::default(); ORDER],
        }
    }
}

impl<const ORDER: FilterOrder> Filter for FilterCascadeUpTo<ORDER> {
    type Coeffs = FilterCascadeCoeffs<ORDER>;

    fn reset(&mut self) {
        if let Some(first_order) = &mut self.first_order {
            first_order.reset();
        }
        for biquad in self.biquads.iter_mut() {
            biquad.reset();
        }
    }

    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        // Unwrapping coeffs.first_order_coeffs is okay because it is the caller's responsibility
        // to ensure that FirstOrderCoeffs are available if the filter needs them
        self.biquads
            .iter_mut()
            .zip(coeffs.biquads.iter())
            .take(self.num_biquads)
            .fold(
                self.first_order
                    .map(|mut first_order| first_order.process(x, &coeffs.first_order.unwrap()))
                    .unwrap_or(x),
                |acc, (biquad, coeffs)| biquad.process(acc, coeffs),
            )
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.first_order
            .is_none_or(|first_order| first_order.is_silent(eps))
            && self
                .biquads
                .iter()
                .take(self.num_biquads)
                .all(|biquad| biquad.is_silent(eps))
    }
}

/// Cascades for `M` filters of order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
pub struct ChainedCascade<const ORDER: FilterOrder, const M: usize> {
    cascades: [FilterCascade<ORDER>; M],
}

pub type ChainedCascadeCoeffs<const ORDER: FilterOrder, const M: usize> =
    [FilterCascadeCoeffs<ORDER>; M];

impl<const ORDER: FilterOrder, const M: usize> Default for ChainedCascade<ORDER, M> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const ORDER: FilterOrder, const M: FilterOrder> Filter for ChainedCascade<ORDER, M> {
    type Coeffs = ChainedCascadeCoeffs<ORDER, M>;

    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        self.cascades
            .iter_mut()
            .zip(coeffs.iter())
            .fold(x, |acc, (cascade, coeffs)| cascade.process(acc, coeffs))
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.cascades.iter().all(|cascade| cascade.is_silent(eps))
    }
}

/// Cascades for `M` filters of up to order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
/// Supports redesigning of filters with different steepness up to `N` but also uses space for `M * N` filters regardless of current design
pub struct ChainedCascadeUpTo<const ORDER: FilterOrder, const M: usize> {
    pub cascades: [FilterCascadeUpTo<ORDER>; M],
}

impl<const ORDER: FilterOrder, const M: usize> Default for ChainedCascadeUpTo<ORDER, M> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const ORDER: FilterOrder, const M: usize> Filter for ChainedCascadeUpTo<ORDER, M> {
    type Coeffs = ChainedCascadeCoeffs<ORDER, M>;

    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        self.cascades
            .iter_mut()
            .zip(coeffs.iter())
            .fold(x, |acc, (cascade, coeffs)| cascade.process(acc, coeffs))
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.cascades.iter().all(|cascade| cascade.is_silent(eps))
    }
}
