/* This has turned out to be quite complex, so I'll finish this later in a separate PR.
//
/// A musical transport with linearly automated tempo.
#[derive(Clone)]
pub struct AutomatedTransport {
    keyframes: Vec<TransportKeyframe>,
    // Contains the start time in seconds for each keyframe.
    cache: Vec<ClockSeconds>,
}

impl Default for AutomatedTransport {
    fn default() -> Self {
        Self {
            keyframes: vec![TransportKeyframe::default()],
            cache: vec![ClockSeconds(0.0)],
        }
    }
}

impl AutomatedTransport {
    pub fn new(keyframes: Vec<TransportKeyframe>) -> Result<Self, DynamicTransportError> {
        // Check prequisits.
        if keyframes.is_empty() {
            return Err(DynamicTransportError::Empty);
        }

        let mut prev_time = MusicalTime(f64::NEG_INFINITY);
        for k in keyframes.iter() {
            if prev_time >= k.time {
                return Err(DynamicTransportError::ImproperOrdering);
            }
            if k.beats_per_minute <= 0.0 {
                return Err(DynamicTransportError::InvalidBPM(k.beats_per_minute));
            }

            prev_time = k.time;
        }

        let mut cache = Vec::with_capacity(keyframes.len());
        cache.push(ClockSeconds(0.0));

        let mut seconds = ClockSeconds(0.0);
        for (i, k) in keyframes.iter().enumerate().skip(1) {
            let prev_k = &keyframes[i - 1];

            let delta_seconds = if prev_k.interpolate_to_next {
                linear_interp_bpm_to_delta_seconds(
                    prev_k.time,
                    k.time,
                    prev_k.beats_per_minute,
                    k.beats_per_minute,
                )
            } else {
                (k.time - prev_k.time).to_seconds(prev_k.beats_per_minute)
            };

            seconds += delta_seconds;
            cache.push(seconds);
        }

        Ok(Self { keyframes, cache })
    }

    pub fn musical_to_seconds(&self, musical: MusicalTime) -> ClockSeconds {
        ClockSeconds(musical.0 * self.seconds_per_beat())
    }

    pub fn musical_to_samples(
        &self,
        musical: MusicalTime,
        sample_rate: NonZeroU32,
    ) -> ClockSamples {
        self.musical_to_seconds(musical).to_samples(sample_rate)
    }

    pub fn samples_to_musical(
        &self,
        sample_time: ClockSamples,
        sample_rate: NonZeroU32,
        sample_rate_recip: f64,
    ) -> MusicalTime {
        self.seconds_to_musical(sample_time.to_seconds(sample_rate, sample_rate_recip))
    }

    pub fn seconds_to_musical(&self, seconds: ClockSeconds) -> MusicalTime {
        MusicalTime(seconds.0 * self.beats_per_second())
    }

    /// Return the musical time that occurs `delta_seconds` seconds after the
    /// given `from` timestamp.
    pub fn delta_seconds_from(
        &self,
        from: MusicalTime,
        delta_seconds: ClockSeconds,
    ) -> MusicalTime {
        from + self.seconds_to_musical(delta_seconds)
    }
}

fn linear_interp_bpm_to_delta_seconds(
    from_beat: MusicalTime,
    to_beat: MusicalTime,
    from_bpm: f64,
    to_bpm: f64,
) -> ClockSeconds {
    // This can be solved with the standard kinematic equation of displacement
    //
    // delta_x = (v0 * t) + (1/2 * a * t^2)
    //
    // where
    // x = minutes
    // v = 1/bpm = mpb
    // t = beats1 - beats0 = delta_beats
    // a = (mpb1 - mbp0) / (beats1 - beats0) = delta_mpb / delta_beats
    //
    // which gives us
    //
    // delta_minutes = (mpb0 * delta_beats) + (1/2 * (delta_mpb / delta_beats) * delta_beats^2)
    //
    // and simplifies to:
    //
    // delta_minutes = (mpb0 * delta_beats) + (1/2 * delta_mpb * delta_beats)

    let delta_beats = (to_beat - from_beat).0;

    let from_mpb = from_bpm.recip();
    let delta_mpb = to_bpm.recip() - from_mpb;

    let delta_minutes = (from_mpb * delta_beats) + (0.5 * delta_mpb * delta_beats);

    ClockSeconds(delta_minutes * 60.0)
}

#[derive(Debug, thiserror::Error)]
pub enum DynamicTransportError {
    /// The dynamic tranport contained no keyframes.
    #[error("The dynamic transport contained no keyframes")]
    Empty,
    /// The keyframes of the dynamic transport are not properly ordered.
    #[error("The keyframes of the dynamic transport are not properly ordered")]
    ImproperOrdering,
    /// Invalid BPM value.
    #[error("Invalid beats per minute value: {0}")]
    InvalidBPM(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportKeyframe {
    /// The musical time at which this keyframe occurs.
    pub time: MusicalTime,

    /// The beats per minute at the start of this keyframe.
    pub beats_per_minute: f64,

    /// If `true`, then the bpm will linearly interpolate from this keyframe to the
    /// next keyframe. If `false`, then the bpm will stay static until right before
    /// the next keyframe, and then immediately jump to the next keyframe.
    pub interpolate_to_next: bool,
}

impl Default for TransportKeyframe {
    fn default() -> Self {
        Self {
            time: MusicalTime::ZERO,
            beats_per_minute: 110.0,
            interpolate_to_next: false,
        }
    }
}
*/
