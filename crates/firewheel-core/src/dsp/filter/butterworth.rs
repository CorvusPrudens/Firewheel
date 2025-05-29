use std::f32;

use crate::dsp::filter::primitives::{prewarp_k, BiquadCoeffs, FirstOrderCoeffs, FirstOrderFilter};

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
        assert!(new_order <= MAX_ORDER);
        // TODO: when should we reset filter memory? we would need to store the current response type for that which is annoying

        let k = prewarp_k(frequency, sample_rate);

        // Odd-order butterworth filters have an additional real pole
        if new_order % 2 != 0 {
            let new_coeffs = FirstOrderCoeffs::from_real_pole(1., k);
            self.first_order
                .get_or_insert_with(|| FirstOrderFilter::with_coeffs(new_coeffs))
                .coeffs = new_coeffs;
        } else {
            self.first_order = None;
        }

        let analog_coeffs = get_analog_coeffs(new_order);
        for (&coeff, biquad) in analog_coeffs.iter().zip(self.biquads.iter_mut()) {
            biquad.coeffs = BiquadCoeffs::from_conjugate_pole(coeff, 1., k);
        }
    }
}

impl<const MAX_ORDER: FilterOrder, const M: usize> Butterworth<MAX_ORDER>
    for ChainedCascadeUpTo<MAX_ORDER, M>
{
    fn design_butterworth(
        &mut self,
        response_type: ResponseType,
        frequency: f32,
        sample_rate: f32,
        new_order: FilterOrder,
    ) {
        self.cascades[0].design_butterworth(response_type, frequency, sample_rate, new_order);
    }
}

trait ButterworthComposite<const MAX_ORDER: FilterOrder> {
    fn design_butterworth_composite(
        &mut self,
        response_type: CompositeResponseType,
        frequency_range: (f32, f32),
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
        frequency_range: (f32, f32),
        sample_rate: f32,
        new_order: FilterOrder,
    ) {
        assert!(new_order <= MAX_ORDER);

        let response_types = response_type.into_response_types();

        self.cascades[0].design_butterworth(
            response_types[0],
            frequency_range.0,
            sample_rate,
            new_order,
        );
        self.cascades[1].design_butterworth(
            response_types[1],
            frequency_range.1,
            sample_rate,
            new_order,
        );
    }
}
