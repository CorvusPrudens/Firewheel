use super::spec::ResponseType;

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

pub struct FilterBank<const NUM_CHANNELS: usize, F: Filter> {
    pub filters: [F; NUM_CHANNELS],
    pub coeffs: <F as Filter>::Coeffs,
    pub response_type: ResponseType,
}

impl<const NUM_CHANNELS: usize, F: Filter> FilterBank<NUM_CHANNELS, F> {
    fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
    }

    // TODO: change function once I know how this is actually called
    fn process(&mut self, xs: [f32; NUM_CHANNELS]) -> [f32; NUM_CHANNELS] {
        let mut result = [0.; NUM_CHANNELS];
        for (filter, (inp, out)) in self
            .filters
            .iter_mut()
            .zip(xs.into_iter().zip(result.iter_mut()))
        {
            *out = filter.process(inp, &self.coeffs);
        }
        result
    }

    fn is_silent(&self, eps: f32) -> bool {
        self.filters.iter().all(|filter| filter.is_silent(eps))
    }
}
