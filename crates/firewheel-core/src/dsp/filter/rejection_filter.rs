use std::f32::consts::TAU;

use super::{
    filter_trait::Filter,
    spec::{OddOrderSteepness, Steepness},
};

// TODO: can be made more concise using generic const expressions once they are stable

pub trait Type {}

#[derive(Default)]
pub struct Lowpass<S: Steepness> {
    cascade: S::ConcreteFilter,
}
#[derive(Default)]
pub struct Highpass<S: Steepness> {
    cascade: S::ConcreteFilter,
}
#[derive(Default)]
pub struct Bandpass<S: Steepness> {
    cascades: [S::ConcreteFilter; 2],
}
#[derive(Default)]
pub struct Bandstop<S: Steepness> {
    cascades: [S::ConcreteFilter; 2],
}
impl<S: Steepness> Type for Lowpass<S> {}
impl<S: Steepness> Type for Highpass<S> {}
impl<S: Steepness> Type for Bandpass<S> {}
impl<S: Steepness> Type for Bandstop<S> {}

impl<S: Steepness> Filter for Lowpass<S> {
    type Coeff = <S::ConcreteFilter as Filter>::Coeff;

    fn reset(&mut self) {
        self.cascade.reset();
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.cascade.process(x, coeffs)
    }
}
impl<S: Steepness> Filter for Highpass<S> {
    type Coeff = <S::ConcreteFilter as Filter>::Coeff;

    fn reset(&mut self) {
        self.cascade.reset();
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        self.cascade.process(x, coeffs)
    }
}
impl<S: Steepness> Filter for Bandpass<S> {
    type Coeff = [<S::ConcreteFilter as Filter>::Coeff; 2];

    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        coeffs
            .into_iter()
            .zip(self.cascades.iter_mut())
            .fold(x, |acc, (coeff, cascade)| cascade.process(acc, coeff))
    }
}
impl<S: Steepness> Filter for Bandstop<S> {
    type Coeff = [<S::ConcreteFilter as Filter>::Coeff; 2];

    fn reset(&mut self) {
        for cascade in self.cascades.iter_mut() {
            cascade.reset();
        }
    }

    // TODO: discuss whether inlining always a good idea
    #[inline(always)]
    fn process(&mut self, x: f32, coeffs: Self::Coeff) -> f32 {
        coeffs
            .into_iter()
            .zip(self.cascades.iter_mut())
            .fold(x, |acc, (coeff, cascade)| cascade.process(acc, coeff))
    }
}
