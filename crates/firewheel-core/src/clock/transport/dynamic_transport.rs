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

/// A musical transport with multiple keyframes of tempo. The tempo
/// immediately jumps from one keyframe to another (the tempo is *NOT*
/// linearly interpolated between keyframes).
#[derive(Debug, Clone)]
pub struct DynamicTransport {
    keyframes: Vec<DynamicTransportKeyframe>,
    cache: Vec<KeyframeCache>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicTransportKeyframe {
    pub beats_per_minute: f64,
    pub duration: DurationMusical,
}

impl DynamicTransport {
    pub fn new(keyframes: Vec<DynamicTransportKeyframe>) -> Self {
        assert_ne!(keyframes.len(), 0);

        let mut new_self = Self {
            keyframes,
            cache: Vec::new(),
        };

        new_self.compute_cache();

        new_self
    }

    pub fn keyframes(&self) -> &[DynamicTransportKeyframe] {
        &self.keyframes
    }

    pub fn edit_keyframes(&mut self, f: impl FnOnce(&mut Vec<DynamicTransportKeyframe>)) {
        (f)(&mut self.keyframes);

        self.compute_cache();
    }

    pub fn musical_to_seconds(
        &self,
        musical: InstantMusical,
        transport_start: InstantSeconds,
        speed_multiplier: f64,
    ) -> InstantSeconds {
        transport_start + self.musical_to_seconds_inner(musical, speed_multiplier)
    }

    pub fn musical_to_samples(
        &self,
        musical: InstantMusical,
        transport_start: InstantSamples,
        speed_multiplier: f64,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        transport_start
            + self
                .musical_to_seconds_inner(musical, speed_multiplier)
                .to_samples(sample_rate)
    }

    pub fn seconds_to_musical(
        &self,
        seconds: InstantSeconds,
        transport_start: InstantSeconds,
        speed_multiplier: f64,
    ) -> InstantMusical {
        self.seconds_to_musical_inner(seconds - transport_start, speed_multiplier)
    }

    pub fn samples_to_musical(
        &self,
        sample_time: InstantSamples,
        transport_start: InstantSamples,
        speed_multiplier: f64,
        sample_rate: NonZeroU32,
        sample_rate_recip: f64,
    ) -> InstantMusical {
        self.seconds_to_musical_inner(
            (sample_time - transport_start).to_seconds(sample_rate, sample_rate_recip),
            speed_multiplier,
        )
    }

    pub fn delta_seconds_from(
        &self,
        from: InstantMusical,
        delta_seconds: DurationSeconds,
        speed_multiplier: f64,
    ) -> InstantMusical {
        self.seconds_to_musical_inner(
            self.musical_to_seconds_inner(from, speed_multiplier) + delta_seconds,
            speed_multiplier,
        )
    }

    pub fn transport_start(
        &self,
        now: InstantSamples,
        playhead: InstantMusical,
        speed_multiplier: f64,
        sample_rate: NonZeroU32,
    ) -> InstantSamples {
        now - self
            .musical_to_seconds_inner(playhead, speed_multiplier)
            .to_samples(sample_rate)
    }

    pub fn bpm_at_musical(&self, musical: InstantMusical, speed_multiplier: f64) -> f64 {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if musical < self.cache[i].start_time_musical {
                return self.keyframes[i - 1].beats_per_minute * speed_multiplier;
            }
        }

        self.keyframes.last().unwrap().beats_per_minute * speed_multiplier
    }

    pub fn proc_transport_info(
        &self,
        frames: usize,
        playhead: InstantMusical,
        speed_multiplier: f64,
        sample_rate: NonZeroU32,
    ) -> ProcTransportInfo {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if playhead < self.cache[i].start_time_musical {
                let frames_left_in_keyframe = DurationSeconds(
                    (self.cache[i].start_time_musical - playhead).0
                        * seconds_per_beat(
                            self.keyframes[i - 1].beats_per_minute,
                            speed_multiplier,
                        ),
                )
                .to_samples(sample_rate)
                .0 as usize;

                return ProcTransportInfo {
                    frames: frames.min(frames_left_in_keyframe),
                    beats_per_minute: self.keyframes[i - 1].beats_per_minute * speed_multiplier,
                };
            }
        }

        ProcTransportInfo {
            frames,
            beats_per_minute: self.keyframes.last().unwrap().beats_per_minute * speed_multiplier,
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
                    keyframe.duration.0 * seconds_per_beat(keyframe.beats_per_minute, 1.0),
                );

                cached
            })
            .collect();
    }

    fn musical_to_seconds_inner(
        &self,
        musical: InstantMusical,
        speed_multiplier: f64,
    ) -> DurationSeconds {
        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if musical < self.cache[i].start_time_musical {
                return (self.cache[i - 1].start_time_seconds
                    + DurationSeconds(
                        (musical - self.cache[i - 1].start_time_musical).0
                            * seconds_per_beat(self.keyframes[i - 1].beats_per_minute, 1.0),
                    ))
                    / speed_multiplier;
            }
        }

        (self.cache.last().unwrap().start_time_seconds
            + DurationSeconds(
                (musical - self.cache.last().unwrap().start_time_musical).0
                    * seconds_per_beat(self.keyframes.last().unwrap().beats_per_minute, 1.0),
            ))
            / speed_multiplier
    }

    fn seconds_to_musical_inner(
        &self,
        seconds: DurationSeconds,
        speed_multiplier: f64,
    ) -> InstantMusical {
        let mult_seconds = seconds * speed_multiplier;

        // TODO: Use a binary search algorithm.
        for i in 1..self.keyframes.len().min(self.cache.len()) {
            if mult_seconds < self.cache[i].start_time_seconds {
                return self.cache[i - 1].start_time_musical
                    + DurationMusical(
                        (mult_seconds - self.cache[i - 1].start_time_seconds).0
                            * beats_per_second(self.keyframes[i - 1].beats_per_minute, 1.0),
                    );
            }
        }

        self.cache.last().unwrap().start_time_musical
            + DurationMusical(
                (mult_seconds - self.cache.last().unwrap().start_time_seconds).0
                    * beats_per_second(self.keyframes.last().unwrap().beats_per_minute, 1.0),
            )
    }
}

impl PartialEq for DynamicTransport {
    fn eq(&self, other: &Self) -> bool {
        self.keyframes.eq(&other.keyframes)
    }
}
