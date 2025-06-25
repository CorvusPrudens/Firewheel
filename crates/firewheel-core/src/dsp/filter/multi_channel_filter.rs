use crate::{
    channel_config::NonZeroChannelCount,
    dsp::filter::{
        cascade::FilterCascadeUpTo,
        filter_trait::Filter,
        primitives::{
            one_pole_iir::OnePoleIirCoeff,
            svf::{SvfCoeff, SvfState},
        },
        spec::FilterOrder,
    },
};

/// A collection of filters `F` that share coefficients.
/// Use the constants `DB_OCT_*` to choose your order based on desired steepness.
pub struct MultiChannelFilter<C, F: Filter> {
    pub filters: C,
    pub coeffs: <F as Filter>::Coeffs,
    pub sample_rate_recip: f32,
    pub current_order: FilterOrder,
}

impl<F> Default for MultiChannelFilter<Vec<F>, F>
where
    F: Filter,
    F::Coeffs: Default,
{
    fn default() -> Self {
        Self {
            filters: Default::default(),
            coeffs: Default::default(),
            sample_rate_recip: 1. / 44100.,
            current_order: 1,
        }
    }
}

impl<const NUM_CHANNELS: usize, F> Default for MultiChannelFilter<[F; NUM_CHANNELS], F>
where
    F: Filter + Default + Copy,
    F::Coeffs: Default,
{
    fn default() -> Self {
        Self {
            filters: [F::default(); NUM_CHANNELS],
            coeffs: Default::default(),
            sample_rate_recip: 1. / 44100.,
            current_order: 1,
        }
    }
}

impl<C, F> MultiChannelFilter<C, F>
where
    C: AsMut<[F]> + AsRef<[F]>,
    F: Filter,
{
    #[inline(always)]
    pub fn reset(&mut self) {
        for filter in self.filters.as_mut().iter_mut() {
            filter.reset();
        }
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32, channel_index: usize) -> f32 {
        // TODO: need to assert that channel_index <= NUM_CHANNELS?
        self.filters
            .as_mut()
            .get_mut(channel_index)
            .unwrap()
            .process(x, &self.coeffs)
    }

    #[inline(always)]
    pub fn is_silent(&self) -> bool {
        self.filters
            .as_ref()
            .iter()
            .all(|filter| filter.is_silent())
    }
}

impl<C, const MAX_ORDER: usize> MultiChannelFilter<C, FilterCascadeUpTo<MAX_ORDER>>
where
    C: AsMut<[FilterCascadeUpTo<MAX_ORDER>]>,
{
    /// Resets filters if they weren't in use before but are now. This ensures stale filter memory does not poison new samples.
    /// Additionally, it stores the new_order.
    fn process_order_change(&mut self, new_order: FilterOrder) {
        if new_order % 2 == 1 && self.current_order % 2 == 0 {
            for filter in self.filters.as_mut().iter_mut() {
                filter.one_pole.reset();
            }
        }
        if new_order > self.current_order {
            for filter in self.filters.as_mut().iter_mut() {
                for svf in filter.svfs[new_order..].iter_mut() {
                    svf.reset();
                }
            }
        }
        self.current_order = new_order;
    }

    /// Designs a lowpass filter. `order <= MAX_ORDER` must be ensured by the caller, otherwise this function panics.
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
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = order / 2;
        }

        self.process_order_change(order);
    }

    /// Designs a highpass filter. `order <= MAX_ORDER` must be ensured by the caller, otherwise this function panics.
    pub fn highpass(&mut self, order: FilterOrder, cutoff_hz: f32, q: f32) {
        assert!(order <= MAX_ORDER);

        if order % 2 == 0 {
            self.coeffs.one_pole = OnePoleIirCoeff::NO_OP;
        } else {
            self.coeffs.one_pole = OnePoleIirCoeff::highpass(cutoff_hz, self.sample_rate_recip);
        }

        SvfCoeff::highpass(
            order,
            cutoff_hz,
            q,
            self.sample_rate_recip,
            &mut self.coeffs.svfs,
        );
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = order / 2;
        }

        self.process_order_change(order);
    }

    /// Designs a bandpass filter. `MAX_ORDER >= 2` must be ensured by the caller, otherwise this function panics.
    pub fn bandpass(&mut self, cutoff_hz: f32, q: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::bandpass(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = 1;
        }

        self.process_order_change(2);
    }

    /// Designs an allpass filter. `MAX_ORDER >= 2` must be ensured by the caller, otherwise this function panics.
    pub fn allpass(&mut self, cutoff_hz: f32, q: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::allpass(cutoff_hz, q, self.sample_rate_recip);
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = 1;
        }

        self.process_order_change(2);
    }

    /// Designs a notch filter. `MAX_ORDER >= 2` must be ensured by the caller, otherwise this function panics.
    pub fn notch(&mut self, center_hz: f32, q: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::notch(center_hz, q, self.sample_rate_recip);
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = 1;
        }

        self.process_order_change(2);
    }

    /// Designs a bell filter. `MAX_ORDER >= 2` must be ensured by the caller, otherwise this function panics.
    pub fn bell(&mut self, center_hz: f32, q: f32, gain_db: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::bell(center_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = 1;
        }

        self.process_order_change(2);
    }

    /// Designs a low shelf filter. `MAX_ORDER >= 2` must be ensured by the caller, otherwise this function panics.
    pub fn low_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::low_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = 1;
        }

        self.process_order_change(2);
    }

    /// Designs a high shelf filter. `MAX_ORDER >= 2` must be ensured by the caller, otherwise this function panics.
    pub fn high_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        assert!(MAX_ORDER >= 2);

        self.coeffs.svfs[0] = SvfCoeff::high_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
        for filter in self.filters.as_mut().iter_mut() {
            filter.num_svfs = 1;
        }

        self.process_order_change(2);
    }
}

/// Implementation using exactly 1 SVF for more space efficient basic filters that don't need the single pole filter
impl<C> MultiChannelFilter<C, [SvfState; 1]>
where
    C: AsMut<[SvfState; 1]>,
{
    /// Designs a lowpass filter.
    pub fn lowpass(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::lowpass(2, cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs);
    }

    /// Designs a highpass filter.
    pub fn highpass(&mut self, cutoff_hz: f32, q: f32) {
        SvfCoeff::highpass(2, cutoff_hz, q, self.sample_rate_recip, &mut self.coeffs);
    }

    /// Designs a bandpass filter.
    pub fn bandpass(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs[0] = SvfCoeff::bandpass(cutoff_hz, q, self.sample_rate_recip);
    }

    /// Designs an allpass filter.
    pub fn allpass(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs[0] = SvfCoeff::allpass(cutoff_hz, q, self.sample_rate_recip);
    }

    /// Designs a notch filter.
    pub fn notch(&mut self, cutoff_hz: f32, q: f32) {
        self.coeffs[0] = SvfCoeff::notch(cutoff_hz, q, self.sample_rate_recip);
    }

    /// Designs a bell filter.
    pub fn bell(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        self.coeffs[0] = SvfCoeff::bell(cutoff_hz, q, gain_db, self.sample_rate_recip);
    }

    /// Designs a low shelf filter.
    pub fn low_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        self.coeffs[0] = SvfCoeff::low_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
    }

    /// Designs a high shelf filter.
    pub fn high_shelf(&mut self, cutoff_hz: f32, q: f32, gain_db: f32) {
        self.coeffs[0] = SvfCoeff::high_shelf(cutoff_hz, q, gain_db, self.sample_rate_recip);
    }
}

impl<F> MultiChannelFilter<Vec<F>, F>
where
    F: Filter + Default + Clone,
    F::Coeffs: Default,
{
    pub fn with_channels(num_channels: NonZeroChannelCount) -> Self {
        Self {
            filters: vec![F::default(); num_channels.get().get() as usize],
            coeffs: Default::default(),
            sample_rate_recip: 1. / 44100.,
            current_order: 1,
        }
    }
}

pub type ArrayMultiChannelFilter<const NUM_CHANNELS: usize, F> =
    MultiChannelFilter<[F; NUM_CHANNELS], F>;
pub type VecMultiChannelFilter<F> = MultiChannelFilter<Vec<F>, F>;
