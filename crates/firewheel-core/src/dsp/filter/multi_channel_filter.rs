use crate::dsp::filter::{
    cascade::FilterCascadeUpTo,
    filter_trait::Filter,
    primitives::{
        spec::{DbOct12, DbOct24, DbOct36, DbOct48},
        svf::SvfCoeff,
    },
};

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

impl<const NUM_CHANNELS: usize> MultiChannelFilter<NUM_CHANNELS, FilterCascadeUpTo<8>> {
    pub fn lowpass_ord2(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::lowpass::<DbOct12>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn lowpass_ord4(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::lowpass::<DbOct24>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 2;
        }
    }

    pub fn lowpass_ord6(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::lowpass::<DbOct36>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 3;
        }
    }

    pub fn lowpass_ord8(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::lowpass::<DbOct48>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
    }

    pub fn highpass_ord2(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::highpass::<DbOct12>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn highpass_ord4(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::highpass::<DbOct24>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 2;
        }
    }

    pub fn highpass_ord6(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::highpass::<DbOct36>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 3;
        }
    }

    pub fn highpass_ord8(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::highpass::<DbOct48>(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 4;
        }
    }

    pub fn notch(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::notch(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn bell(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        SvfCoeff::bell(
            cutoff_hz,
            q,
            gain_db,
            self.sample_rate_recip,
            &mut self.coeffs.svfs,
        );
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn low_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        SvfCoeff::low_shelf(
            cutoff_hz,
            q,
            gain_db,
            self.sample_rate_recip,
            &mut self.coeffs.svfs,
        );
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn high_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        SvfCoeff::high_shelf(
            cutoff_hz,
            q,
            gain_db,
            self.sample_rate_recip,
            &mut self.coeffs.svfs,
        );
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn allpass(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::allpass(cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs.svfs);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }
}
