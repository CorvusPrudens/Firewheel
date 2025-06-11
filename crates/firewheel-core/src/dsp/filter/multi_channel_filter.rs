use crate::dsp::filter::{
    cascade::FilterCascadeUpTo,
    filter_trait::Filter,
    primitives::{one_pole_iir::OnePoleIirCoeff, svf::SvfCoeff},
    spec::FilterOrder,
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
    pub fn is_silent(&self) -> bool {
        self.filters.iter().all(|filter| filter.is_silent())
    }
}

impl<const NUM_CHANNELS: usize, const MAX_ORDER: usize>
    MultiChannelFilter<NUM_CHANNELS, FilterCascadeUpTo<MAX_ORDER>>
{
    pub fn lowpass(&mut self, order: FilterOrder, cutoff_hz: f32, q: f32) {
        assert!(order <= MAX_ORDER);

        if order % 2 == 0 {
            self.coeffs.one_pole = OnePoleIirCoeff::NO_OP;
        } else {
            self.coeffs.one_pole = OnePoleIirCoeff::lowpass(cutoff_hz, self.sample_rate_recip);
        }

        SvfCoeff::lowpass(
            order,
            cutoff_hz,
            q,
            self.sample_rate_recip,
            &mut self.coeffs.svfs,
        );
        for filter in self.filters.iter_mut() {
            filter.num_svfs = order / 2;
        }
    }
    pub fn highpass(&mut self, order: FilterOrder, cutoff_hz: f32, q: f32) {
        assert!(order <= MAX_ORDER);

        SvfCoeff::highpass(
            order,
            cutoff_hz,
            q,
            self.sample_rate_recip,
            &mut self.coeffs.svfs,
        );
        for filter in self.filters.iter_mut() {
            filter.num_svfs = order;
        }
    }

    pub fn notch(&mut self, cutoff_hz: f32, q: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::notch(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn bell(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::bell(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn low_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::low_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn high_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::high_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }

    pub fn allpass(&mut self, cutoff_hz: f32, q: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::allpass(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.iter_mut() {
            filter.num_svfs = 1;
        }
    }
}
