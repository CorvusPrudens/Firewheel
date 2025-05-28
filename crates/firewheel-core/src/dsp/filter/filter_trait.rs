pub trait Filter {
    type Coeff;

    fn reset(&mut self);
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32;
}
