use std::{
    ops::{Add, AddAssign, Sub, SubAssign},
    sync::atomic::{AtomicU64, Ordering},
};

use atomic_float::AtomicF64;

/// When a particular audio event should occur.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum EventDelay {
    /// The event should happen immediately when it is recieved.
    #[default]
    Immediate,
    /// The event should happen when the real-time clock reaches the given time
    /// in seconds.
    ///
    /// The value is an absolute time, *NOT* a delta time. Use
    /// [`AudioGraph::realtime_clock_secs`] to get the current time of the
    /// realtime clock.
    DelayUntilSeconds(f64),
    /// The event should happen when the sample clock reaches the given time in
    /// samples.
    ///
    /// This is more accurate than [`EventDelay::DelayUntilSeconds`],
    /// but it does *NOT* account for any output underflows that may occur.
    /// If any underflows occur, then this will become out of sync
    /// with [`EventDelay::DelayUntilSeconds`]. Prefer to use
    /// [`EventDelay::DelayUntilSeconds`] unless you are syncing your game to
    /// the sample event clock (or you are not concerned about underflows
    /// happenning.)
    ///
    /// This value is an absolute time, *NOT* a delta time. Use
    /// [`AudioGraph::sample_clock_time`] to get the current time of the sample
    /// clock.
    DelayUntilSample(SampleTime),
}

/// Time in units of samples
#[repr(transparent)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SampleTime(pub u64);

impl SampleTime {
    pub const fn new(samples: u64) -> Self {
        Self(samples)
    }

    pub fn from_secs_f64(seconds: f64, sample_rate: u32) -> Self {
        assert!(seconds >= 0.0);

        let seconds_u64 = seconds.floor() as u64;
        let fract_samples_u64 = (seconds.fract() * f64::from(sample_rate)).round() as u64;

        Self((seconds_u64 * u64::from(sample_rate)) + fract_samples_u64)
    }

    #[inline]
    pub fn seconds(&self, sample_rate: u32) -> u32 {
        (self.0 / u64::from(sample_rate)) as u32
    }

    #[inline]
    pub fn fract_samples(&self, sample_rate: u32) -> u32 {
        (self.0 % u64::from(sample_rate)) as u32
    }

    pub fn as_secs_f64(&self, sample_rate: u32, sample_rate_recip: f64) -> f64 {
        let seconds = self.seconds(sample_rate);
        let fract_samples = self.fract_samples(sample_rate);

        f64::from(seconds) + (f64::from(fract_samples) * sample_rate_recip)
    }

    pub fn add_secs_f64(self, seconds: f64, sample_rate: u32) -> Self {
        assert!(seconds >= 0.0);

        let seconds_u64 = seconds.floor() as u64;
        let fract_samples_u64 = (seconds.fract() * f64::from(sample_rate)).round() as u64;

        Self(self.0 + (seconds_u64 * u64::from(sample_rate)) + fract_samples_u64)
    }
}

impl Add for SampleTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for SampleTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign for SampleTime {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for SampleTime {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

#[repr(transparent)]
#[derive(Debug)]
pub struct SampleTimeShared(AtomicU64);

impl SampleTimeShared {
    pub fn new(time: SampleTime) -> Self {
        Self(AtomicU64::new(time.0))
    }

    pub fn load(&self) -> SampleTime {
        SampleTime(self.0.load(Ordering::SeqCst))
    }

    pub fn store(&self, time: SampleTime) {
        self.0.store(time.0, Ordering::SeqCst);
    }
}

#[repr(transparent)]
#[derive(Debug)]
pub struct SecondsShared(AtomicF64);

impl SecondsShared {
    pub fn new(secs: f64) -> Self {
        Self(AtomicF64::new(secs))
    }

    pub fn load(&self) -> f64 {
        self.0.load(Ordering::SeqCst)
    }

    pub fn store(&self, secs: f64) {
        self.0.store(secs, Ordering::SeqCst);
    }
}

/// Musical time in units of sub-beats (where 1 beat = 1920 sub-beats)
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MusicalTime {
    /// The amount of sub-beats (where 1 beat = 1920 sub-beats)
    pub sub_beats: u64,
}

impl MusicalTime {
    /// The number of subdivisions per musical beat
    ///
    /// This number was chosen because it is neatly divisible by a bunch of
    /// common factors such as 2, 3, 4, 5, 6, 8, 16, 32, 64, and 128.
    pub const SUBBEATS_PER_BEAT: u32 = 1920;

    pub const fn new(sub_beats: u64) -> Self {
        Self { sub_beats }
    }

    pub fn from_beats_f64(beats: f64) -> Self {
        assert!(beats >= 0.0);

        let beats_u64 = beats.floor() as u64;
        let fract_sub_beats = (beats.fract() * f64::from(Self::SUBBEATS_PER_BEAT)).round() as u64;

        Self {
            sub_beats: (beats_u64 * u64::from(Self::SUBBEATS_PER_BEAT)) + fract_sub_beats,
        }
    }

    pub fn as_beats_f64(&self) -> f64 {
        let beats_u64 = self.beats();
        let fract_sub_beats_u32 = self.fract_sub_beats();

        beats_u64 as f64
            + (f64::from(fract_sub_beats_u32) * (1.0 / f64::from(Self::SUBBEATS_PER_BEAT)))
    }

    /// The number of whole-beats
    #[inline]
    pub fn beats(&self) -> u64 {
        self.sub_beats / u64::from(Self::SUBBEATS_PER_BEAT)
    }

    /// The number of sub-beats *after* [`Self::beats()`]
    #[inline]
    pub fn fract_sub_beats(&self) -> u32 {
        (self.sub_beats % u64::from(Self::SUBBEATS_PER_BEAT)) as u32
    }
}

impl Add for MusicalTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            sub_beats: self.sub_beats + rhs.sub_beats,
        }
    }
}

impl Sub for MusicalTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            sub_beats: self.sub_beats - rhs.sub_beats,
        }
    }
}

impl AddAssign for MusicalTime {
    fn add_assign(&mut self, rhs: Self) {
        self.sub_beats += rhs.sub_beats;
    }
}

impl SubAssign for MusicalTime {
    fn sub_assign(&mut self, rhs: Self) {
        self.sub_beats -= rhs.sub_beats;
    }
}

/// Describes how to translate real time to musical time
#[derive(Debug, Clone, PartialEq)]
pub enum TempoMap {
    Constant { beats_per_minute: f64 },
    PieceWise { parts: Vec<TempoPart> },
}

/// A single part in a [`TempoMap`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoPart {
    /// The tempo in beats per minute
    pub beats_per_minute: f64,
    /// The length of this part in sub-beats (where 1 beat = 1920 sub-beats)
    pub len_sub_beats: u64,
}

impl TempoMap {
    pub fn musical_to_clock_time(&self, time: MusicalTime, sample_rate: u32) -> SampleTime {
        match self {
            &TempoMap::Constant { beats_per_minute } => {
                let seconds_per_beat = 60.0 / beats_per_minute;

                let beats_f64 = time.as_beats_f64();
                let secs_f64 = beats_f64 * seconds_per_beat;

                SampleTime::from_secs_f64(secs_f64, sample_rate)
            }
            TempoMap::PieceWise { parts: _ } => {
                todo!()
            }
        }
    }

    pub fn clock_time_to_musical(
        &self,
        time: SampleTime,
        sample_rate: u32,
        sample_rate_recip: f64,
    ) -> MusicalTime {
        match self {
            &TempoMap::Constant { beats_per_minute } => {
                let beats_per_second = beats_per_minute * (1.0 / 60.0);

                let secs_f64 = time.as_secs_f64(sample_rate, sample_rate_recip);
                let beats_f64 = secs_f64 * beats_per_second;

                MusicalTime::from_beats_f64(beats_f64)
            }
            TempoMap::PieceWise { parts: _ } => {
                todo!()
            }
        }
    }
}
