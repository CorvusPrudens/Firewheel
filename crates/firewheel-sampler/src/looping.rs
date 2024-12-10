use std::{
    any::Any,
    num::{NonZeroU32, NonZeroUsize},
    sync::Arc,
    u32,
};

use arrayvec::ArrayVec;
use firewheel_core::{
    dsp::{decibel::normalized_volume_to_raw_gain, smoothing_filter},
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, NodeEventType, ProcInfo,
        ProcessStatus,
    },
    sample_resource::SampleResource,
    ChannelConfig, ChannelCount, SilenceMask, StreamInfo,
};
use smallvec::SmallVec;

use crate::voice::SamplerVoice;

pub const MAX_OUT_CHANNELS: usize = 8;
pub const STATIC_ALLOC_TRACKS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossfadeMode {
    PlayNewTracksFromStart,
    SyncTracks,
}

/// The state of the track crossfade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossfadeTracks {
    pub index_a: u32,
    pub index_b: u32,
}

impl CrossfadeTracks {
    pub const FIRST_TRACK_ONLY: Self = Self::single_track(0);

    pub const fn single_track(index: u32) -> Self {
        Self {
            index_a: index,
            index_b: index,
        }
    }

    pub const fn is_single_track(&self) -> bool {
        self.index_a == self.index_b
    }
}

impl Default for CrossfadeTracks {
    fn default() -> Self {
        Self::FIRST_TRACK_ONLY
    }
}

impl From<u64> for CrossfadeTracks {
    fn from(value: u64) -> Self {
        let bytes = value.to_ne_bytes();
        CrossfadeTracks {
            index_a: u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            index_b: u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }
}

impl Into<u64> for CrossfadeTracks {
    fn into(self) -> u64 {
        let bytes_a = self.index_a.to_ne_bytes();
        let bytes_b = self.index_b.to_ne_bytes();
        u64::from_ne_bytes([
            bytes_a[0], bytes_a[1], bytes_a[2], bytes_a[3], bytes_b[0], bytes_b[1], bytes_b[2],
            bytes_b[3],
        ])
    }
}

pub struct Track {
    pub sample: Arc<dyn SampleResource>,
    pub normalized_volume: f32,
}

impl Clone for Track {
    fn clone(&self) -> Self {
        Self {
            sample: Arc::clone(&self.sample),
            normalized_volume: self.normalized_volume,
        }
    }
}

pub enum LoopingSamplerEvent {
    LoadTracks(LoadTracksEvent),
    SetTrackPlayhead {
        track_idx: u32,
        playhead_samples: u64,
    },
}

pub struct LoadTracksEvent {
    pub tracks: SmallVec<[Track; STATIC_ALLOC_TRACKS]>,
    pub crossfade_mode: CrossfadeMode,
    pub crossfade_tracks: CrossfadeTracks,
}

impl LoadTracksEvent {
    pub fn single_track(track: Track) -> Self {
        Self {
            tracks: smallvec::smallvec![track],
            crossfade_mode: CrossfadeMode::PlayNewTracksFromStart,
            crossfade_tracks: CrossfadeTracks::FIRST_TRACK_ONLY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopingSamplerConfig {
    pub max_tracks: NonZeroU32,
    pub declick_duration_seconds: f32,
    pub mono_to_stereo: bool,
}

impl Default for LoopingSamplerConfig {
    fn default() -> Self {
        Self {
            max_tracks: NonZeroU32::new(STATIC_ALLOC_TRACKS as u32).unwrap(),
            declick_duration_seconds: smoothing_filter::DEFAULT_SMOOTH_SECONDS,
            mono_to_stereo: true,
        }
    }
}

pub struct LoopingSamplerNode {
    config: LoopingSamplerConfig,
}

impl LoopingSamplerNode {
    /// The ID of the crossfade parameter.
    pub const PARAM_CROSSFADE: u32 = 0;
    /// The ID of the tracks parameter.
    pub const PARAM_TRACKS: u32 = 1;

    pub fn new(config: LoopingSamplerConfig) -> Self {
        Self { config }
    }
}

impl AudioNode for LoopingSamplerNode {
    fn debug_name(&self) -> &'static str {
        "looping_sampler"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::ZERO,
            num_max_supported_inputs: ChannelCount::ZERO,
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::new(MAX_OUT_CHANNELS as u32).unwrap(),
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: ChannelCount::STEREO,
            },
            equal_num_ins_and_outs: false,
            updates: false,
            uses_events: true,
        }
    }

    fn activate(
        &mut self,
        stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(LoopingSamplerProcessor::new(
            stream_info,
            &self.config,
        )))
    }
}

impl Into<Box<dyn AudioNode>> for LoopingSamplerNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}

struct LoopingSamplerProcessor {
    tracks: SmallVec<[TrackState; STATIC_ALLOC_TRACKS]>,
    /// Used when changing the playhead or when stopping the old tracks
    /// to be replaced with new ones.
    tmp_tracks: SmallVec<[TrackState; STATIC_ALLOC_TRACKS]>,
    num_active_tracks: usize,
    max_tracks: usize,

    declick_filter_coeff: smoothing_filter::Coeff,
    mono_to_stereo: bool,

    crossfade_tracks: CrossfadeTracks,
    crossfade_val: f32,
    crossfade_gain_a: f32,
    crossfade_gain_b: f32,

    crossfade_a_smooth_filter_state: f32,
    crossfade_a_smooth_filter_target: f32,
    crossfade_b_smooth_filter_state: f32,
    crossfade_b_smooth_filter_target: f32,
}

impl LoopingSamplerProcessor {
    pub fn new(stream_info: &StreamInfo, config: &LoopingSamplerConfig) -> Self {
        let max_tracks = config.max_tracks.get() as usize;

        Self {
            tracks: SmallVec::with_capacity(max_tracks),
            tmp_tracks: SmallVec::with_capacity(max_tracks),
            num_active_tracks: 0,
            max_tracks,
            declick_filter_coeff: smoothing_filter::Coeff::new(
                stream_info.sample_rate,
                config.declick_duration_seconds,
            ),
            mono_to_stereo: config.mono_to_stereo,
            crossfade_tracks: CrossfadeTracks::FIRST_TRACK_ONLY,
            crossfade_val: 0.0,
            crossfade_gain_a: 1.0,
            crossfade_gain_b: 0.0,
            crossfade_a_smooth_filter_state: 1.0,
            crossfade_a_smooth_filter_target: 1.0,
            crossfade_b_smooth_filter_state: 0.0,
            crossfade_b_smooth_filter_target: 0.0,
        }
    }
}

impl AudioNodeProcessor for LoopingSamplerProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let mut set_crossfade_tracks =
            |num_tracks: usize, mut crossfade_tracks: CrossfadeTracks| {
                if num_tracks == 0 {
                    self.crossfade_tracks = CrossfadeTracks::FIRST_TRACK_ONLY;
                    return;
                }

                if crossfade_tracks.index_a as usize >= num_tracks {
                    log_track_out_of_range_error(crossfade_tracks.index_a, num_tracks);
                    crossfade_tracks.index_a = 0;
                }
                if crossfade_tracks.index_b as usize >= num_tracks {
                    log_track_out_of_range_error(crossfade_tracks.index_b, num_tracks);
                    crossfade_tracks.index_b = 0;
                }

                self.crossfade_tracks = crossfade_tracks;
            };

        for msg in events {
            match msg {
                NodeEventType::F32Param {
                    id,
                    value,
                    smoothing,
                } => {}
                NodeEventType::U64Param {
                    id,
                    value,
                    smoothing,
                } => {}
                NodeEventType::Pause => {
                    todo!()
                }
                NodeEventType::Resume => {
                    todo!()
                }
                NodeEventType::Stop => {
                    todo!()
                }
                NodeEventType::Custom(event) => {
                    let Some(event) = event.downcast_ref::<LoopingSamplerEvent>() else {
                        continue;
                    };

                    match event {
                        LoopingSamplerEvent::LoadTracks(event) => {
                            if event.tracks.len() > self.max_tracks {
                                log::warn!(
                                    "Recieved a LoadTracksEvent with {} tracks on a LoopingSamplerNode with {} allocated tracks. Please increase the allocated tracks to avoid allocating on the audio thread.",
                                    event.tracks.len(),
                                    self.max_tracks
                                );

                                self.max_tracks = event.tracks.len();

                                if event.tracks.len() > self.tracks.capacity() {
                                    self.tracks.reserve(event.tracks.len() - self.tracks.len());
                                }
                                if event.tracks.len() > self.tmp_tracks.capacity() {
                                    self.tmp_tracks
                                        .reserve(event.tracks.len() - self.tmp_tracks.len());
                                }
                            }

                            self.tmp_tracks.clear();
                            if self.num_active_tracks != 0 {
                                // Declick the old tracks that are still playing.
                                std::mem::swap(&mut self.tracks, &mut self.tmp_tracks);
                                for track in self.tmp_tracks.iter_mut() {
                                    track.voice.pause();
                                }
                            }

                            self.tracks = event
                                .tracks
                                .iter()
                                .map(|track| {
                                    let mut gain =
                                        normalized_volume_to_raw_gain(track.normalized_volume);
                                    if gain < 0.00001 {
                                        gain = 0.0;
                                    }
                                    if gain > 0.99999 && gain < 1.00001 {
                                        gain = 1.0;
                                    }

                                    let mut state = TrackState {
                                        voice: SamplerVoice::new(),
                                    };

                                    state.voice.init_with_sample(&track.sample, gain, 0);

                                    state
                                })
                                .collect();

                            set_crossfade_tracks(self.tracks.len(), event.crossfade_tracks);
                        }
                        LoopingSamplerEvent::SetTrackPlayhead {
                            track_idx,
                            playhead_samples,
                        } => {
                            todo!()
                        }
                    }
                }
                _ => {}
            }
        }

        todo!()
    }
}

struct TrackState {
    voice: SamplerVoice,
}

fn log_track_out_of_range_error(i: u32, num_tracks: usize) {
    log::error!(
        "Track index {} is out of range in LoopingSamplerNode with {} tracks, reverting to track 0",
        i,
        num_tracks
    );
}
