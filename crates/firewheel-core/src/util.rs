//! General conversion functions and utilities.

use crate::SilenceMask;

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

/// De-interleave audio channels
pub fn deinterleave<V: AsMut<[f32]>>(
    channels: &mut [V],
    interleaved: &[f32],
    num_interleaved_channels: usize,
    calculate_silence_mask: bool,
) -> SilenceMask {
    if channels.is_empty() {
        return SilenceMask::NONE_SILENT;
    }

    if num_interleaved_channels == 0 {
        for ch in channels.iter_mut() {
            ch.as_mut().fill(0.0);
        }

        return SilenceMask::new_all_silent(channels.len());
    }

    let mut silence_mask = SilenceMask::NONE_SILENT;

    let (num_filled_channels, frames) = if num_interleaved_channels == 1 {
        // Mono, no need to deinterleave.

        let frames = interleaved.len();
        let ch = &mut channels[0].as_mut()[..frames];

        ch.copy_from_slice(interleaved);

        if calculate_silence_mask {
            if ch.iter().find(|&&s| s != 0.0).is_none() {
                silence_mask.set_channel(0, true);
            }
        }

        (1, frames)
    } else if num_interleaved_channels == 2 && channels.len() >= 2 {
        // Provide an optimized loop for stereo.

        let frames = interleaved.len() / 2;

        let (ch0, ch1) = channels.split_first_mut().unwrap();
        let ch0 = &mut ch0.as_mut()[..frames];
        let ch1 = &mut ch1[0].as_mut()[..frames];

        for (in_chunk, (ch0_s, ch1_s)) in interleaved
            .chunks_exact(2)
            .zip(ch0.iter_mut().zip(ch1.iter_mut()))
        {
            *ch0_s = in_chunk[0];
            *ch1_s = in_chunk[1];
        }

        if calculate_silence_mask {
            for (ch_i, ch) in channels.iter_mut().enumerate() {
                if ch.as_mut()[0..frames].iter().find(|&&s| s != 0.0).is_none() {
                    silence_mask.set_channel(ch_i, true);
                }
            }
        }

        (2, frames)
    } else {
        let mut num_filled_channels = 0;
        let frames = interleaved.len() / num_interleaved_channels;

        for (ch_i, ch) in (0..num_interleaved_channels).zip(channels.iter_mut()) {
            let ch = &mut ch.as_mut()[..frames];

            for (in_chunk, out_s) in interleaved
                .chunks_exact(num_interleaved_channels)
                .zip(ch.iter_mut())
            {
                *out_s = in_chunk[ch_i];
            }

            if calculate_silence_mask && ch_i < 64 {
                if ch.iter().find(|&&s| s != 0.0).is_none() {
                    silence_mask.set_channel(ch_i, true);
                }
            }

            num_filled_channels += 1;
        }

        (num_filled_channels, frames)
    };

    if num_filled_channels < channels.len() {
        for (ch_i, ch) in channels.iter_mut().enumerate().skip(num_filled_channels) {
            ch.as_mut()[..frames].fill(0.0);

            if calculate_silence_mask && ch_i < 64 {
                silence_mask.set_channel(ch_i, true);
            }
        }
    }

    silence_mask
}

/// Interleave audio channels
pub fn interleave<V: AsRef<[f32]>>(
    channels: &[V],
    interleaved: &mut [f32],
    num_interleaved_channels: usize,
    silence_mask: Option<SilenceMask>,
) {
    if channels.is_empty() || num_interleaved_channels == 0 {
        interleaved.fill(0.0);
        return;
    }

    if let Some(silence_mask) = silence_mask {
        if channels.len() <= 64 {
            if silence_mask.all_channels_silent(channels.len()) {
                interleaved.fill(0.0);
                return;
            }
        }
    }

    if num_interleaved_channels == 1 {
        // Mono, no need to interleave.
        interleaved.copy_from_slice(&channels[0].as_ref()[..interleaved.len()]);
        return;
    }

    if num_interleaved_channels == 2 && channels.len() >= 2 {
        // Provide an optimized loop for stereo.
        let frames = interleaved.len() / 2;

        let ch1 = &channels[0].as_ref()[..frames];
        let ch2 = &channels[1].as_ref()[..frames];

        for (out_chunk, (&ch1_s, &ch2_s)) in interleaved
            .chunks_exact_mut(2)
            .zip(ch1.iter().zip(ch2.iter()))
        {
            out_chunk[0] = ch1_s;
            out_chunk[1] = ch2_s;
        }

        return;
    }

    let any_channel_silent = if let Some(silence_mask) = silence_mask {
        if channels.len() <= 64 {
            silence_mask.any_channel_silent(channels.len())
        } else {
            true
        }
    } else {
        false
    };

    if num_interleaved_channels > channels.len() || any_channel_silent {
        interleaved.fill(0.0);
    }

    for (ch_i, ch) in channels.iter().enumerate() {
        if let Some(silence_mask) = silence_mask {
            if ch_i < 64 {
                if silence_mask.is_channel_silent(ch_i) {
                    continue;
                }
            }
        }

        for (out_chunk, &in_s) in interleaved
            .chunks_exact_mut(num_interleaved_channels)
            .zip(ch.as_ref().iter())
        {
            out_chunk[ch_i] = in_s;
        }
    }
}
