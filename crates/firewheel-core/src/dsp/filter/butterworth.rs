use std::f32;

use super::{
    cascade::{ChainedCascadeUpTo, FilterCascadeUpTo},
    spec::{CompositeResponseType, FilterOrder, ResponseType},
};

/// Returns the coefficients for the analog prototype of a Butterworth filter.
/// Since the real pole is always at -1 and the conjugate poles are distributed
/// uniformly around the unit circle, there is only one coefficient we need to store.
/// The slice returned is a list of all of these coefficients
fn get_analog_coeffs(order: FilterOrder) -> &'static [f32] {
    match order {
        1 => &[],
        2 => &[f32::consts::SQRT_2],
        3 => &[1.],
        4 => &[0.765_366_85, 1.847_759],
        5 => &[0.618_034, 1.618_034],
        6 => &[0.517_638_1, f32::consts::SQRT_2, 1.931_851_6],
        8 => &[0.390_180_65, 1.111_140_5, 1.662_939_2, 1.961_570_5],
        _ => panic!("Unsupported filter order"),
    }
}

trait Butterworth<const MAX_ORDER: FilterOrder> {
    fn design_butterworth(
        &mut self,
        response_type: ResponseType,
        frequency: f32,
        sample_rate: f32,
        new_order: FilterOrder,
    );
}

impl<const MAX_ORDER: FilterOrder> Butterworth<MAX_ORDER> for FilterCascadeUpTo<MAX_ORDER> {
    fn design_butterworth(
        &mut self,
        response_type: ResponseType,
        frequency: f32,
        sample_rate: f32,
        new_order: FilterOrder,
    ) {
        todo!()
    }
}

trait ButterworthComposite<const MAX_ORDER: FilterOrder> {
    fn design_butterworth_composite(
        &mut self,
        response_type: CompositeResponseType,
        frequency: f32,
        sample_rate: f32,
        new_order: FilterOrder,
    );
}

impl<const MAX_ORDER: FilterOrder> ButterworthComposite<MAX_ORDER>
    for ChainedCascadeUpTo<MAX_ORDER, 2>
{
    fn design_butterworth_composite(
        &mut self,
        response_type: CompositeResponseType,
        frequency: f32,
        sample_rate: f32,
        new_order: FilterOrder,
    ) {
        todo!()
    }
}
