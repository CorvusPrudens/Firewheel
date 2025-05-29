use super::{filter_trait::Filter, primitives::*, spec::FilterOrder};

/// A cascade of `N` biquads + an optional first order filter
#[derive(Clone, Copy)]
pub struct FilterCascade<const ORDER: FilterOrder> {
    first_order: Option<FirstOrderFilter>,
    biquads: [Biquad; ORDER],
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
    fn process(&mut self, x: f32) -> f32 {
        self.biquads.process(
            self.first_order
                .map(|mut first_order| first_order.process(x))
                .unwrap_or(x),
        )
    }
}

/// A cascade of up to `N` biquads + an optional first order filter
/// Supports redesigning of filters with different steepness up to `N` but also uses space for `N` filters regardless of current design
#[derive(Clone, Copy)]
pub struct FilterCascadeUpTo<const ORDER: FilterOrder> {
    first_order: Option<FirstOrderFilter>,
    num_biquads: usize,
    biquads: [Biquad; ORDER],
}

impl<const ORDER: FilterOrder> FilterCascadeUpTo<ORDER> {
    fn new(first_order: Option<FirstOrderFilter>, biquads: [Biquad; ORDER]) -> Self {
        Self {
            first_order,
            num_biquads: ORDER,
            biquads,
        }
    }
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
    fn process(&mut self, x: f32) -> f32 {
        self.biquads.iter_mut().take(self.num_biquads).fold(
            self.first_order
                .map(|mut first_order| first_order.process(x))
                .unwrap_or(x),
            |acc, biquad| biquad.process(acc),
        )
    }
}

/// Cascades for `M` filters of order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
pub struct ChainedCascade<const ORDER: FilterOrder, const M: usize> {
    cascades: [FilterCascade<ORDER>; M],
}

impl<const ORDER: FilterOrder, const M: usize> Default for ChainedCascade<ORDER, M> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const ORDER: FilterOrder, const M: FilterOrder> Filter for ChainedCascade<ORDER, M> {
    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32) -> f32 {
        self.cascades
            .iter_mut()
            .fold(x, |acc, cascade| cascade.process(acc))
    }
}

/// Cascades for `M` filters of up to order `N` each
/// Useful for filters that chain multiple filters together, like bandpass or bandstop
/// Supports redesigning of filters with different steepness up to `N` but also uses space for `M * N` filters regardless of current design
pub struct ChainedCascadeUpTo<const ORDER: FilterOrder, const M: usize> {
    cascades: [FilterCascadeUpTo<ORDER>; M],
}

impl<const ORDER: FilterOrder, const M: usize> Default for ChainedCascadeUpTo<ORDER, M> {
    fn default() -> Self {
        Self {
            cascades: [Default::default(); M],
        }
    }
}

impl<const ORDER: FilterOrder, const M: usize> Filter for ChainedCascadeUpTo<ORDER, M> {
    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }
    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32) -> f32 {
        self.cascades
            .iter_mut()
            .fold(x, |acc, cascade| cascade.process(acc))
    }
}
