use crate::clock::{
    DurationMusical, DurationSeconds, InstantMusical, InstantSamples, InstantSeconds,
    ProcTransportInfo,
};
use core::num::NonZeroU32;

/// A musical transport with a single static tempo in beats per minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticTransport {
    pub beats_per_minute: f64,
}

impl Default for StaticTransport {
    fn default() -> Self {
        Self {
            beats_per_minute: 110.0,
        }
    }
}

impl StaticTransport {
    pub const fn new(beats_per_minute: f64) -> Self {
        Self { beats_per_minute }
    }

    pub const fn beats_per_minute(&self) -> f64 {
        self.beats_per_minute
    }

    pub fn seconds_per_beat(&self) -> f64 {
        60.0 / self.beats_per_minute
    }

    pub fn beats_per_second(&self) -> f64 {
        self.beats_per_minute * (1.0 / 60.0)
    }

    pub fn musical_to_seconds(
        &self,
        musical: InstantMusical,
        transport_start: InstantSeconds,
    ) -> InstantSeconds {
        transport_start + DurationSeconds(musical.0 * self.seconds_per_beat())
    }

    pub fn musical_to_samples(
        &self,
        musical: InstantMusical,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        transport_start
            + DurationSeconds(musical.0 * self.seconds_per_beat()).to_samples(sample_rate)
    }

    pub fn samples_to_musical(
        &self,
        sample_time: InstantSamples,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
        sample_rate_recip: f64,
    ) -> InstantMusical {
        InstantMusical(
            (sample_time - transport_start)
                .to_seconds(sample_rate, sample_rate_recip)
                .0
                * self.beats_per_second(),
        )
    }

    pub fn seconds_to_musical(
        &self,
        seconds: InstantSeconds,
        transport_start: InstantSeconds,
    ) -> InstantMusical {
        InstantMusical((seconds - transport_start).0 * self.beats_per_second())
    }

    /// Return the musical time that occurs `delta_seconds` seconds after the
    /// given `from` timestamp.
    pub fn delta_seconds_from(
        &self,
        from: InstantMusical,
        delta_seconds: DurationSeconds,
    ) -> InstantMusical {
        from + DurationMusical(delta_seconds.0 * self.beats_per_second())
    }

    pub fn transport_start(
        &self,
        now: InstantSamples,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        now - DurationSeconds(playhead.0 * self.seconds_per_beat()).to_samples(sample_rate)
    }

    pub fn proc_transport_info(&self, frames: usize) -> ProcTransportInfo {
        ProcTransportInfo {
            frames,
            beats_per_minute: self.beats_per_minute,
            delta_beats_per_minute: 0.0,
        }
    }
}
