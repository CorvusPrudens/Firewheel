mod piecewise_transport;
mod static_transport;

use bevy_platform::sync::Arc;
use core::{fmt::Debug, num::NonZeroU32, ops::Range};

pub use piecewise_transport::{PiecewiseTransport, PiecewiseTransportKeyframe};
pub use static_transport::StaticTransport;

use crate::{
    clock::{DurationSeconds, InstantMusical, InstantSamples, InstantSeconds},
    collector::ArcGc,
    diff::Notify,
};

/// A trait describing a musical transport, which is essentially a map between
/// time in seconds and time in musical beats (and vice versa).
pub trait MusicalTransport: Debug + Send + Sync + 'static {
    /// Convert the time in musical beats to the corresponding time in seconds.
    fn musical_to_seconds(
        &self,
        musical: InstantMusical,
        transport_start: InstantSeconds,
    ) -> InstantSeconds;

    /// Convert the time in musical beats to the corresponding time in samples.
    fn musical_to_samples(
        &self,
        musical: InstantMusical,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
    ) -> InstantSamples;

    /// Convert the time in seconds to the corresponding time in musical beats.
    fn seconds_to_musical(
        &self,
        seconds: InstantSeconds,
        transport_start: InstantSeconds,
    ) -> InstantMusical;

    /// Convert the time in samples to the corresponding time in musical beats.
    fn samples_to_musical(
        &self,
        sample_time: InstantSamples,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
        sample_rate_recip: f64,
    ) -> InstantMusical;

    /// Return the musical time that occurs `delta_seconds` seconds after the
    /// given `from` timestamp.
    fn delta_seconds_from(
        &self,
        from: InstantMusical,
        delta_seconds: DurationSeconds,
    ) -> InstantMusical;

    /// Return the tempo in beats per minute at the given musical time.
    fn bpm_at_musical(&self, musical: InstantMusical) -> f64;

    /// Return information about this transport for this processing block.
    fn proc_transport_info(
        &self,
        frames: usize,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> ProcTransportInfo;

    /// Return the instant of time of the beginning of this transport
    /// (musical time of `0`).
    fn transport_start(
        &self,
        now: InstantSamples,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> InstantSamples;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcTransportInfo {
    /// The number of frames in this processing block that this information
    /// lasts for before either the information changes, or the end of the
    /// processing block is reached (whichever comes first).
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
#[derive(Debug)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct TransportState {
    /// The current musical transport.
    pub transport: Option<ArcGc<dyn MusicalTransport>>,

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

impl TransportState {
    pub fn set_transport<T: MusicalTransport>(&mut self, transport: Option<T>) {
        self.transport =
            transport.map(|t| ArcGc::new_unsized(|| Arc::new(t) as Arc<dyn MusicalTransport>))
    }

    /// Set the transport to a single static tempo ([`StaticTransport`]).
    ///
    /// If `beats_per_minute` is `None`, then this will set the transport to `None`.
    pub fn set_static_transport(&mut self, beats_per_minute: Option<f64>) {
        self.set_transport(beats_per_minute.map(|bpm| StaticTransport::new(bpm)));
    }
}

impl Clone for TransportState {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.as_ref().map(|t| ArcGc::clone(t)),
            playing: self.playing.clone(),
            playhead: self.playhead.clone(),
            stop_at: self.stop_at.clone(),
            loop_range: self.loop_range.clone(),
        }
    }
}

impl PartialEq for TransportState {
    fn eq(&self, other: &Self) -> bool {
        let transports_are_equal = self
            .transport
            .as_ref()
            .map(|t1| {
                other
                    .transport
                    .as_ref()
                    .map(|t2| ArcGc::ptr_eq(t1, t2))
                    .unwrap_or(false)
            })
            .unwrap_or(other.transport.is_none());

        transports_are_equal
            && self.playing.eq(&other.playing)
            && self.playhead.eq(&other.playhead)
            && self.stop_at.eq(&other.stop_at)
            && self.loop_range.eq(&other.loop_range)
    }
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
