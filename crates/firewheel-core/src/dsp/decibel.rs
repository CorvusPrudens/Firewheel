/// Returns the raw linear gain from the given decibel value.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(0.05 * db)
}

/// Returns the decibel value from the raw linear gain.
#[inline]
pub fn gain_to_db(amp: f32) -> f32 {
    20.0 * amp.log10()
}

/// Returns the raw linear gain from the given decibel value.
///
/// If `db <= -100.0`, then 0.0 will be returned instead (negative infinity gain).
#[inline]
pub fn db_to_gain_clamped_neg_100_db(db: f32) -> f32 {
    if db <= -100.0 {
        0.0
    } else {
        db_to_gain(db)
    }
}

/// Returns the decibel value from the raw linear gain value.
///
/// If `amp <= 0.00001`, then the minimum of `-100.0` dB will be
/// returned instead (representing negative infinity gain when paired with
/// [`db_to_gain_clamped_neg_100_db`]).
#[inline]
pub fn gain_to_db_clamped_neg_100_db(amp: f32) -> f32 {
    if amp <= 0.00001 {
        -100.0
    } else {
        gain_to_db(amp)
    }
}

/// Map a normalized value (where `0.0` means mute and `1.0` means unity
/// gain) to the corresponding raw gain value (not decibels) for use in
/// DSP. Values above `1.0` are allowed.
#[inline]
pub fn normalized_volume_to_raw_gain(normalized_volume: f32) -> f32 {
    let n = normalized_volume.max(0.0);
    n * n
}

/// A struct that converts a value in decibels to a normalized range used in
/// meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DbMeterNormalizer {
    min_db: f32,
    range_recip: f32,
    factor: f32,
}

impl DbMeterNormalizer {
    /// * `min_db` - The minimum decibel value shown in the meter.
    /// * `max_db` - The maximum decibel value shown in the meter.
    /// * `center_db` - The decibel value that will appear halfway (0.5) in the
    /// normalized range. For example, if you had `min_db` as `-100.0` and
    /// `max_db` as `0.0`, then a good `center_db` value would be `-22`.
    pub fn new(min_db: f32, max_db: f32, center_db: f32) -> Self {
        assert!(max_db > min_db);
        assert!(center_db > min_db && center_db < max_db);

        let range_recip = (max_db - min_db).recip();
        let center_normalized = ((center_db - min_db) * range_recip).clamp(0.0, 1.0);

        Self {
            min_db,
            range_recip,
            factor: 0.5_f32.log(center_normalized),
        }
    }

    #[inline]
    pub fn normalize(&self, db: f32) -> f32 {
        ((db - self.min_db) * self.range_recip)
            .clamp(0.0, 1.0)
            .powf(self.factor)
    }
}

impl Default for DbMeterNormalizer {
    fn default() -> Self {
        Self::new(-100.0, 0.0, -22.0)
    }
}
