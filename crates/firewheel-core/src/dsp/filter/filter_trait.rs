/// A trait defining all functions a generic filter needs to support
pub trait Filter {
    /// The type of coefficients needed for the filter to process samples
    type Coeffs;

    /// Resets the filter memory
    fn reset(&mut self);

    /// Processes a single sample (should be forced to be inlined)
    fn process(&mut self, x: f32, coeffs: &Self::Coeffs) -> f32;

    /// Checks whether the filter is silent, i.e. whether all the memory is <= eps
    fn is_silent(&self, eps: f32) -> bool;
}
