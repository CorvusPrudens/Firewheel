use crate::dsp::filter::{
    filter_trait::Filter,
    primitives::{
        one_pole_iir::{OnePoleIirCoeff, OnePoleIirState},
        svf::{SvfCoeff, SvfState},
    },
    spec::FilterOrder,
};

/// A cascade of `N` biquads + an optional first order filter
#[derive(Clone, Copy)]
pub struct FilterCascade<const ORDER: FilterOrder> {
    // TODO: think about whether Option is even needed. It could just be used depending on whether the coeffs are supplied or not
    one_pole: Option<OnePoleIirState>,
    svfs: [SvfState; ORDER],
}

pub struct FilterCascadeCoeffs<const ORDER: FilterOrder> {
    pub one_pole: Option<OnePoleIirCoeff>,
    pub svfs: [SvfCoeff; ORDER],
}

impl<const ORDER: FilterOrder> Default for FilterCascadeCoeffs<ORDER> {
    fn default() -> Self {
        Self {
            one_pole: None,
            svfs: [Default::default(); ORDER],
        }
    }
}

impl<const ORDER: FilterOrder> Default for FilterCascade<ORDER> {
    fn default() -> Self {
        Self {
            one_pole: Default::default(),
            svfs: [Default::default(); ORDER],
        }
    }
}

impl<const ORDER: FilterOrder> Filter for FilterCascade<ORDER> {
    type Coeffs = FilterCascadeCoeffs<ORDER>;

    #[inline(always)]
    fn reset(&mut self) {
        if let Some(first_order) = &mut self.one_pole {
            first_order.reset();
        }
        for biquad in self.svfs.iter_mut() {
            biquad.reset();
        }
    }

    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        // Unwrapping coeffs.first_order_coeffs is okay because it is the caller's responsibility
        // to ensure that FirstOrderCoeffs are available if the filter needs them
        let y1 = self
            .one_pole
            .map(|mut first_order| first_order.process(x, &coeffs.one_pole.unwrap()))
            .unwrap_or(x);
        self.svfs.process(y1, &coeffs.svfs)
    }

    #[inline(always)]
    fn is_silent(&self, eps: f32) -> bool {
        self.one_pole
            .is_none_or(|first_order| first_order.is_silent(eps))
            && self.svfs.is_silent(eps)
    }
}

/// A cascade of up to `N` biquads + an optional first order filter
/// Supports redesigning of filters with different steepness up to `N` but also uses space for `N` filters regardless of current design
#[derive(Clone, Copy)]
pub struct FilterCascadeUpTo<const ORDER: FilterOrder> {
    pub one_pole: Option<OnePoleIirState>,
    pub svfs: [SvfState; ORDER],
    pub num_svfs: usize,
}

impl<const ORDER: FilterOrder> Default for FilterCascadeUpTo<ORDER> {
    fn default() -> Self {
        Self {
            one_pole: if ORDER % 2 == 0 {
                None
            } else {
                Some(Default::default())
            },
            num_svfs: Default::default(),
            svfs: [Default::default(); ORDER],
        }
    }
}

impl<const ORDER: FilterOrder> Filter for FilterCascadeUpTo<ORDER> {
    type Coeffs = FilterCascadeCoeffs<ORDER>;

    #[inline(always)]
    fn reset(&mut self) {
        if let Some(one_pole) = &mut self.one_pole {
            one_pole.reset();
        }
        for biquad in self.svfs.iter_mut() {
            biquad.reset();
        }
    }

    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32 {
        // Unwrapping coeffs.one_pole is okay because it is the caller's responsibility
        // to ensure that OnePoleIirCoeffs are available if the filter needs them
        let y1 = self
            .one_pole
            .map(|mut one_pole| one_pole.process(x, &coeffs.one_pole.unwrap()))
            .unwrap_or(x);
        self.svfs
            .iter_mut()
            .zip(coeffs.svfs.iter())
            .take(self.num_svfs)
            .fold(y1, |acc, (biquad, coeffs)| biquad.process(acc, coeffs))
    }

    #[inline(always)]
    fn is_silent(&self, eps: f32) -> bool {
        self.one_pole
            .is_none_or(|first_order| first_order.is_silent(eps))
            && self
                .svfs
                .iter()
                .take(self.num_svfs)
                .all(|biquad| biquad.is_silent(eps))
    }
}
