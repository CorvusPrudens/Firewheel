use core::num::NonZeroU32;

use crate::clock::{
    beats_per_second, seconds_per_beat, DurationMusical, DurationSeconds, InstantMusical,
    InstantSamples, InstantSeconds, ProcTransportInfo,
};

#[derive(Debug, Clone)]
struct KeyframeCache {
    start_time_musical: InstantMusical,
    start_time_seconds: DurationSeconds,
}

/// A musical transport with a single static tempo in beats per minute.
#[derive(Debug, Clone)]
pub struct PiecewiseTransport {
    keyframes: Vec<PiecewiseTransportKeyframe>,
    cache: Vec<KeyframeCache>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PiecewiseTransportKeyframe {
    pub beats_per_minute: f64,
    pub duration: DurationMusical,
}

impl PiecewiseTransport {
    pub fn new(keyframes: Vec<PiecewiseTransportKeyframe>) -> Self {
        assert_ne!(keyframes.len(), 0);

        let mut new_self = Self {
            keyframes,
            cache: Vec::new(),
        };

        new_self.compute_cache();

        new_self
    }

    pub fn keyframes(&self) -> &[PiecewiseTransportKeyframe] {
        &self.keyframes
    }

    pub fn edit_keyframes(&mut self, f: impl FnOnce(&mut Vec<PiecewiseTransportKeyframe>)) {
        (f)(&mut self.keyframes);

        self.compute_cache();
    }

    pub fn musical_to_seconds(
        &self,
        musical: InstantMusical,
        transport_start: InstantSeconds,
    ) -> InstantSeconds {
        transport_start + self.musical_to_seconds_inner(musical)
    }

    pub fn musical_to_samples(
        &self,
        musical: InstantMusical,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        transport_start
            + self
                .musical_to_seconds_inner(musical)
                .to_samples(sample_rate)
    }

    pub fn seconds_to_musical(
        &self,
        seconds: InstantSeconds,
        transport_start: InstantSeconds,
    ) -> InstantMusical {
        self.seconds_to_musical_inner(seconds - transport_start)
    }

    pub fn samples_to_musical(
        &self,
        sample_time: InstantSamples,
        transport_start: InstantSamples,
        sample_rate: NonZeroU32,
        sample_rate_recip: f64,
    ) -> InstantMusical {
        self.seconds_to_musical_inner(
            (sample_time - transport_start).to_seconds(sample_rate, sample_rate_recip),
        )
    }

    /// Return the musical time that occurs `delta_seconds` seconds after the
    /// given `from` timestamp.
    pub fn delta_seconds_from(
        &self,
        from: InstantMusical,
        delta_seconds: DurationSeconds,
    ) -> InstantMusical {
        self.seconds_to_musical_inner(self.musical_to_seconds_inner(from) + delta_seconds)
    }

    pub fn transport_start(
        &self,
        now: InstantSamples,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        now - self
            .musical_to_seconds_inner(playhead)
            .to_samples(sample_rate)
    }

    pub fn bpm_at_musical(&self, musical: InstantMusical) -> f64 {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if musical < self.cache[i].start_time_musical {
                return self.keyframes[i - 1].beats_per_minute;
            }
        }

        self.keyframes.last().unwrap().beats_per_minute
    }

    pub fn proc_transport_info(
        &self,
        frames: usize,
        playhead: InstantMusical,
        sample_rate: NonZeroU32,
    ) -> ProcTransportInfo {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if playhead < self.cache[i].start_time_musical {
                let frames_left_in_keyframe = DurationSeconds(
                    (self.cache[i].start_time_musical - playhead).0
                        * seconds_per_beat(self.keyframes[i - 1].beats_per_minute),
                )
                .to_samples(sample_rate)
                .0 as usize;

                return ProcTransportInfo {
                    frames: frames.min(frames_left_in_keyframe),
                    beats_per_minute: self.keyframes[i - 1].beats_per_minute,
                    delta_beats_per_minute: 0.0,
                };
            }
        }

        ProcTransportInfo {
            frames,
            beats_per_minute: self.keyframes.last().unwrap().beats_per_minute,
            delta_beats_per_minute: 0.0,
        }
    }

    fn compute_cache(&mut self) {
        let mut start_time_musical = InstantMusical::ZERO;
        let mut start_time_seconds = DurationSeconds::ZERO;

        self.cache = self
            .keyframes
            .iter()
            .map(|keyframe| {
                let cached = KeyframeCache {
                    start_time_musical,
                    start_time_seconds,
                };

                start_time_musical += keyframe.duration;
                start_time_seconds += DurationSeconds(
                    keyframe.duration.0 * seconds_per_beat(keyframe.beats_per_minute),
                );

                cached
            })
            .collect();
    }

    fn musical_to_seconds_inner(&self, musical: InstantMusical) -> DurationSeconds {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if musical < self.cache[i].start_time_musical {
                return self.cache[i - 1].start_time_seconds
                    + DurationSeconds(
                        (musical - self.cache[i - 1].start_time_musical).0
                            * seconds_per_beat(self.keyframes[i - 1].beats_per_minute),
                    );
            }
        }

        self.cache.last().unwrap().start_time_seconds
            + DurationSeconds(
                (musical - self.cache.last().unwrap().start_time_musical).0
                    * seconds_per_beat(self.keyframes.last().unwrap().beats_per_minute),
            )
    }

    fn seconds_to_musical_inner(&self, seconds: DurationSeconds) -> InstantMusical {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if seconds < self.cache[i].start_time_seconds {
                return self.cache[i - 1].start_time_musical
                    + DurationMusical(
                        (seconds - self.cache[i - 1].start_time_seconds).0
                            * beats_per_second(self.keyframes[i - 1].beats_per_minute),
                    );
            }
        }

        self.cache.last().unwrap().start_time_musical
            + DurationMusical(
                (seconds - self.cache.last().unwrap().start_time_seconds).0
                    * beats_per_second(self.keyframes.last().unwrap().beats_per_minute),
            )
    }
}

impl PartialEq for PiecewiseTransport {
    fn eq(&self, other: &Self) -> bool {
        self.keyframes.eq(&other.keyframes)
    }
}
