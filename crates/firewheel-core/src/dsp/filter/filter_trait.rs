pub trait Filter {
    fn reset(&mut self);
    fn process(&mut self, x: f32) -> f32;
}
