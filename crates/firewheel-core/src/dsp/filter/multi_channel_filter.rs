use crate::dsp::filter::{cascade::FilterCascadeUpTo, primitives::svf::SvfCoeff};

use super::filter_trait::Filter;

/// A collection of `NUM_CHANNELS` filters `F` that share coefficients.
/// Use the constants `DB_OCT_*` in `spec.rs` to choose your order based on desired steepness.
pub struct MultiChannelFilter<const NUM_CHANNELS: usize, F: Filter> {
    pub filters: [F; NUM_CHANNELS],
    pub coeffs: <F as Filter>::Coeffs,
    pub sample_rate_recip: f32,
}

impl<const NUM_CHANNELS: usize, F> Default for MultiChannelFilter<NUM_CHANNELS, F>
where
    F: Filter + Default + Copy,
    F::Coeffs: Default,
{
    fn default() -> Self {
        Self {
            filters: [Default::default(); NUM_CHANNELS],
            coeffs: Default::default(),
            sample_rate_recip: 1. / 44100.,
        }
    }
}

impl<const NUM_CHANNELS: usize, F: Filter> MultiChannelFilter<NUM_CHANNELS, F> {
    #[inline(always)]
    pub fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32, channel_index: usize) -> f32 {
        // TODO: need to assert that channel_index <= NUM_CHANNELS?
        self.filters[channel_index].process(x, &self.coeffs)
    }

    #[inline(always)]
    pub fn is_silent(&self, eps: f32) -> bool {
        self.filters.iter().all(|filter| filter.is_silent(eps))
    }
}

impl<const NUM_CHANNELS: usize> MultiChannelFilter<NUM_CHANNELS, FilterCascadeUpTo<8>> {
    pub fn lowpass_ord2(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs.svfs[0] = SvfCoeff::lowpass_ord2(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn lowpass_ord4(&mut self, cutoff_hz: f32, q: f32) {
        for (svf, coeff) in self
            .coeffs
            .svfs
            .iter_mut()
            .zip(SvfCoeff::lowpass_ord4(cutoff_hz, q, self.sample_rate_recip).into_iter())
        {
            *svf = coeff;
        }
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 2;
        }
    }

    pub fn lowpass_ord6(&mut self, cutoff_hz: f32, q: f32) {
        for (svf, coeff) in self
            .coeffs
            .svfs
            .iter_mut()
            .zip(SvfCoeff::lowpass_ord6(cutoff_hz, q, self.sample_rate_recip).into_iter())
        {
            *svf = coeff;
        }
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 3;
        }
    }

    pub fn lowpass_ord8(&mut self, cutoff_hz: f32, q: f32) {
        for (svf, coeff) in self
            .coeffs
            .svfs
            .iter_mut()
            .zip(SvfCoeff::lowpass_ord8(cutoff_hz, q, self.sample_rate_recip).into_iter())
        {
            *svf = coeff;
        }
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 4;
        }
    }

    pub fn highpass_ord2(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs.svfs[0] = SvfCoeff::highpass_ord2(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn highpass_ord4(&mut self, cutoff_hz: f32, q: f32) {
        for (svf, coeff) in self
            .coeffs
            .svfs
            .iter_mut()
            .zip(SvfCoeff::highpass_ord4(cutoff_hz, q, self.sample_rate_recip).into_iter())
        {
            *svf = coeff;
        }
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 2;
        }
    }

    pub fn highpass_ord6(&mut self, cutoff_hz: f32, q: f32) {
        for (svf, coeff) in self
            .coeffs
            .svfs
            .iter_mut()
            .zip(SvfCoeff::highpass_ord6(cutoff_hz, q, self.sample_rate_recip).into_iter())
        {
            *svf = coeff;
        }
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 3;
        }
    }

    pub fn highpass_ord8(&mut self, cutoff_hz: f32, q: f32) {
        for (svf, coeff) in self
            .coeffs
            .svfs
            .iter_mut()
            .zip(SvfCoeff::highpass_ord8(cutoff_hz, q, self.sample_rate_recip).into_iter())
        {
            *svf = coeff;
        }
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 4;
        }
    }

    pub fn notch(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs.svfs[0] = SvfCoeff::notch(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn bell(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        self.coeffs.svfs[0] = SvfCoeff::bell(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn low_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        self.coeffs.svfs[0] = SvfCoeff::low_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn high_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        self.coeffs.svfs[0] = SvfCoeff::high_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn allpass(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs.svfs[0] = SvfCoeff::allpass(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }
}
