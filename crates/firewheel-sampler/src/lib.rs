use crossbeam_utils::atomic::AtomicCell;
use firewheel_graph::FirewheelCtx;
use smallvec::SmallVec;
use std::{
    num::{NonZeroU32, NonZeroUsize},
    ops::Range,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    clock::EventDelay,
    dsp::{
        buffer::InstanceBuffer,
        decibel::normalized_volume_to_raw_gain,
        declick::{DeclickValues, Declicker},
    },
    node::{
        AudioNodeProcessor, NodeEventIter, NodeEventType, NodeHandle, NodeID, ProcInfo,
        ProcessStatus, RepeatMode,
    },
    sample_resource::SampleResource,
    SilenceMask,
};

pub const MAX_OUT_CHANNELS: usize = 8;
pub const DEFAULT_NUM_DECLICKERS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerConfig {
    /// The number of channels in this node.
    pub channels: ChannelCount,
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
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            channels: ChannelCount::STEREO,
            mono_to_stereo: true,
            num_declickers: DEFAULT_NUM_DECLICKERS as u32,
            crossfade_on_restart: true,
        }
    }
}

pub struct SamplerNode {
    shared_state: Arc<SharedState>,
    sample_rate: NonZeroU32,
    sample_rate_recip: f64,
    sample: Option<Arc<dyn SampleResource>>,
    normalized_volume: f32,
    repeat_mode: RepeatMode,
    handle: NodeHandle,
}

impl SamplerNode {
    pub const PARAM_PLAYHEAD_SECONDS: u32 = 0;
    pub const PARAM_PLAYHEAD_FRAMES: u32 = 0;

    pub fn new(config: SamplerConfig, cx: &mut FirewheelCtx) -> Self {
        assert_ne!(config.channels.get(), 0);

        let sample_rate = cx.stream_info().sample_rate;

        let stop_declicker_buffers = if config.num_declickers == 0 {
            None
        } else {
            Some(InstanceBuffer::new(
                config.num_declickers as usize,
                NonZeroUsize::new(config.channels.get() as usize).unwrap(),
                NonZeroUsize::new(cx.stream_info().declick_frames.get() as usize).unwrap(),
            ))
        };

        let shared_state = Arc::new(SharedState {
            playhead_frames: AtomicU64::new(0),
            status: AtomicCell::new(SamplerStatus {
                playback: PlaybackStatus::NoSample,
                latest_queued_event: LatestQueuedEvent::None,
            }),
        });

        let handle = cx.add_node(
            "sampler",
            ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: config.channels,
            },
            true,
            Box::new(SamplerProcessor {
                config,
                sample: None,
                sample_len_frames: 0,
                sample_num_channels: NonZeroUsize::MIN,
                sample_mono_to_stereo: false,
                gain: 0.0,
                playhead: 0,
                repeat_mode: RepeatMode::default(),
                num_times_looped_back: 0,
                declicker: Declicker::SettledAt0,
                paused: true,
                shared_state: Arc::clone(&shared_state),
                stop_declickers: smallvec::smallvec![StopDeclickerState::default(); config.num_declickers as usize],
                num_active_stop_declickers: 0,
                stop_declicker_buffers,
                sample_rate,
                has_begun_playing: false,
            }),
        );

        Self {
            shared_state,
            sample_rate: NonZeroU32::MIN,
            sample_rate_recip: 0.0,
            sample: None,
            normalized_volume: 1.0,
            repeat_mode: RepeatMode::PlayOnce,
            handle,
        }
    }

    /// Get the current playhead in units of frames (samples in a single channel
    /// of audio).
    pub fn playhead_frames(&self) -> u64 {
        self.shared_state.playhead_frames.load(Ordering::Relaxed)
    }

    /// Get the current playhead in units of seconds.
    pub fn playhead_seconds(&self) -> f64 {
        let playhead_frames = self.playhead_frames();
        let sample_rate_u64 = self.sample_rate.get() as u64;
        let whole_seconds = playhead_frames / sample_rate_u64;
        let fract_samples = playhead_frames % sample_rate_u64;

        whole_seconds as f64 + (fract_samples as f64 * self.sample_rate_recip)
    }

    /// Returns the current status.
    pub fn status(&self) -> SamplerStatus {
        self.shared_state.status.load()
    }

    pub fn set_playhead_seconds_event(&self, seconds: f64) -> NodeEventType {
        NodeEventType::F64Param {
            id: Self::PARAM_PLAYHEAD_SECONDS,
            value: seconds,
            smoothing: false,
        }
    }

    pub fn set_playhead_frames_event(&self, frames: u64) -> NodeEventType {
        NodeEventType::U64Param {
            id: Self::PARAM_PLAYHEAD_FRAMES,
            value: frames,
            smoothing: false,
        }
    }

    /// The ID of this node
    pub fn id(&self) -> NodeID {
        self.handle.id
    }

    /// Use the given sample with the given settings. This will stop any
    /// currently playing samples and reset the playhead to the beginning.
    /// Call `start_or_restart` to begin playback.
    ///
    /// If this node already has this sample with the given settings, then
    /// this will do nothing.
    pub fn set_sample(
        &mut self,
        replace_with_sample: Option<&Arc<dyn SampleResource>>,
        normalized_volume: f32,
        repeat_mode: RepeatMode,
        delay: EventDelay,
    ) {
        if let Some(new_sample) = replace_with_sample {
            if let Some(old_sample) = self.sample.as_ref() {
                if Arc::ptr_eq(old_sample, new_sample)
                    && self.normalized_volume == normalized_volume
                    && self.repeat_mode == repeat_mode
                {
                    return;
                }
            }

            self.sample = Some(Arc::clone(new_sample));
        } else if self.sample.is_none() {
            return;
        };

        self.normalized_volume = normalized_volume;
        self.repeat_mode = repeat_mode;
        let sample = Arc::clone(self.sample.as_ref().unwrap());

        self.handle.queue_event(
            NodeEventType::NewSample {
                sample,
                normalized_volume,
                repeat_mode,
            },
            delay,
        );

        self.set_latest_queued_event(LatestQueuedEvent::NewSampleEventQueued);
    }

    /// Discard any sample data loaded in this node. This will stop any
    /// currently playing samples and reset the playhead to the beginning.
    pub fn discard_sample(&mut self) {
        if self.sample.is_some() {
            self.sample = None;

            self.handle
                .queue_event(NodeEventType::DiscardData, EventDelay::Immediate);

            self.set_latest_queued_event(LatestQueuedEvent::DiscardEventQueued);
        }
    }

    /// Start/restart playback.
    pub fn start_or_restart(&mut self, delay: EventDelay) {
        if self.sample.is_none() {
            return;
        }

        self.handle
            .queue_event(NodeEventType::StartOrRestart, delay);

        self.set_latest_queued_event(LatestQueuedEvent::StartOrRestartEventQueued);
    }

    /// Pause playback.
    pub fn pause(&mut self, delay: EventDelay) {
        self.handle.queue_event(NodeEventType::Pause, delay);

        self.set_latest_queued_event(LatestQueuedEvent::PauseEventQueued);
    }

    /// Resume playback.
    pub fn resume(&mut self, delay: EventDelay) {
        self.handle.queue_event(NodeEventType::Resume, delay);

        self.set_latest_queued_event(LatestQueuedEvent::ResumeEventQueued);
    }

    /// Stop playback and reset the playhead back to the beginning.
    ///
    /// This will also discard any pending delayed events.
    pub fn stop(&mut self) {
        self.handle
            .queue_event(NodeEventType::Stop, EventDelay::Immediate);

        self.set_latest_queued_event(LatestQueuedEvent::StopEventQueued);
    }

    /// Set the playhead to the given time in seconds.
    pub fn set_playhead_seconds(&mut self, seconds: f64, delay: EventDelay) {
        self.handle.queue_event(
            NodeEventType::F64Param {
                id: SamplerNode::PARAM_PLAYHEAD_SECONDS,
                value: seconds,
                smoothing: false,
            },
            delay,
        );
    }

    /// Set the playhead to the given time in frames (samples in a single
    /// channel of audio, not to be confused with video frames).
    pub fn set_playhead_frames(&mut self, frames: u64, delay: EventDelay) {
        self.handle.queue_event(
            NodeEventType::U64Param {
                id: SamplerNode::PARAM_PLAYHEAD_FRAMES,
                value: frames,
                smoothing: false,
            },
            delay,
        );
    }

    /// The current sample loaded into this node.
    pub fn sample(&self) -> Option<&Arc<dyn SampleResource>> {
        self.sample.as_ref()
    }

    pub fn normalized_volume(&self) -> f32 {
        self.normalized_volume
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    fn set_latest_queued_event(&mut self, latest_queued_event: LatestQueuedEvent) {
        let playback = self.shared_state.status.load().playback;
        self.shared_state.status.store(SamplerStatus {
            playback,
            latest_queued_event,
        });
    }
}

pub struct SamplerProcessor {
    config: SamplerConfig,
    sample: Option<Arc<dyn SampleResource>>,
    sample_len_frames: u64,
    sample_num_channels: NonZeroUsize,
    sample_mono_to_stereo: bool,
    gain: f32,
    playhead: u64,
    repeat_mode: RepeatMode,
    num_times_looped_back: u64,
    declicker: Declicker,
    paused: bool,
    shared_state: Arc<SharedState>,
    stop_declicker_buffers: Option<InstanceBuffer<f32, MAX_OUT_CHANNELS>>,
    stop_declickers: SmallVec<[StopDeclickerState; DEFAULT_NUM_DECLICKERS]>,
    num_active_stop_declickers: usize,
    sample_rate: NonZeroU32,
    has_begun_playing: bool,
}

impl SamplerProcessor {
    fn process_internal(
        &mut self,
        buffers: &mut [&mut [f32]],
        frames: usize,
        looping: bool,
        declick_values: &DeclickValues,
    ) {
        // TODO: effects like doppler shifting

        self.copy_from_sample(buffers, 0..frames, looping);

        if !self.declicker.is_settled() {
            self.declicker
                .process(buffers, 0..frames, declick_values, self.gain);
        } else if self.gain != 1.0 {
            let n_channels = buffers.len().min(self.sample_num_channels.get());
            for b in buffers[..n_channels].iter_mut() {
                for s in b[..frames].iter_mut() {
                    *s *= self.gain;
                }
            }
        }

        if self.sample_mono_to_stereo {
            let (b0, b1) = buffers.split_first_mut().unwrap();
            b1[0][..frames].copy_from_slice(&b0[..frames]);
        }
    }

    /// Fill the buffer with raw data from the sample, starting from the
    /// current playhead. Then increment the playhead.
    fn copy_from_sample(
        &mut self,
        buffers: &mut [&mut [f32]],
        range_in_buffer: Range<usize>,
        looping: bool,
    ) {
        let Some(sample) = self.sample.as_ref() else {
            return;
        };

        assert!(self.playhead <= self.sample_len_frames);

        let block_frames = range_in_buffer.end - range_in_buffer.start;
        let first_copy_frames = if self.playhead + block_frames as u64 > self.sample_len_frames {
            (self.sample_len_frames - self.playhead) as usize
        } else {
            block_frames
        };

        if first_copy_frames > 0 {
            sample.fill_buffers(
                buffers,
                range_in_buffer.start..range_in_buffer.start + first_copy_frames,
                self.playhead,
            );

            self.playhead += first_copy_frames as u64;
        }

        if first_copy_frames < block_frames {
            if looping {
                let second_copy_frames = block_frames - first_copy_frames;

                sample.fill_buffers(
                    buffers,
                    range_in_buffer.start + first_copy_frames..range_in_buffer.end,
                    0,
                );

                self.playhead = second_copy_frames as u64;
                self.num_times_looped_back += 1;
            } else {
                let n_channels = buffers.len().min(self.sample_num_channels.get());
                for b in buffers[..n_channels].iter_mut() {
                    b[range_in_buffer.start + first_copy_frames..range_in_buffer.end].fill(0.0);
                }

                self.paused = true;
            }
        }
    }

    fn current_sample_is_processing(&self) -> bool {
        self.sample.is_some() && (!self.paused || !self.declicker.is_settled())
    }

    fn num_channels_filled(&self, num_out_channels: usize) -> usize {
        if self.sample_mono_to_stereo {
            2
        } else {
            self.sample_num_channels.get().min(num_out_channels)
        }
    }

    fn stop(&mut self, declick_values: &DeclickValues, num_out_channels: usize) {
        if self.current_sample_is_processing() {
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

                    let fade_out_frames = stop_declicker_buffers.frames().get();

                    self.stop_declickers[declicker_i].frames_left = fade_out_frames;
                    self.stop_declickers[declicker_i].channels = n_channels;

                    let mut tmp_buffers = stop_declicker_buffers
                        .get_mut(declicker_i, n_channels, fade_out_frames)
                        .unwrap();

                    self.process_internal(&mut tmp_buffers, fade_out_frames, false, declick_values);

                    self.num_active_stop_declickers += 1;
                }

                self.stop_declicker_buffers = Some(stop_declicker_buffers);
            }
        }

        self.playhead = 0;
        self.paused = true;
        self.num_times_looped_back = 0;
        self.declicker.reset_to_1();
        self.has_begun_playing = false;
    }

    fn set_playhead(
        &mut self,
        playhead_frames: u64,
        declick_values: &DeclickValues,
        num_out_channels: usize,
    ) {
        if self.playhead == playhead_frames {
            return;
        }

        let paused = self.paused;

        self.stop(declick_values, num_out_channels);

        self.playhead = playhead_frames;
        self.paused = paused;

        if playhead_frames > 0 {
            // Fade in to declick.
            self.declicker.reset_to_0();

            if !self.paused {
                self.declicker.fade_to_1(declick_values);
            }
        }
    }

    fn update_playback_status(&mut self) {
        let status = self.shared_state.status.load();

        let new_playback_status = if self.sample.is_none() {
            PlaybackStatus::NoSample
        } else if !self.has_begun_playing {
            PlaybackStatus::NotStartedYet
        } else if self.playhead >= self.sample_len_frames
            && !self.repeat_mode.do_loop(self.num_times_looped_back)
        {
            PlaybackStatus::Finished
        } else if self.paused {
            PlaybackStatus::Paused
        } else if self.repeat_mode == RepeatMode::RepeatEndlessly {
            PlaybackStatus::PlayingEndlessly
        } else {
            PlaybackStatus::Playing
        };

        if status.playback != new_playback_status {
            self.shared_state.status.store(SamplerStatus {
                playback: new_playback_status,
                latest_queued_event: status.latest_queued_event,
            });
        }
    }
}

#[derive(Default, Clone, Copy)]
struct StopDeclickerState {
    frames_left: usize,
    channels: usize,
}

impl AudioNodeProcessor for SamplerProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let mut playback_status_changed = false;

        for msg in events {
            match msg {
                NodeEventType::Pause => {
                    let mut status = self.shared_state.status.load();
                    if status.latest_queued_event == LatestQueuedEvent::PauseEventQueued {
                        status.latest_queued_event = LatestQueuedEvent::None;
                        self.shared_state.status.store(status);
                    }

                    if self.sample.is_some() && !self.paused {
                        self.paused = true;
                        self.declicker.fade_to_0(proc_info.declick_values);
                    }

                    playback_status_changed = true;
                }
                NodeEventType::Resume => {
                    let mut status = self.shared_state.status.load();
                    if status.latest_queued_event == LatestQueuedEvent::ResumeEventQueued {
                        status.latest_queued_event = LatestQueuedEvent::None;
                        self.shared_state.status.store(status);
                    }

                    if self.sample.is_some() && self.paused && self.has_begun_playing {
                        self.paused = false;
                        self.declicker.fade_to_1(proc_info.declick_values);
                    }

                    playback_status_changed = true;
                }
                NodeEventType::Stop => {
                    let mut status = self.shared_state.status.load();
                    if status.latest_queued_event == LatestQueuedEvent::StopEventQueued {
                        status.latest_queued_event = LatestQueuedEvent::None;
                        self.shared_state.status.store(status);
                    }

                    self.stop(proc_info.declick_values, outputs.len());

                    playback_status_changed = true;
                }
                NodeEventType::StartOrRestart => {
                    let mut status = self.shared_state.status.load();
                    if status.latest_queued_event == LatestQueuedEvent::StartOrRestartEventQueued {
                        status.latest_queued_event = LatestQueuedEvent::None;
                        self.shared_state.status.store(status);
                    }

                    if self.sample.is_none() {
                        continue;
                    }

                    self.stop(proc_info.declick_values, outputs.len());

                    self.paused = false;

                    // Crossfade with the previous sample.
                    if self.config.crossfade_on_restart && self.num_active_stop_declickers > 0 {
                        self.declicker.reset_to_0();
                        self.declicker.fade_to_1(proc_info.declick_values);
                    }

                    self.has_begun_playing = true;
                    playback_status_changed = true;
                }
                NodeEventType::DiscardData => {
                    let mut status = self.shared_state.status.load();
                    if status.latest_queued_event == LatestQueuedEvent::DiscardEventQueued {
                        status.latest_queued_event = LatestQueuedEvent::None;
                        self.shared_state.status.store(status);
                    }

                    self.stop(proc_info.declick_values, outputs.len());

                    self.sample = None;

                    playback_status_changed = true;
                }
                NodeEventType::F64Param { id, value, .. } => {
                    if *id != SamplerNode::PARAM_PLAYHEAD_SECONDS {
                        continue;
                    }

                    let playhead_frames = if *value <= 0.0 {
                        0
                    } else {
                        (value.floor() as u64 * self.sample_rate.get() as u64)
                            + (value.fract() * self.sample_rate.get() as f64).round() as u64
                    };

                    self.set_playhead(playhead_frames, proc_info.declick_values, outputs.len());

                    playback_status_changed = true;
                }
                NodeEventType::U64Param { id, value, .. } => {
                    if *id != SamplerNode::PARAM_PLAYHEAD_FRAMES {
                        continue;
                    }

                    self.set_playhead(*value, proc_info.declick_values, outputs.len());

                    playback_status_changed = true;
                }
                NodeEventType::NewSample {
                    sample,
                    normalized_volume,
                    repeat_mode,
                } => {
                    self.stop(proc_info.declick_values, outputs.len());

                    self.gain = normalized_volume_to_raw_gain(*normalized_volume);
                    if self.gain < 0.00001 {
                        self.gain = 0.0;
                    }
                    if self.gain > 0.99999 && self.gain < 1.00001 {
                        self.gain = 1.0;
                    }

                    self.repeat_mode = *repeat_mode;
                    self.sample_len_frames = sample.len_frames();
                    self.sample_num_channels = sample.num_channels();
                    self.sample = Some(Arc::clone(sample));
                    self.sample_mono_to_stereo = self.config.mono_to_stereo
                        && outputs.len() > 1
                        && self.sample_num_channels.get() == 1;

                    let mut status = self.shared_state.status.load();
                    if status.latest_queued_event == LatestQueuedEvent::NewSampleEventQueued {
                        status.latest_queued_event = LatestQueuedEvent::None;
                        self.shared_state.status.store(status);
                    }

                    playback_status_changed = true;
                }
                _ => {}
            }
        }

        let current_sample_is_processing = self.current_sample_is_processing();

        if !current_sample_is_processing && self.num_active_stop_declickers == 0 {
            if playback_status_changed {
                self.update_playback_status();
            }

            return ProcessStatus::ClearAllOutputs;
        }

        let mut num_filled_channels = 0;

        if current_sample_is_processing {
            let looping = self.repeat_mode.do_loop(self.num_times_looped_back);

            self.process_internal(outputs, proc_info.frames, looping, proc_info.declick_values);

            num_filled_channels = self.num_channels_filled(outputs.len());
        }

        self.update_playback_status();

        for (i, out_buf) in outputs.iter_mut().enumerate().skip(num_filled_channels) {
            if !proc_info.out_silence_mask.is_channel_silent(i) {
                out_buf[..proc_info.frames].fill(0.0);
            }
        }

        if self.num_active_stop_declickers > 0 {
            let tmp_buffers = self.stop_declicker_buffers.as_ref().unwrap();
            let fade_out_frames = tmp_buffers.frames().get();

            for (declicker_i, declicker) in self.stop_declickers.iter_mut().enumerate() {
                if declicker.frames_left == 0 {
                    continue;
                }

                let tmp_buffers = tmp_buffers
                    .get(declicker_i, declicker.channels, fade_out_frames)
                    .unwrap();

                let copy_frames = proc_info.frames.min(declicker.frames_left);
                let start_frame = fade_out_frames - declicker.frames_left;

                for (out_buf, tmp_buf) in outputs.iter_mut().zip(tmp_buffers.iter()) {
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

        let out_silence_mask = if num_filled_channels >= outputs.len() {
            SilenceMask::NONE_SILENT
        } else {
            let mut mask = SilenceMask::new_all_silent(outputs.len());
            for i in 0..num_filled_channels {
                mask.set_channel(i, false);
            }
            mask
        };

        ProcessStatus::OutputsModified { out_silence_mask }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerStatus {
    pub playback: PlaybackStatus,
    pub latest_queued_event: LatestQueuedEvent,
}

impl SamplerStatus {
    pub fn is_playing(&self) -> bool {
        match self.latest_queued_event {
            LatestQueuedEvent::NewSampleEventQueued
            | LatestQueuedEvent::StopEventQueued
            | LatestQueuedEvent::DiscardEventQueued
            | LatestQueuedEvent::PauseEventQueued => false,
            LatestQueuedEvent::StartOrRestartEventQueued => true,
            LatestQueuedEvent::ResumeEventQueued => {
                self.playback != PlaybackStatus::Finished
                    && self.playback != PlaybackStatus::NoSample
                    && self.playback != PlaybackStatus::NotStartedYet
            }
            _ => match self.playback {
                PlaybackStatus::Playing | PlaybackStatus::PlayingEndlessly => true,
                _ => false,
            },
        }
    }

    pub fn finished(&self) -> bool {
        self.playback == PlaybackStatus::Finished
    }

    /// Returns a `score` of how good of a candidate this node is to
    /// be given new work. The higher the score, the better the candidate
    /// for the work.
    ///
    /// This is useful when assigning work to a pool of sampler nodes.
    /// If all nodes in the pool are currently working, then the one
    /// with the highest score is the "oldest" one.
    pub fn new_work_score(&self, playhead_frames: u64) -> u64 {
        match self.latest_queued_event {
            LatestQueuedEvent::NewSampleEventQueued => u64::MAX - 4,
            LatestQueuedEvent::StopEventQueued => u64::MAX - 3,
            LatestQueuedEvent::DiscardEventQueued => u64::MAX - 2,
            LatestQueuedEvent::StartOrRestartEventQueued => playhead_frames,
            _ => match self.playback {
                PlaybackStatus::NoSample => u64::MAX,
                PlaybackStatus::Finished => u64::MAX - 1,
                PlaybackStatus::NotStartedYet => u64::MAX - 3,
                _ => playhead_frames,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    /// The sampler currently has no sample loaded.
    NoSample,
    /// The sampler has not currently started playing its sample yet.
    NotStartedYet,
    /// The sampler is currently paused.
    Paused,
    /// The sampler is currently playing a sample.
    Playing,
    /// The sampler is currently playing a sample on an endless loop.
    PlayingEndlessly,
    /// The sampler has finished playing the sample.
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatestQueuedEvent {
    None,
    NewSampleEventQueued,
    StopEventQueued,
    StartOrRestartEventQueued,
    DiscardEventQueued,
    PauseEventQueued,
    ResumeEventQueued,
}

struct SharedState {
    playhead_frames: AtomicU64,
    status: AtomicCell<SamplerStatus>,
}
