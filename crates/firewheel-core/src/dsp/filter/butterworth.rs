use std::f32;
use std::f32::consts::SQRT_2;
use std::num::NonZero;

use crate::dsp::filter::spec::ResponseType;

use super::filter_trait::FilterBank;
use super::primitives::{prewarp_k, BiquadCoeffs, FirstOrderCoeffs};

use super::spec::{
    DB_OCT_12, DB_OCT_18, DB_OCT_24, DB_OCT_36, DB_OCT_48, DB_OCT_6, DB_OCT_72, DB_OCT_96,
};
use super::{
    cascade::FilterCascadeUpTo,
    spec::{FilterOrder, SimpleResponseType},
};

/// Returns the coefficients for the analog prototype of a Butterworth filter.
/// Since the real pole is always at -1 and the conjugate poles are distributed
/// uniformly around the unit circle, there is only one coefficient we need to store.
/// The slice returned is a list of all of these coefficients
fn get_analog_coeffs(order: FilterOrder) -> &'static [f32] {
    match order {
        DB_OCT_6 => &[],
        DB_OCT_12 => &[-SQRT_2],
        DB_OCT_18 => &[-1.],
        DB_OCT_24 => &[-0.765_366_85, -1.847_759],
        DB_OCT_36 => &[-0.517_638_1, -SQRT_2, -1.931_851_6],
        DB_OCT_48 => &[-0.390_180_65, -1.111_140_5, -1.662_939_2, -1.961_570_5],
        DB_OCT_72 => &[
            -0.261_052_37,
            -0.765_366_85,
            -1.217_522_9,
            -1.586_706_6,
            -1.847_759,
            -1.982_889_8,
        ],
        DB_OCT_96 => &[
            -0.196_034_28,
            -0.580_569_3,
            -0.942_793_5,
            -1.268_786_5,
            -1.546_020_9,
            -1.763_842_6,
            -1.913_880_7,
            -1.990_369_4,
        ],
        _ => panic!("Unsupported filter order {}", order),
    }
}

pub trait Butterworth<const MAX_ORDER: FilterOrder> {
    fn design_butterworth(
        &mut self,
        response_type: SimpleResponseType,
        frequency: f32,
        sample_rate: NonZero<u32>,
        new_order: FilterOrder,
    );
}

impl<const NUM_CHANNELS: usize, const MAX_ORDER: FilterOrder> Butterworth<MAX_ORDER>
    for FilterBank<NUM_CHANNELS, FilterCascadeUpTo<MAX_ORDER>>
{
    fn design_butterworth(
        &mut self,
        response_type: SimpleResponseType,
        cutoff_hz: f32,
        sample_rate: NonZero<u32>,
        new_order: FilterOrder,
    ) {
        assert!(new_order <= MAX_ORDER);
        // TODO: what to do with smoothing and filter memory? we should definitely reset it when the response type changes
        self.sample_rate = sample_rate;
        self.cutoff_hz = cutoff_hz;
        self.order = new_order;
        self.response_type = ResponseType::Simple(response_type);

        let k = prewarp_k(cutoff_hz, sample_rate);

        // Odd-order butterworth filters have an additional real pole
        if new_order % 2 != 0 {
            self.coeffs.first_order = Some(FirstOrderCoeffs::from_real_pole(1., k));
        } else {
            self.coeffs.first_order = None;
        }

        let analog_coeffs = get_analog_coeffs(new_order);
        for (&coeff, biquad) in analog_coeffs.iter().zip(self.coeffs.biquads.iter_mut()) {
            *biquad = BiquadCoeffs::from_conjugate_pole(coeff, 1., k);
        }

        // TODO: handle highpass case
    }
}

/*
impl<const NUM_CHANNELS: usize, const MAX_ORDER: FilterOrder, const M: usize> Butterworth<MAX_ORDER>
    for FilterBank<NUM_CHANNELS, ChainedCascadeUpTo<MAX_ORDER, M>>
{
    fn design_butterworth(
        &mut self,
        response_type: SimpleResponseType,
        frequency: f32,
        sample_rate: NonZero<u32>,
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
        sample_rate: NonZero<u32>,
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
        sample_rate: NonZero<u32>,
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
 */
