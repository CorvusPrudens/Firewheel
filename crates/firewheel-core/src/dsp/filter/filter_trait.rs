use std::num::NonZero;

use super::spec::{ResponseType, SimpleResponseType, DB_OCT_12, DB_OCT_24, DB_OCT_6};

pub trait Filter {
    /// The type of coefficients needed for the Filter to process samples
    type Coeffs;

    /// Resets the filter memory
    fn reset(&mut self);

    /// Processes a single sample
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32;

    /// Checks whether the filter is silent, i.e. whether all the memory is <= eps
    fn is_silent(&self, eps: f32) -> bool;
}

/// A collection of `NUM_CHANNELS` filters `F` that share coefficients.
/// Use the constants `DB_OCT_*` in `spec.rs` to choose your order based on desired steepness.
pub struct FilterBank<const NUM_CHANNELS: usize, F: Filter> {
    pub filters: [F; NUM_CHANNELS],
    pub coeffs: <F as Filter>::Coeffs,
    pub response_type: ResponseType,
    pub cutoff_hz: f32,
    pub sample_rate: NonZero<u32>,
    pub order: usize,
}

impl<const NUM_CHANNELS: usize, F> Default for FilterBank<NUM_CHANNELS, F>
where
    F: Filter + Default + Copy,
    F::Coeffs: Default,
{
    fn default() -> Self {
        Self {
            filters: [Default::default(); NUM_CHANNELS],
            coeffs: Default::default(),
            response_type: ResponseType::Simple(SimpleResponseType::Lowpass),
            cutoff_hz: Default::default(),
            sample_rate: NonZero::new(44100).unwrap(),
            order: DB_OCT_12,
        }
    }
}

impl<const NUM_CHANNELS: usize, F: Filter> FilterBank<NUM_CHANNELS, F> {
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

    pub fn is_silent(&self, eps: f32) -> bool {
        self.filters.iter().all(|filter| filter.is_silent(eps))
    }
}
