mod piecewise_transport;
mod static_transport;

use core::{num::NonZeroU32, ops::Range};

pub use piecewise_transport::{PiecewiseTransport, PiecewiseTransportKeyframe};
pub use static_transport::StaticTransport;

use crate::{
    clock::{DurationSeconds, InstantMusical, InstantSamples, InstantSeconds},
    collector::ArcGc,
    diff::Notify,
};

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum MusicalTransport {
    /// A musical transport with a single static tempo in beats per minute.
    Static(StaticTransport),
    /// A musical transport with multiple keyframes of tempo. The tempo
    /// immediately jumps from one keyframe to another (the tempo is *NOT*
    /// linearly interpolated between keyframes).
    Piecewise(ArcGc<PiecewiseTransport>),
    // TODO: Linearly automated tempo.
}

impl Default for MusicalTransport {
    fn default() -> Self {
        Self::Static(StaticTransport::default())
    }
}

impl MusicalTransport {
    pub fn musical_to_seconds(
        &self,
        musical: InstantMusical,
        transport_start: InstantSeconds,
    ) -> InstantSeconds {
        match self {
            MusicalTransport::Static(s) => s.musical_to_seconds(musical, transport_start),
            MusicalTransport::Piecewise(s) => s.musical_to_seconds(musical, transport_start),
        }
    }

    pub fn musical_to_samples(
        &self,
        musical: InstantMusical,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        match self {
            MusicalTransport::Static(s) => {
                s.musical_to_samples(musical, transport_start, sample_rate)
            }
            MusicalTransport::Piecewise(s) => {
                s.musical_to_samples(musical, transport_start, sample_rate)
            }
        }
    }

    pub fn samples_to_musical(
        &self,
        sample_time: InstantSamples,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
        sample_rate_recip: f64,
    ) -> InstantMusical {
        match self {
            MusicalTransport::Static(s) => {
                s.samples_to_musical(sample_time, transport_start, sample_rate, sample_rate_recip)
            }
            MusicalTransport::Piecewise(s) => {
                s.samples_to_musical(sample_time, transport_start, sample_rate, sample_rate_recip)
            }
        }
    }

    pub fn seconds_to_musical(
        &self,
        seconds: InstantSeconds,
        transport_start: InstantSeconds,
    ) -> InstantMusical {
        match self {
            MusicalTransport::Static(s) => s.seconds_to_musical(seconds, transport_start),
            MusicalTransport::Piecewise(s) => s.seconds_to_musical(seconds, transport_start),
        }
    }

    /// Return the musical time that occurs `delta_seconds` seconds after the
    /// given `from` timestamp.
    pub fn delta_seconds_from(
        &self,
        from: InstantMusical,
        delta_seconds: DurationSeconds,
    ) -> InstantMusical {
        match self {
            MusicalTransport::Static(s) => s.delta_seconds_from(from, delta_seconds),
            MusicalTransport::Piecewise(s) => s.delta_seconds_from(from, delta_seconds),
        }
    }

    /// Return the tempo in beats per minute at the given musical time.
    pub fn bpm_at_musical(&self, musical: InstantMusical) -> f64 {
        match self {
            MusicalTransport::Static(s) => s.beats_per_minute(),
            MusicalTransport::Piecewise(s) => s.bpm_at_musical(musical),
        }
    }

    pub fn proc_transport_info(
        &self,
        frames: usize,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> ProcTransportInfo {
        match self {
            MusicalTransport::Static(s) => s.proc_transport_info(frames),
            MusicalTransport::Piecewise(s) => s.proc_transport_info(frames, playhead, sample_rate),
        }
    }

    pub fn transport_start(
        &self,
        now: InstantSamples,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        match self {
            MusicalTransport::Static(s) => s.transport_start(now, playhead, sample_rate),
            MusicalTransport::Piecewise(s) => s.transport_start(now, playhead, sample_rate),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcTransportInfo {
    /// The number of frames in this processing block that this information
    /// lasts for before it changes.
    pub frames: usize,

    /// The beats per minute at the first frame of this process block.
    pub beats_per_minute: f64,

    /// The rate at which `beats_per_minute` changes each frame in this
    /// processing block.
    ///
    /// For example, if this value is `0.0`, then the bpm remains static for
    /// the entire duration of this processing block.
    ///
    /// And for example, if this is `0.1`, then the bpm increases by `0.1`
    /// each frame, and if this is `-0.1`, then the bpm decreased by `0.1`
    /// each frame.
    pub delta_beats_per_minute: f64,
}

impl ProcTransportInfo {
    /// Get the BPM at the given frame.
    ///
    /// Returns `None` if `frame >= self.frames`.
    pub fn bpm_at_frame(&self, frame: usize) -> Option<f64> {
        (frame < self.frames)
            .then(|| self.beats_per_minute + (self.delta_beats_per_minute * frame as f64))
    }
}

/// The state of the musical transport in a Firewheel context.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct TransportState {
    /// The current musical transport.
    pub transport: Option<MusicalTransport>,

    /// Whether or not the musical transport is playing (true) or is paused (false).
    pub playing: Notify<bool>,

    /// The playhead of the musical transport.
    pub playhead: Notify<InstantMusical>,

    /// If this is `Some`, then the transport will automatically stop when the playhead
    /// reaches the given musical time.
    ///
    /// This has no effect if [`TransportState::loop_range`] is `Some`.
    pub stop_at: Option<InstantMusical>,

    /// If this is `Some`, then the transport will continously loop the given region.
    pub loop_range: Option<Range<InstantMusical>>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            transport: None,
            playing: Notify::new(false),
            playhead: Notify::new(InstantMusical::ZERO),
            stop_at: None,
            loop_range: None,
        }
    }
}

#[inline]
pub fn seconds_per_beat(beats_per_minute: f64) -> f64 {
    60.0 / beats_per_minute
}

#[inline]
pub fn beats_per_second(beats_per_minute: f64) -> f64 {
    beats_per_minute * (1.0 / 60.0)
}
