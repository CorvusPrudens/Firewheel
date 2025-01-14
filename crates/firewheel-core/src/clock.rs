use std::ops::{Add, AddAssign, Sub, SubAssign};

use crate::node::ProcInfo;

/// When a particular audio event should occur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventDelay {
    /// The event should happen when the clock reaches the given time in
    /// seconds.
    ///
    /// Note, this clock is not perfectly accurate, but it does correctly
    /// account for any output underflows that may occur.
    ///
    /// The value is an absolute time, *NOT* a delta time. Use
    /// [`FirewheelCtx::clock_now`] to get the current time of the clock.
    DelayUntilSeconds(ClockSeconds),

    /// The event should happen when the clock reaches the given time in
    /// samples (of a single channel of audio).
    ///
    /// This is more accurate than `DelayUntilSeconds`, but it does not
    /// account for any output underflows that may occur. This clock is
    /// ideal for syncing events to a custom musical transport.
    ///
    /// The value is an absolute time, *NOT* a delta time. Use
    /// [`FirewheelCtx::clock_samples`] to get the current time of the clock.
    DelayUntilSamples(ClockSamples),

    /// The event should happen when the musical clock reaches the given
    /// musical time.
    ///
    /// Like `DelayUntilSamples`, this is very accurate, but note it also
    /// does not account for any output underflows that may occur.
    DelayUntilMusical(MusicalTime),
}

impl EventDelay {
    pub fn elapsed_or_get(&self, proc_info: &ProcInfo) -> Option<Self> {
        match self {
            EventDelay::DelayUntilSeconds(seconds) => {
                if *seconds <= proc_info.clock_seconds.start {
                    None
                } else {
                    Some(*self)
                }
            }
            EventDelay::DelayUntilSamples(samples) => {
                if *samples <= proc_info.clock_samples {
                    None
                } else {
                    Some(*self)
                }
            }
            EventDelay::DelayUntilMusical(musical) => {
                if let Some(transport) = &proc_info.transport_info {
                    if transport.paused || *musical <= transport.musical_clock.start {
                        None
                    } else {
                        Some(*self)
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn elapsed_on_frame(&self, proc_info: &ProcInfo, sample_rate: u32) -> Option<usize> {
        match self {
            EventDelay::DelayUntilSeconds(seconds) => {
                if *seconds <= proc_info.clock_seconds.start {
                    Some(0)
                } else if *seconds >= proc_info.clock_seconds.end {
                    None
                } else {
                    let frame = ((seconds.0 - proc_info.clock_seconds.start.0)
                        * f64::from(sample_rate))
                    .round() as usize;

                    if frame >= proc_info.frames {
                        None
                    } else {
                        Some(frame)
                    }
                }
            }
            EventDelay::DelayUntilSamples(samples) => {
                if *samples <= proc_info.clock_samples {
                    Some(0)
                } else {
                    let frame = samples.0 - proc_info.clock_samples.0;

                    if frame >= proc_info.frames as i64 {
                        None
                    } else {
                        Some(frame as usize)
                    }
                }
            }
            EventDelay::DelayUntilMusical(musical) => {
                if let Some(transport) = &proc_info.transport_info {
                    if transport.paused || *musical >= transport.musical_clock.end {
                        None
                    } else if *musical <= transport.musical_clock.start {
                        Some(0)
                    } else {
                        let frame = transport.transport.musical_to_sample(*musical, sample_rate)
                            - proc_info.clock_samples;

                        if frame.0 >= proc_info.frames as i64 {
                            None
                        } else {
                            Some(frame.0 as usize)
                        }
                    }
                } else {
                    None
                }
            }
        }
    }
}

/// An absolute clock time in units of seconds.
#[repr(transparent)]
#[derive(Default, Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ClockSeconds(pub f64);

impl Add for ClockSeconds {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for ClockSeconds {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign for ClockSeconds {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for ClockSeconds {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl From<f64> for ClockSeconds {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl Into<f64> for ClockSeconds {
    fn into(self) -> f64 {
        self.0
    }
}

/// An absolute clock time in units of samples (in a single channel of audio).
#[repr(transparent)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockSamples(pub i64);

impl ClockSamples {
    pub const fn new(samples: i64) -> Self {
        Self(samples)
    }

    pub fn from_secs_f64(seconds: f64, sample_rate: u32) -> Self {
        let seconds_i64 = seconds.floor() as i64;
        let fract_samples_i64 = (seconds.fract() * f64::from(sample_rate)).round() as i64;

        Self((seconds_i64 * i64::from(sample_rate)) + fract_samples_i64)
    }

    /// (whole seconds, samples *after* whole seconds)
    pub fn whole_seconds_and_fract(&self, sample_rate: u32) -> (i64, u32) {
        let whole_seconds = self.0 / i64::from(sample_rate);
        let fract_samples = self.0 % i64::from(sample_rate);

        if fract_samples < 0 {
            (
                whole_seconds - 1,
                sample_rate - (fract_samples.abs() as u32),
            )
        } else {
            (whole_seconds, fract_samples as u32)
        }
    }

    #[inline]
    pub fn fract_second_samples(&self, sample_rate: u32) -> u32 {
        (self.0 % i64::from(sample_rate)) as u32
    }

    pub fn as_secs_f64(&self, sample_rate: u32, sample_rate_recip: f64) -> f64 {
        let whole_seconds = self.0 / i64::from(sample_rate);
        let fract_samples = self.0 % i64::from(sample_rate);

        whole_seconds as f64 + (fract_samples as f64 * sample_rate_recip)
    }
}

impl Add for ClockSamples {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for ClockSamples {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign for ClockSamples {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for ClockSamples {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl From<i64> for ClockSamples {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl Into<i64> for ClockSamples {
    fn into(self) -> i64 {
        self.0
    }
}

/// Musical time in units of beats.
#[derive(Default, Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct MusicalTime(pub f64);

impl MusicalTime {
    pub const fn new(beats: f64) -> Self {
        Self(beats)
    }

    /// Convert to the corresponding time in samples.
    pub fn to_sample_time(&self, seconds_per_beat: f64, sample_rate: u32) -> ClockSamples {
        let secs_f64 = self.0 * seconds_per_beat;
        ClockSamples::from_secs_f64(secs_f64, sample_rate)
    }

    /// Convert from the corresponding time in samples.
    pub fn from_sample_time(
        sample_time: ClockSamples,
        beats_per_second: f64,
        sample_rate: u32,
        sample_rate_recip: f64,
    ) -> Self {
        let secs_f64 = sample_time.as_secs_f64(sample_rate, sample_rate_recip);
        MusicalTime(secs_f64 * beats_per_second)
    }
}

pub fn seconds_per_beat(beats_per_minute: f64) -> f64 {
    60.0 / beats_per_minute
}

pub fn beats_per_second(beats_per_minute: f64) -> f64 {
    beats_per_minute * (1.0 / 60.0)
}

impl Add for MusicalTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for MusicalTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign for MusicalTime {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for MusicalTime {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MusicalTransport {
    beats_per_minute: f64,
    seconds_per_beat: f64,
    // TODO: Automated tempo?
}

impl MusicalTransport {
    pub fn new(beats_per_minute: f64) -> Self {
        Self {
            beats_per_minute,
            seconds_per_beat: seconds_per_beat(beats_per_minute),
        }
    }

    pub fn beats_per_minute(&self) -> f64 {
        self.beats_per_minute
    }

    pub fn seconds_per_beat(&self) -> f64 {
        self.seconds_per_beat
    }
}

impl MusicalTransport {
    /// Convert from musical time the corresponding time in samples.
    pub fn musical_to_sample(&self, musical: MusicalTime, sample_rate: u32) -> ClockSamples {
        musical.to_sample_time(self.seconds_per_beat, sample_rate)
    }

    /// Convert from the time in samples to the corresponding musical time.
    pub fn sample_to_musical(
        &self,
        sample_time: ClockSamples,
        sample_rate: u32,
        sample_rate_recip: f64,
    ) -> MusicalTime {
        MusicalTime::from_sample_time(
            sample_time,
            self.beats_per_minute * (1.0 / 60.0),
            sample_rate,
            sample_rate_recip,
        )
    }
}
