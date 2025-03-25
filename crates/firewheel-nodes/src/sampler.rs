use smallvec::SmallVec;
use std::{
    num::{NonZeroU32, NonZeroUsize},
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount, NonZeroChannelCount},
    clock::{ClockSamples, ClockSeconds, EventDelay},
    collector::ArcGc,
    diff::{Diff, EventQueue, PathBuilder},
    dsp::{
        buffer::InstanceBuffer,
        declick::{DeclickValues, Declicker, FadeType},
        volume::{Volume, DEFAULT_AMP_EPSILON},
    },
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, ConstructProcessorContext, ProcBuffers,
        ProcInfo, ProcessStatus,
    },
    sample_resource::SampleResource,
    SilenceMask, StreamInfo,
};

pub const MAX_OUT_CHANNELS: usize = 8;
pub const DEFAULT_NUM_DECLICKERS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct SamplerConfig {
    /// The number of channels in this node.
    pub channels: NonZeroChannelCount,
    /// If `true`, then mono samples will be converted to stereo during playback.
    ///
    /// By default this is set to `true`.
    pub mono_to_stereo: bool,
    /// The maximum number of "declickers" present on this node.
    /// The more declickers there are, the more samples that can be declicked
    /// when played in rapid succession. (Note more declickers will allocate
    /// more memory).
    ///
    /// By default this is set to `2`.
    pub num_declickers: u32,
    /// If true, then samples will be crossfaded-in when restarting (if the
    /// sample is currently playing when the restart event is sent).
    ///
    /// By default this is set to `true`.
    pub crossfade_on_restart: bool,
    /// If the resutling amplitude of the volume is less than or equal to this
    /// value, then the amplitude will be clamped to `0.0` (silence).
    pub amp_epsilon: f32,
    /// The quality of the resampling algorithm used for changing the playback
    /// speed.
    pub playback_speed_quality: PlaybackSpeedQuality,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            channels: NonZeroChannelCount::STEREO,
            mono_to_stereo: true,
            num_declickers: DEFAULT_NUM_DECLICKERS as u32,
            crossfade_on_restart: true,
            amp_epsilon: DEFAULT_AMP_EPSILON,
            playback_speed_quality: PlaybackSpeedQuality::default(),
        }
    }
}

/// The quality of the resampling algorithm used for changing the playback
/// speed of a sampler node.
#[non_exhaustive]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackSpeedQuality {
    #[default]
    /// Low quality, fast performance. Recommended for most use cases.
    ///
    /// More specifically, this uses a linear resampling algorithm with no
    /// antialiasing filter.
    Linear,
}

#[derive(Clone)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct SamplerNode {
    /// The current sequence loaded into the sampler.
    pub sequence: Option<SequenceType>,

    /// The current playback state.
    pub playback: PlaybackState,

    /// The playhead state.
    pub playhead: PlayheadState,
    /*
    /// The speed at which to play the sample at. `1.0` means to play the sound at
    /// its original speed, `< 1.0` means to play the sound slower (which will make
    /// it lower-pitched), and `> 1.0` means to play the sound faster (which will
    /// make it higher-pitched).
    pub playback_speed: f64,
    */
}

impl Diff for SamplerNode {
    fn diff<E: EventQueue>(&self, baseline: &Self, _path: PathBuilder, event_queue: &mut E) {
        if self.sequence != baseline.sequence {
            event_queue.push(self.sync_sequence_event());
        }

        if self.playhead != baseline.playhead {
            event_queue.push(self.sync_playhead_event());
        }

        if self.playback != baseline.playback {
            event_queue.push(self.sync_playback_event());
        }

        /*
        if self.playback_speed != baseline.playback_speed {
            event_queue.push(NodeEventType::Param {
                data: ParamData::F64(self.playback_speed),
                path: Self::PLAYBACK_SPEED_PARAM_PATH,
            });
        }
        */
    }
}

impl Default for SamplerNode {
    fn default() -> Self {
        Self {
            sequence: None,
            playback: PlaybackState::default(),
            playhead: PlayheadState::default(),
            //playback_speed: 1.0,
        }
    }
}

impl SamplerNode {
    /// Set the parameters to a play a single sample.
    ///
    /// * `sample` - The sample resource to use.
    /// * `volume` - The volume to play the sample at. Note that this node does not
    /// support changing the volume while playing. Instead, use a node like the volume
    /// node for that.
    /// * `repeat_mode` - How many times a sample/sequence should be repeated for each
    /// `StartOrRestart` command.
    pub fn set_sample(
        &mut self,
        sample: ArcGc<dyn SampleResource>,
        volume: Volume,
        repeat_mode: RepeatMode,
    ) {
        self.sequence = Some(SequenceType::SingleSample {
            sample,
            volume,
            repeat_mode,
        });
    }

    /// Returns an event type to sync the `sequence` parameter.
    pub fn sync_sequence_event(&self) -> NodeEventType {
        NodeEventType::Custom(Box::new(SamplerEvent::SetSequence {
            sequence: self.sequence.clone(),
        }))
    }

    /// Returns an event type to sync the `playback` parameter.
    pub fn sync_playback_event(&self) -> NodeEventType {
        NodeEventType::Custom(Box::new(SamplerEvent::SetPlayback(self.playback)))
    }

    /// Returns an event type to sync the `playhead` parameter.
    pub fn sync_playhead_event(&self) -> NodeEventType {
        NodeEventType::Custom(Box::new(SamplerEvent::SetPlayhead(self.playhead)))
    }

    /// Play the sequence in this node.
    ///
    /// If a sequence is already playing, then it will restart from the beginning.
    pub fn start_or_restart(&mut self, delay: Option<EventDelay>) {
        self.playhead.restart();
        self.playback = PlaybackState::Play { delay };
    }

    /// Pause sequence playback.
    pub fn pause(&mut self) {
        self.playback = PlaybackState::Pause;
    }

    /// Resume sequence playback.
    pub fn resume(&mut self, delay: Option<EventDelay>) {
        self.playback = PlaybackState::Play { delay };
    }

    /// Stop sequence playback.
    ///
    /// Calling [`SamplerNode::resume`] after this will restart the sequence from
    /// the beginning.
    pub fn stop(&mut self) {
        self.playback = PlaybackState::Stop;
        self.playhead.restart();
    }
}

#[derive(Clone)]
pub struct SamplerState {
    shared_state: ArcGc<SharedState>,
}

impl SamplerState {
    fn new() -> Self {
        Self {
            shared_state: ArcGc::new(SharedState::default()),
        }
    }

    /// Get the current position of the playhead in units of frames (samples of
    /// a single channel of audio).
    pub fn playhead_frames(&self) -> u64 {
        self.shared_state
            .sequence_playhead_frames
            .load(Ordering::Relaxed)
    }

    /// Get the current position of the sequence playhead in seconds.
    ///
    /// * `sample_rate` - The sample rate of the current audio stream.
    pub fn playhead_seconds(&self, sample_rate: NonZeroU32) -> f64 {
        self.playhead_frames() as f64 / sample_rate.get() as f64
    }

    /// Returns `true` if the sequence has either not started playing yet or has finished
    /// playing.
    pub fn stopped(&self) -> bool {
        self.shared_state.stopped.load(Ordering::Relaxed)
    }

    /// Manually set the shared `stopped` flag. This can be useful to account for the delay
    /// between sending a play event and the node's processor receiving that event.
    pub fn mark_stopped(&self, stopped: bool) {
        self.shared_state.stopped.store(stopped, Ordering::Release);
    }

    /// A score of how suitible this node is to start new work (Play a new sample). The
    /// higher the score, the better the candidate.
    pub fn worker_score(&self, params: &SamplerNode) -> u64 {
        if params.sequence.is_some() {
            let stopped = self.stopped();

            match params.playback {
                PlaybackState::Stop => u64::MAX - 1,
                PlaybackState::Pause => {
                    if stopped {
                        u64::MAX - 2
                    } else {
                        u64::MAX - 3
                    }
                }
                PlaybackState::Play { .. } => {
                    let playhead_frames = self.playhead_frames();

                    if stopped {
                        if playhead_frames > 0 {
                            // Sequence has likely finished playing.
                            u64::MAX - 4
                        } else {
                            // Sequence has likely not started playing yet.
                            u64::MAX - 5
                        }
                    } else {
                        // The older the sample is, the better it is as a candidate to steal
                        // work from.
                        playhead_frames
                    }
                }
            }
        } else {
            u64::MAX
        }
    }
}

/// A parameter representing the current playback state of a sequence.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum PlaybackState {
    /// Stop the sequence.
    ///
    /// When the sequence is started again, it will restart from the beginning.
    #[default]
    Stop,
    /// Pause the sequence.
    ///
    /// When the sequence is started again, it will continue from where it last
    /// left off.
    Pause,
    /// Play the sequence.
    Play {
        /// The exact time at which the sequence should begin playing.
        ///
        /// Set to `None` to play the sequence immediately.
        delay: Option<EventDelay>,
    },
}

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        if let PlaybackState::Play { .. } = self {
            true
        } else {
            false
        }
    }
}

/// A parameter representing the playhead of a sequence.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct PlayheadState {
    pub playhead: Playhead,
    /// A unique ID given for this assignment. Changing this value allows a
    /// diffing system to know when it should send an update event, even if the
    /// value of `playhead` has not changed.
    pub id: u64,
}

impl PlayheadState {
    pub fn restart(&mut self) {
        self.playhead = Playhead::default();
        self.id += 1;
    }

    /// Set the playhead in units of seconds.
    pub const fn set_seconds(&mut self, seconds: f64) {
        self.playhead = Playhead::Seconds(seconds);
    }

    /// Set the playhead in units of frames (samples in a single channel of audio).
    pub const fn set_frames(&mut self, frames: u64) {
        self.playhead = Playhead::Frames(frames);
    }
}

/// The playhead of a sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Playhead {
    /// The playhead in units of seconds.
    Seconds(f64),
    /// The playhead in units of frames (samples in a single channel of audio).
    Frames(u64),
}

impl Playhead {
    pub fn as_frames(&self, sample_rate: u32) -> u64 {
        match *self {
            Self::Seconds(seconds) => {
                if seconds <= 0.0 {
                    0
                } else {
                    (seconds.floor() as u64 * sample_rate as u64)
                        + (seconds.fract() * sample_rate as f64).round() as u64
                }
            }
            Self::Frames(frames) => frames,
        }
    }
}

impl Default for Playhead {
    fn default() -> Self {
        Self::Seconds(0.0)
    }
}

#[derive(Clone)]
pub enum SamplerEvent {
    /// Set the sampler state. This will stop any currently playing sequence.
    SetSequence { sequence: Option<SequenceType> },
    /// Set the playback parameter.
    SetPlayback(PlaybackState),
    /// Set the playhead parameter.
    SetPlayhead(PlayheadState),
}

impl Into<NodeEventType> for SamplerEvent {
    fn into(self) -> NodeEventType {
        NodeEventType::Custom(Box::new(self))
    }
}

/// The current sequence loaded into the sampler.
#[derive(Clone, PartialEq)]
pub enum SequenceType {
    SingleSample {
        /// The sample resource to use.
        sample: ArcGc<dyn SampleResource>,
        /// The volume to play the sample at.
        ///
        /// Note that this node does not support changing the volume while
        /// playing. Instead, use a node like the volume node for that.
        volume: Volume,
        /// How many times a sample/sequence should be repeated.
        repeat_mode: RepeatMode,
    },
    /// A sequence with multiple events (NOT IMPLEMENTED YET, WILL PANIC IF USED)
    Sequence {
        sequence: ArcGc<Vec<SequenceEvent>>,
        timing: SequenceTiming,
    },
}

/// The method of timing to use for a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceTiming {
    /// Use time in units of seconds.
    Seconds,
    /// Use time in units of samples (of a single channel of audio).
    Samples,
}

#[derive(Clone, PartialEq)]
pub struct SequenceEvent {
    pub event: SequenceEventType,
    /// The amount of time from the start of the sequence that this event should occur.
    ///
    /// If the timing is set to [`SequenceTiming::Seconds`], then this is in units of seconds. If
    /// the timing is set to [`SequenceTiming::Samples`], then this is in units of samples (of a
    /// single channel of audio).
    pub offset_from_start: f64,
}

#[derive(Clone, PartialEq)]
pub enum SequenceEventType {
    PlaySample {
        /// The sample resource to use.
        sample: ArcGc<dyn SampleResource>,
        /// The volume to play the sample at, where `0.0` is silence and `1.0`
        /// is unity gain.
        ///
        /// Note that this node does not support changing the volume while
        /// playing. Instead, use a node like the volume node for that.
        normalized_volume: f32,
        // TODO: Pitch
    },
    /// Stop the currently playing sample.
    Stop,
    /// Set the position of the playhead in seconds.
    SetPlayheadSeconds(f64),
    /// Set the position of the playhead in units of samples (of a single channel
    /// of audio).
    SetPlayheadSamples(u64),
}

/// How many times a sample/sequence should be repeated for each `StartOrRestart` command.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    /// Play the sample/sequence once and then stop.
    #[default]
    PlayOnce,
    /// Repeat the sample/sequence the given number of times.
    RepeatMultiple { num_times_to_repeat: u32 },
    /// Repeat the sample/sequence endlessly.
    RepeatEndlessly,
}

impl RepeatMode {
    pub fn do_loop(&self, num_times_looped_back: u64) -> bool {
        match self {
            Self::PlayOnce => false,
            &Self::RepeatMultiple {
                num_times_to_repeat,
            } => num_times_looped_back < num_times_to_repeat as u64,
            Self::RepeatEndlessly => true,
        }
    }
}

impl AudioNode for SamplerNode {
    type Configuration = SamplerConfig;

    fn info(&self, config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo::new()
            .debug_name("sampler")
            .channel_config(ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: config.channels.get(),
            })
            .uses_events(true)
            .custom_state(SamplerState::new())
    }

    fn construct_processor(
        &self,
        config: &Self::Configuration,
        cx: ConstructProcessorContext,
    ) -> impl AudioNodeProcessor {
        let stop_declicker_buffers = if config.num_declickers == 0 {
            None
        } else {
            Some(InstanceBuffer::<f32, MAX_OUT_CHANNELS>::new(
                config.num_declickers as usize,
                NonZeroUsize::new(config.channels.get().get() as usize).unwrap(),
                cx.stream_info.declick_frames.get() as usize,
            ))
        };

        SamplerProcessor {
            config: config.clone(),
            params: self.clone(),
            shared_state: ArcGc::clone(&cx.custom_state::<SamplerState>().unwrap().shared_state),
            loaded_sample_state: None,
            declicker: Declicker::SettledAt1,
            stop_declicker_buffers,
            stop_declickers: smallvec::smallvec![StopDeclickerState::default(); config.num_declickers as usize],
            num_active_stop_declickers: 0,
            playback_state: self.playback,
            playback_start_time_seconds: ClockSeconds::default(),
            playback_pause_time_seconds: ClockSeconds::default(),
            playback_start_time_frames: ClockSamples::default(),
            playback_pause_time_frames: ClockSamples::default(),
            start_delay: None,
            sample_rate: cx.stream_info.sample_rate.get() as f64,
            amp_epsilon: config.amp_epsilon,
            is_first_process: true,
        }
    }
}

pub struct SamplerProcessor {
    config: SamplerConfig,
    params: SamplerNode,
    shared_state: ArcGc<SharedState>,

    loaded_sample_state: Option<LoadedSampleState>,

    declicker: Declicker,

    playback_state: PlaybackState,

    stop_declicker_buffers: Option<InstanceBuffer<f32, MAX_OUT_CHANNELS>>,
    stop_declickers: SmallVec<[StopDeclickerState; DEFAULT_NUM_DECLICKERS]>,
    num_active_stop_declickers: usize,

    playback_start_time_seconds: ClockSeconds,
    playback_pause_time_seconds: ClockSeconds,
    playback_start_time_frames: ClockSamples,
    playback_pause_time_frames: ClockSamples,

    start_delay: Option<EventDelay>,

    sample_rate: f64,
    amp_epsilon: f32,

    is_first_process: bool,
}

impl SamplerProcessor {
    /// Returns `true` if the sample has finished playing, and also
    /// returns the number of channels that were filled.
    fn process_internal(
        &mut self,
        buffers: &mut [&mut [f32]],
        frames: usize,
        looping: bool,
        declick_values: &DeclickValues,
        start_on_frame: Option<usize>,
    ) -> (bool, usize) {
        let range_in_buffer = if let Some(frame) = start_on_frame {
            for ch in buffers.iter_mut() {
                ch[..frame].fill(0.0);
            }

            frame..frames
        } else {
            0..frames
        };

        // TODO: effects like pitch (doppler) shifting

        let (finished_playing, mut channels_filled) =
            self.copy_from_sample(buffers, range_in_buffer, looping);

        let Some(state) = self.loaded_sample_state.as_ref() else {
            return (true, 0);
        };

        if !self.declicker.is_settled() {
            self.declicker.process(
                buffers,
                0..frames,
                declick_values,
                state.gain,
                FadeType::EqualPower3dB,
            );
        } else if state.gain != 1.0 {
            for b in buffers[..channels_filled].iter_mut() {
                for s in b[..frames].iter_mut() {
                    *s *= state.gain;
                }
            }
        }

        if state.sample_mono_to_stereo {
            let (b0, b1) = buffers.split_first_mut().unwrap();
            b1[0][..frames].copy_from_slice(&b0[..frames]);

            channels_filled = 2;
        }

        (finished_playing, channels_filled)
    }

    /// Fill the buffer with raw data from the sample, starting from the
    /// current playhead. Then increment the playhead.
    ///
    /// Returns `true` if the sample has finished playing, and also
    /// returns the number of channels that were filled.
    fn copy_from_sample(
        &mut self,
        buffers: &mut [&mut [f32]],
        range_in_buffer: Range<usize>,
        looping: bool,
    ) -> (bool, usize) {
        let Some(state) = self.loaded_sample_state.as_mut() else {
            return (true, 0);
        };

        assert!(state.playhead <= state.sample_len_frames);

        let block_frames = range_in_buffer.end - range_in_buffer.start;
        let first_copy_frames = if state.playhead + block_frames as u64 > state.sample_len_frames {
            (state.sample_len_frames - state.playhead) as usize
        } else {
            block_frames
        };

        if first_copy_frames > 0 {
            state.sample.fill_buffers(
                buffers,
                range_in_buffer.start..range_in_buffer.start + first_copy_frames,
                state.playhead,
            );

            state.playhead += first_copy_frames as u64;
        }

        if first_copy_frames < block_frames {
            if looping {
                let mut frames_left = block_frames - first_copy_frames;

                while frames_left > 0 {
                    let copy_frames = (frames_left as u64).min(state.sample_len_frames) as usize;

                    state.sample.fill_buffers(
                        buffers,
                        range_in_buffer.start + first_copy_frames..range_in_buffer.end,
                        0,
                    );

                    state.playhead = copy_frames as u64;
                    state.num_times_looped_back += 1;

                    frames_left -= copy_frames;
                }
            } else {
                let n_channels = buffers.len().min(state.sample_num_channels.get());
                for b in buffers[..n_channels].iter_mut() {
                    b[range_in_buffer.start + first_copy_frames..range_in_buffer.end].fill(0.0);
                }

                return (true, n_channels);
            }
        }

        (false, buffers.len().min(state.sample_num_channels.get()))
    }

    fn currently_processing_sample(&self) -> bool {
        if self.params.sequence.is_none() || self.start_delay.is_some() {
            false
        } else {
            self.playback_state.is_playing()
                || (self.playback_state == PlaybackState::Pause && !self.declicker.is_settled())
        }
    }

    fn num_channels_filled(&self, num_out_channels: usize) -> usize {
        if let Some(state) = &self.loaded_sample_state {
            if state.sample_mono_to_stereo {
                2
            } else {
                state.sample_num_channels.get().min(num_out_channels)
            }
        } else {
            0
        }
    }

    fn stop(&mut self, declick_values: &DeclickValues, num_out_channels: usize) {
        if self.currently_processing_sample() {
            // Fade out the sample into a temporary look-ahead
            // buffer to declick.

            self.declicker.fade_to_0(declick_values);

            // Work around the borrow checker.
            if let Some(mut stop_declicker_buffers) = self.stop_declicker_buffers.take() {
                if self.num_active_stop_declickers < stop_declicker_buffers.num_instances() {
                    let declicker_i = self
                        .stop_declickers
                        .iter()
                        .enumerate()
                        .find_map(|(i, d)| if d.frames_left == 0 { Some(i) } else { None })
                        .unwrap();

                    let n_channels = self.num_channels_filled(num_out_channels);

                    let fade_out_frames = stop_declicker_buffers.frames();

                    self.stop_declickers[declicker_i].frames_left = fade_out_frames;
                    self.stop_declickers[declicker_i].channels = n_channels;

                    let mut tmp_buffers = stop_declicker_buffers
                        .get_mut(declicker_i, n_channels, fade_out_frames)
                        .unwrap();

                    self.process_internal(
                        &mut tmp_buffers,
                        fade_out_frames,
                        false,
                        declick_values,
                        None,
                    );

                    self.num_active_stop_declickers += 1;
                }

                self.stop_declicker_buffers = Some(stop_declicker_buffers);
            }
        }

        if let Some(state) = &mut self.loaded_sample_state {
            state.playhead = 0;
            state.num_times_looped_back = 0;
        }

        self.declicker.reset_to_1();
        self.start_delay = None;
    }

    fn load_sample(
        &mut self,
        sample: ArcGc<dyn SampleResource>,
        volume: Volume,
        repeat_mode: RepeatMode,
        num_out_channels: usize,
    ) {
        let mut gain = volume.amp_clamped(self.amp_epsilon);
        if gain > 0.99999 && gain < 1.00001 {
            gain = 1.0;
        }

        let sample_len_frames = sample.len_frames();
        let sample_num_channels = sample.num_channels();

        let sample_mono_to_stereo =
            self.config.mono_to_stereo && num_out_channels > 1 && sample_num_channels.get() == 1;

        self.loaded_sample_state = Some(LoadedSampleState {
            sample,
            sample_len_frames,
            sample_num_channels,
            sample_mono_to_stereo,
            gain,
            playhead: 0,
            repeat_mode,
            num_times_looped_back: 0,
        });
    }
}

impl AudioNodeProcessor for SamplerProcessor {
    fn process(
        &mut self,
        buffers: ProcBuffers,
        proc_info: &ProcInfo,
        mut events: NodeEventList,
    ) -> ProcessStatus {
        let mut sequence_changed = false;
        let mut playhead_changed = false;
        let mut playback_changed = false;

        events.for_each(|event| match event {
            NodeEventType::Custom(event) => {
                let Some(event) = event.downcast_mut::<SamplerEvent>() else {
                    return;
                };

                match event {
                    SamplerEvent::SetSequence { sequence } => {
                        // Return the old sequence to the main thread to be deallocated.
                        std::mem::swap(&mut self.params.sequence, sequence);

                        sequence_changed = true;
                    }
                    SamplerEvent::SetPlayback(playback) => {
                        if self.params.playback != *playback {
                            self.params.playback = *playback;
                            playback_changed = true;
                        }
                    }
                    SamplerEvent::SetPlayhead(playhead) => {
                        if self.params.playhead != *playhead {
                            self.params.playhead = *playhead;
                            playhead_changed = true;
                        }
                    }
                }
            }
            _ => {}
        });

        if sequence_changed || self.is_first_process {
            self.stop(proc_info.declick_values, buffers.outputs.len());

            self.loaded_sample_state = None;

            match &self.params.sequence {
                None => {
                    self.playback_state = PlaybackState::Stop;
                    self.shared_state.stopped.store(true, Ordering::Relaxed);
                }
                Some(SequenceType::SingleSample {
                    sample,
                    volume,
                    repeat_mode,
                }) => {
                    self.load_sample(
                        ArcGc::clone(sample),
                        *volume,
                        *repeat_mode,
                        buffers.outputs.len(),
                    );
                }
                _ => todo!(),
            }
        }

        if playhead_changed || self.is_first_process {
            let playhead_frames = self
                .params
                .playhead
                .playhead
                .as_frames(self.sample_rate as u32);

            if let Some(SequenceType::SingleSample { .. }) = &self.params.sequence {
                let state = self.loaded_sample_state.as_ref().unwrap();

                let playhead_frames = playhead_frames.min(state.sample_len_frames);

                if state.playhead != playhead_frames {
                    let playback_state = self.playback_state;

                    self.stop(proc_info.declick_values, buffers.outputs.len());

                    let state = self.loaded_sample_state.as_mut().unwrap();

                    state.playhead = playhead_frames;
                    self.playback_state = playback_state;

                    self.shared_state
                        .sequence_playhead_frames
                        .store(playhead_frames, Ordering::Relaxed);

                    if playhead_frames > 0 {
                        // Fade in to declick.
                        self.declicker.reset_to_0();

                        if self.playback_state.is_playing() {
                            self.declicker.fade_to_1(proc_info.declick_values);
                        }
                    }
                }
            }

            // If the sequence previously finished, restart it.
            playback_changed = true;
        }

        if playback_changed || self.is_first_process {
            match self.params.playback {
                PlaybackState::Stop => {
                    self.stop(proc_info.declick_values, buffers.outputs.len());

                    self.playback_state = PlaybackState::Stop;
                }
                PlaybackState::Pause => {
                    if self.playback_state.is_playing() {
                        self.playback_state = PlaybackState::Pause;

                        self.declicker.fade_to_0(proc_info.declick_values);

                        self.playback_pause_time_seconds = proc_info.clock_seconds.start;
                        self.playback_pause_time_frames = proc_info.clock_samples;
                    }
                }
                PlaybackState::Play { delay } => {
                    self.playback_state = PlaybackState::Play { delay: None };

                    // Crossfade with the previous sample.
                    if self.config.crossfade_on_restart && self.num_active_stop_declickers > 0 {
                        self.declicker.reset_to_0();
                        self.declicker.fade_to_1(proc_info.declick_values);
                    }

                    self.start_delay = delay.and_then(|delay| delay.elapsed_or_get(&proc_info));

                    if self.start_delay.is_none() {
                        self.playback_start_time_seconds = proc_info.clock_seconds.start;
                        self.playback_start_time_frames = proc_info.clock_samples;
                    }
                }
            }
        }

        self.is_first_process = false;

        self.shared_state.stopped.store(
            self.playback_state == PlaybackState::Stop,
            Ordering::Relaxed,
        );

        let start_on_frame = if let Some(delay) = self.start_delay {
            if let Some(frame) = delay.elapsed_on_frame(&proc_info, self.sample_rate as u32) {
                self.start_delay = None;

                self.playback_start_time_seconds = proc_info.clock_seconds.start;
                self.playback_start_time_frames = proc_info.clock_samples;

                Some(frame)
            } else {
                None
            }
        } else {
            None
        };

        let currently_processing_sample = self.currently_processing_sample();

        if !currently_processing_sample && self.num_active_stop_declickers == 0 {
            return ProcessStatus::ClearAllOutputs;
        }

        let mut num_filled_channels = 0;

        if currently_processing_sample {
            match &self.params.sequence {
                None => {}
                Some(SequenceType::SingleSample { .. }) => {
                    let sample_state = self.loaded_sample_state.as_ref().unwrap();
                    let looping = sample_state
                        .repeat_mode
                        .do_loop(sample_state.num_times_looped_back);

                    let (finished, n_channels) = self.process_internal(
                        buffers.outputs,
                        proc_info.frames,
                        looping,
                        proc_info.declick_values,
                        start_on_frame,
                    );

                    num_filled_channels = n_channels;

                    self.shared_state.sequence_playhead_frames.store(
                        self.loaded_sample_state.as_ref().unwrap().playhead,
                        Ordering::Relaxed,
                    );

                    if finished {
                        self.playback_state = PlaybackState::Stop;
                        self.shared_state.stopped.store(true, Ordering::Relaxed);
                    }
                }
                Some(SequenceType::Sequence {
                    sequence: _,
                    timing: _,
                }) => {
                    todo!()
                }
            }
        }

        for (i, out_buf) in buffers
            .outputs
            .iter_mut()
            .enumerate()
            .skip(num_filled_channels)
        {
            if !proc_info.out_silence_mask.is_channel_silent(i) {
                out_buf[..proc_info.frames].fill(0.0);
            }
        }

        if self.num_active_stop_declickers > 0 {
            let tmp_buffers = self.stop_declicker_buffers.as_ref().unwrap();
            let fade_out_frames = tmp_buffers.frames();

            for (declicker_i, declicker) in self.stop_declickers.iter_mut().enumerate() {
                if declicker.frames_left == 0 {
                    continue;
                }

                let tmp_buffers = tmp_buffers
                    .get(declicker_i, declicker.channels, fade_out_frames)
                    .unwrap();

                let copy_frames = proc_info.frames.min(declicker.frames_left);
                let start_frame = fade_out_frames - declicker.frames_left;

                for (out_buf, tmp_buf) in buffers.outputs.iter_mut().zip(tmp_buffers.iter()) {
                    for (os, &ts) in out_buf[..copy_frames]
                        .iter_mut()
                        .zip(tmp_buf[start_frame..start_frame + copy_frames].iter())
                    {
                        *os += ts;
                    }
                }

                declicker.frames_left -= copy_frames;
                if declicker.frames_left == 0 {
                    self.num_active_stop_declickers -= 1;
                }

                num_filled_channels = num_filled_channels.max(declicker.channels);
            }
        }

        let out_silence_mask = if num_filled_channels >= buffers.outputs.len() {
            SilenceMask::NONE_SILENT
        } else {
            let mut mask = SilenceMask::new_all_silent(buffers.outputs.len());
            for i in 0..num_filled_channels {
                mask.set_channel(i, false);
            }
            mask
        };

        ProcessStatus::OutputsModified { out_silence_mask }
    }

    fn new_stream(&mut self, stream_info: &StreamInfo) {
        if stream_info.sample_rate.get() as f64 != self.sample_rate {
            self.sample_rate = stream_info.sample_rate.get() as f64;

            self.stop_declicker_buffers = if self.config.num_declickers == 0 {
                None
            } else {
                Some(InstanceBuffer::<f32, MAX_OUT_CHANNELS>::new(
                    self.config.num_declickers as usize,
                    NonZeroUsize::new(self.config.channels.get().get() as usize).unwrap(),
                    stream_info.declick_frames.get() as usize,
                ))
            };

            // The sample rate has changed, meaning that the sample resources now have
            // the incorrect sample rate and the user must reload them.
            self.params.sequence = None;
            self.loaded_sample_state = None;
            self.playback_state = PlaybackState::Stop;
            self.shared_state.stopped.store(true, Ordering::Relaxed);
        }
    }
}

struct SharedState {
    sequence_playhead_frames: AtomicU64,
    stopped: AtomicBool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            sequence_playhead_frames: AtomicU64::new(0),
            stopped: AtomicBool::new(true),
        }
    }
}

struct LoadedSampleState {
    sample: ArcGc<dyn SampleResource>,
    sample_len_frames: u64,
    sample_num_channels: NonZeroUsize,
    sample_mono_to_stereo: bool,
    gain: f32,
    playhead: u64,
    repeat_mode: RepeatMode,
    num_times_looped_back: u64,
}

#[derive(Default, Clone, Copy)]
struct StopDeclickerState {
    frames_left: usize,
    channels: usize,
}
