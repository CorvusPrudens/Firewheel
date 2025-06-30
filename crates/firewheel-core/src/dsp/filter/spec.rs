use crate::diff::{Diff, Patch};

/// Specifies what kind of filter to design.
#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
pub enum FilterSpec {
    /// Lowpass filter type, removing high frequencies, which makes the sound duller/muffled.
    Lowpass {
        /// Determines how aggressive the filter is. The steepness is equal to `(6 * order) dB/oct`. Typical values are `DB_OCT12`, `DB_OCT18` or `DB_OCT24` but orders up to `DB_OCT96` are supported. Higher orders will panic!
        ///
        /// When changing the order, make sure it is <= the filter node's `MAX_ORDER` const generic parameter. If you don't adhere to this, it will panic.
        order: FilterOrder,
        /// The cutoff frequency.
        ///
        /// If the `q` is set to `1.0` then this will be the point where the gain is at -3 dB. Frequencies above `cutoff_hz` will be attenuated (made quieter); the higher the frequency the quieter it becomes.
        cutoff_hz: f32,
        /// The q factor, defining the behaviour around `cutoff_hz`.
        ///
        /// A `q` of `1.0` makes it a Butterworth filter with no resonant peak. Lower values make the rolloff even more gentle while higher values result in a resonant peak around the cutoff frequency.
        ///
        /// Additionally, the width of the resonant peak depends on `order`. Counter-intuitively, higher orders produce wider resonant peaks.
        ///
        /// Finally, if you choose `order = DB_OCT6`, `q` does not produce any resonant peak.
        q: f32,
    },
    /// Highpass filter type, removing low frequencies, which can remove bass, rumble or boxiness. Depending on the sound it can also make it sound clearer, because the low frequencies don't take up as much attention anymore.
    Highpass {
        /// Determines how aggressive the filter is. The steepness is equal to `(6 * order) dB/oct`. Typical values are `DB_OCT12`, `DB_OCT18` or `DB_OCT24` but orders up to `DB_OCT96` are supported. Higher orders will panic!
        ///
        /// When changing the order, make sure it is <= the filter node's `MAX_ORDER` const generic parameter. If you don't adhere to this, it will panic.
        order: FilterOrder,
        /// The cutoff frequency.
        ///
        /// If the `q` is set to `1.0` then this will be the point where the gain is at -3 dB. Frequencies below `cutoff_hz` will be attenuated (made quieter); the lower the frequency the quieter it becomes.
        cutoff_hz: f32,
        /// The q factor, defining the behaviour around `cutoff_hz`.
        ///
        /// A `q` of `1.0` makes it a Butterworth filter with no resonant peak. Lower values make the rolloff even more gentle while higher values result in a resonant peak around the cutoff frequency.
        ///
        /// Additionally, the width of the resonant peak depends on `order`. Counter-intuitively, higher orders produce wider resonant peaks.
        ///
        /// Finally, if you choose `order = DB_OCT6`, `q` does not produce any resonant peak.
        q: f32,
    },
    /// Bandpass filter type (combination of lowpass and highpass), which can make a telephone effect, if placed around `1.2 kHz` It is fixed at second order, resulting in a steepness of about ~5 db/oct. This depends a bit on the chosen `q` factor and frequency range one considers.
    Bandpass {
        /// The center frequency of the bandpass filter, i.e. the frequency that will be attenuated the least.
        cutoff_hz: f32,
        /// The q factor, defining the shape around `cutoff_hz`.
        ///
        /// A `q` of `1.0` makes the cutoff frequency have a gain of 0 dB. Higher values will make the filter more aggressive, boosting the cutoff frequency and sharpening the rolloff around it, while lower values will make the filter more gentle, attenuating the cutoff frequency and softening the rollloff around it.
        q: f32,
    },
    /// Does not change the frequency response at all and only changes the phase with a 180° phase shift around the cuttoff frequency.
    Allpass {
        /// The frequency at which the phase will shift 180°.
        cutoff_hz: f32,
        /// The q factor, defining the shape of the phase shift. Lower values will make the slope more gentle while higher values will make the slope more aggressive.
        q: f32,
    },
    /// Bell filter type. Boosts/cuts frequencies in a bell shape.
    Bell {
        /// The maximum gain, occurring at `center_hz`.
        gain_db: f32,
        /// The frequency which experiences exactly a volume change of `gain_db`.
        center_hz: f32,
        /// Determines how wide/narrow a bell is. If we choose a gain of 10dB, then the gain at half/double the center frequency will be...
        ///
        ///             q =  0.5 --> 6.26 dB
        ///             q =  1.0 --> 3.21 dB
        ///             q =  2.0 --> 1.15 dB
        ///             q =  4.0 --> 0.33 dB
        ///             q = 10.0 --> 0.05 dB
        q: f32,
    },
    /// Low shelf filter type. Boosts/cuts all frequencies below the cutoff frequency. The gain of frequencies near the cutoff frequency is determined by the q factor.
    LowShelf {
        /// The gain at 0 Hz. See the description of `q` for more details.
        gain_db: f32,
        /// Frequency where gain is equal to `gain_db / 2`. The gain will increase below `cutoff_hz` and decrease above it. The exact behaviour depends on the value of `q`.
        cutoff_hz: f32,
        /// Determines how gentle the gain goes from 0 dB at nyquist (`sample_rate / 2`) to `gain_db` at 0 Hz. A q of `FRAC_1_SQRT_2` gives no resonant peak. Smaller values will make the slope even more gentle, while larger values will give a resonant peak. This resonant peak overshoots the value below the cutoff frequency and undershoots 0 dB above the cutoff frequency by the same amount. The larger the `q` factor the higher the overshoot/undershoot and the closer the resonant peaks will be to the cutoff frequency.
        q: f32,
    },
    /// High shelf filter type. Boosts/cuts all frequencies below the cutoff frequency. The gain of frequencies near the cutoff frequency is determined by the q factor.
    HighShelf {
        /// The gain at `sample_rate / 2` (nyquist). See the description of `q` for more details.
        gain_db: f32,
        /// Frequency where gain is equal to `gain_db / 2`. The gain will increase above `cutoff_hz` and decrease below it. The exact behaviour depends on the value of `q`.
        cutoff_hz: f32,
        /// Determines how gentle the gain goes from 0 dB at 0 Hz to `gain_db` at nyquist (`sample_rate / 2`). A q of `FRAC_1_SQRT_2` gives no resonant peak. Smaller values will make the slope even more gentle, while larger values will give a resonant peak. This resonant peak overshoots the value above the cutoff frequency and undershoots 0 dB below the cutoff frequency by the same amount. The larger the `q` factor the higher the overshoot/undershoot and the closer the resonant peaks will be to the cutoff frequency.
        q: f32,
    },
    /// Removes a single frequency entirely and attenuates nearby frequencies a bit.
    Notch {
        /// The center frequency which will removed entirely from the input signal.
        center_hz: f32,
        /// The q factor.
        ///
        /// Higher values make the notch filter more selective/aggressive, making it remove less and less of the frequencies around the cutoff frequency. A `q` of `10.0` will already be very selective/aggressive.
        q: f32,
    },
}

impl Default for FilterSpec {
    fn default() -> Self {
        Self::Lowpass {
            order: 2,
            cutoff_hz: 440.,
            q: 1.,
        }
    }
}

impl FilterSpec {
    pub fn get_type(&self) -> &'static str {
        match self {
            FilterSpec::Lowpass { .. } => "Lowpass",
            FilterSpec::Highpass { .. } => "Highpass",
            FilterSpec::Bandpass { .. } => "Bandpass",
            FilterSpec::Allpass { .. } => "Allpass",
            FilterSpec::Bell { .. } => "Bell",
            FilterSpec::LowShelf { .. } => "Low Shelf",
            FilterSpec::HighShelf { .. } => "High Shelf",
            FilterSpec::Notch { .. } => "Notch",
        }
    }
}

pub type FilterOrder = usize;

/// Filter order achieving a steepness of 6 dB/oct
pub const DB_OCT_6: FilterOrder = 1;
/// Filter order achieving a steepness of 12 dB/oct
pub const DB_OCT_12: FilterOrder = 2;
/// Filter order achieving a steepness of 18 dB/oct
pub const DB_OCT_18: FilterOrder = 3;
/// Filter order achieving a steepness of 24 dB/oct
pub const DB_OCT_24: FilterOrder = 4;
/// Filter order achieving a steepness of 30 dB/oct
pub const DB_OCT_30: FilterOrder = 5;
/// Filter order achieving a steepness of 36 dB/oct
pub const DB_OCT_36: FilterOrder = 6;
/// Filter order achieving a steepness of 42 dB/oct
pub const DB_OCT_42: FilterOrder = 7;
/// Filter order achieving a steepness of 48 dB/oct
pub const DB_OCT_48: FilterOrder = 8;
/// Filter order achieving a steepness of 54 dB/oct
pub const DB_OCT_54: FilterOrder = 9;
/// Filter order achieving a steepness of 60 dB/oct
pub const DB_OCT_60: FilterOrder = 10;
/// Filter order achieving a steepness of 66 dB/oct
pub const DB_OCT_66: FilterOrder = 11;
/// Filter order achieving a steepness of 72 dB/oct
pub const DB_OCT_72: FilterOrder = 12;
/// Filter order achieving a steepness of 78 dB/oct
pub const DB_OCT_78: FilterOrder = 13;
/// Filter order achieving a steepness of 84 dB/oct
pub const DB_OCT_84: FilterOrder = 14;
/// Filter order achieving a steepness of 90 dB/oct
pub const DB_OCT_90: FilterOrder = 15;
/// Filter order achieving a steepness of 96 dB/oct
pub const DB_OCT_96: FilterOrder = 16;
