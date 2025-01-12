use crossbeam_utils::atomic::AtomicCell;
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
    channel_config::{ChannelConfig, ChannelCount, NonZeroChannelCount},
    clock::EventDelay,
    dsp::{
        buffer::InstanceBuffer,
        decibel::normalized_volume_to_raw_gain,
        declick::{DeclickValues, Declicker},
    },
    event::{NodeEventList, NodeEventType, SequenceCommand},
    node::{AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    sample_resource::SampleResource,
    SilenceMask, StreamInfo,
};

pub const MAX_OUT_CHANNELS: usize = 8;
pub const DEFAULT_NUM_DECLICKERS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            channels: NonZeroChannelCount::STEREO,
            mono_to_stereo: true,
            num_declickers: DEFAULT_NUM_DECLICKERS as u32,
            crossfade_on_restart: true,
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

impl PlaybackState {
    pub fn is_playing(&self) -> bool {
        self == &PlaybackState::Playing
    }
}

struct SharedState {
    playhead_frames: AtomicU64,
    playback_state: AtomicCell<PlaybackState>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            playhead_frames: AtomicU64::new(0),
            playback_state: AtomicCell::new(PlaybackState::Stopped),
        }
    }
}

/// The state of a sampler node.
#[derive(Clone)]
pub struct SamplerState {
    /// The current sequence loaded into the sampler.
    pub sequence: Option<SequenceType>,
    /// The configuration of this sampler node.
    ///
    /// This cannot be changed once the node is added to the audio graph.
    pub config: SamplerConfig,
    shared_state: Arc<SharedState>,
}

impl Default for SamplerState {
    fn default() -> Self {
        Self::new(None, SamplerConfig::default())
    }
}

impl SamplerState {
    pub fn new(sequence: Option<SequenceType>, config: SamplerConfig) -> Self {
        Self {
            sequence,
            config,
            shared_state: Arc::new(SharedState::default()),
        }
    }

    /// Set the sequence to a single sample.
    ///
    /// * `sample` - The sample resource to use.
    /// * `normalized_volume` - The volume to play the sample at, where `0.0` is silence and
    /// `1.0` is unity gain. Note that this node does not support changing the volume while
    /// playing. Instead, use a node like the volume node for that.
    /// * `repeat_mode` - How many times a sample/sequence should be repeated for each
    /// `StartOrRestart` command.
    pub fn set_sample(
        &mut self,
        sample: Arc<dyn SampleResource>,
        normalized_volume: f32,
        repeat_mode: RepeatMode,
    ) {
        self.sequence = Some(SequenceType::SingleSample {
            sample,
            normalized_volume,
            repeat_mode,
        });
    }

    /// Clear the sample resource from this state.
    pub fn clear_sample(&mut self) {
        self.sequence = None;
    }

    /// Get the current position of the playhead in seconds.
    ///
    /// Only returns `Some` when the sequence is [`SequenceType::SingleSample`].
    ///
    /// * `sample_rate` - The sample rate of the current audio stream.
    pub fn playhead_seconds(&self, sample_rate: NonZeroU32) -> Option<f64> {
        if let Some(SequenceType::SingleSample { .. }) = &self.sequence {
            let frames = self.shared_state.playhead_frames.load(Ordering::Relaxed);

            Some(frames as f64 / sample_rate.get() as f64)
        } else {
            None
        }
    }

    /// Get the current position of the playhead in units of samples (of
    /// a single channel of audio).
    ///
    /// Only returns `Some` when the sequence is [`SequenceType::SingleSample`].
    pub fn playhead_samples(&self) -> Option<u64> {
        if let Some(SequenceType::SingleSample { .. }) = &self.sequence {
            Some(self.shared_state.playhead_frames.load(Ordering::Relaxed))
        } else {
            None
        }
    }

    /// The current playback state.
    pub fn playback_state(&self) -> PlaybackState {
        self.shared_state.playback_state.load()
    }

    /// A score of how suitible this node is to start new work (Play a new sample). The
    /// higher the score, the better the candidate.
    pub fn worker_score(&self) -> u64 {
        if self.sequence.is_some() {
            match self.playback_state() {
                PlaybackState::Stopped => u64::MAX - 1,
                PlaybackState::Paused => u64::MAX - 2,
                PlaybackState::Playing => {
                    // The older the sample is, the better it is as a candidate to steal
                    // work from.
                    self.playhead_samples().unwrap_or(0)
                }
            }
        } else {
            u64::MAX
        }
    }

    /// Return an event type to sync the new sequence to the processor.
    ///
    /// * `start_immediately` - If `true`, then the new sequence will be started
    /// immediately when the processor receives the event.
    pub fn sync_sequence_event(&self, start_immediately: bool) -> NodeEventType {
        if start_immediately {
            self._flag_playback_state(if self.sequence.is_some() {
                PlaybackState::Playing
            } else {
                PlaybackState::Stopped
            });
        }

        SamplerEvent::SetSequence {
            sequence: self.sequence.clone(),
            start_immediately,
        }
        .into()
    }

    /// Return an event type to start/restart the current sequence.
    ///
    /// * `delay` - The exact moment when the sequence should start.
    pub fn start_or_restart_event(&self, delay: EventDelay) -> NodeEventType {
        self._flag_playback_state(if self.sequence.is_some() {
            PlaybackState::Playing
        } else {
            PlaybackState::Stopped
        });

        NodeEventType::SequenceCommand(SequenceCommand::StartOrRestart { delay })
    }

    /// Return an event type to pause the current sequence.
    pub fn pause_event(&self) -> NodeEventType {
        self._flag_playback_state(PlaybackState::Paused);

        NodeEventType::SequenceCommand(SequenceCommand::Pause)
    }

    /// Return an event type to resume the current sequence.
    pub fn resume_event(&self) -> NodeEventType {
        self._flag_playback_state(if self.sequence.is_some() {
            PlaybackState::Playing
        } else {
            PlaybackState::Stopped
        });

        NodeEventType::SequenceCommand(SequenceCommand::Resume)
    }

    /// Return an event type to stop the current sequence.
    pub fn stop_event(&self) -> NodeEventType {
        self._flag_playback_state(PlaybackState::Stopped);

        NodeEventType::SequenceCommand(SequenceCommand::Stop)
    }

    /// Return an event type to set the position of the playhead in seconds.
    ///
    /// This only has an effect when the sequence is [`SequenceType::SingleSample`].
    pub fn set_playhead_event(&self, seconds: f64) -> NodeEventType {
        SamplerEvent::SetPlayheadSeconds(seconds).into()
    }

    /// Return an event type to set the position of the playhead in units of
    /// samples (of a single channel of audio).
    ///
    /// This only has an effect when the sequence is [`SequenceType::SingleSample`].
    pub fn set_playhead_samples_event(&self, samples: u64) -> NodeEventType {
        SamplerEvent::SetPlayheadSamples(samples).into()
    }

    /// Manually mark the playback state of this node. This can be used to account
    /// for the delay between when creating a [`SamplerEvent`] and when the processor
    /// receives the event when using [`SamplerState::worker_score`].
    ///
    /// Note, if you use the methods on this struct to construct the events, then
    /// this is automatically done for you.
    pub fn _flag_playback_state(&self, state: PlaybackState) {
        self.shared_state.playback_state.store(state);
    }
}

#[derive(Clone)]
pub enum SamplerEvent {
    /// Set the sampler state. This will stop any currently playing sequence.
    SetSequence {
        sequence: Option<SequenceType>,
        /// If `true`, then the new sequence will be started immediately.
        start_immediately: bool,
    },
    /// Set the position of the playhead in seconds.
    SetPlayheadSeconds(f64),
    /// Set the position of the playhead in units of samples (of a single channel
    /// of audio).
    SetPlayheadSamples(u64),
}

impl Into<NodeEventType> for SamplerEvent {
    fn into(self) -> NodeEventType {
        NodeEventType::Custom(Box::new(self))
    }
}

/// The current sequence loaded into the sampler.
#[derive(Clone)]
pub enum SequenceType {
    SingleSample {
        /// The sample resource to use.
        sample: Arc<dyn SampleResource>,
        /// The volume to play the sample at, where `0.0` is silence and `1.0`
        /// is unity gain.
        ///
        /// Note that this node does not support changing the volume while
        /// playing. Instead, use a node like the volume node for that.
        normalized_volume: f32,
        /// How many times a sample/sequence should be repeated for each `StartOrRestart` command.
        repeat_mode: RepeatMode,
        // TODO: Pitch
    },
    Sequence {
        sequence: Vec<SequenceEvent>,
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

#[derive(Clone)]
pub struct SequenceEvent {
    pub event: SequenceEventType,
    /// The amount of time from the start of the sequence that this event should occur.
    ///
    /// If the timing is set to [`SequenceTiming::Seconds`], then this is in units of seconds. If
    /// the timing is set to [`SequenceTiming::Samples`], then this is in units of samples (of a
    /// single channel of audio).
    pub offset_from_start: f64,
}

#[derive(Clone)]
pub enum SequenceEventType {
    PlaySample {
        /// The sample resource to use.
        sample: Arc<dyn SampleResource>,
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

impl AudioNodeConstructor for SamplerState {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "sampler",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: self.config.channels.get(),
            },
            uses_events: true,
        }
    }

    fn processor(&self, stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        let stop_declicker_buffers = if self.config.num_declickers == 0 {
            None
        } else {
            Some(InstanceBuffer::<f32, MAX_OUT_CHANNELS>::new(
                self.config.num_declickers as usize,
                NonZeroUsize::new(self.config.channels.get().get() as usize).unwrap(),
                stream_info.declick_frames.get() as usize,
            ))
        };

        let mut sampler = Box::new(SamplerProcessor {
            config: self.config.clone(),
            sequence: None,
            shared_state: Arc::clone(&self.shared_state),
            loaded_sample_state: None,
            declicker: Declicker::SettledAt1,
            playback_state: PlaybackState::Stopped,
            stop_declicker_buffers,
            stop_declickers: smallvec::smallvec![StopDeclickerState::default(); self.config.num_declickers as usize],
            num_active_stop_declickers: 0,
            playback_start_time_seconds: 0.0,
            playback_pause_time_seconds: 0.0,
            playback_start_time_frames: 0,
            playback_pause_time_frames: 0,
            sample_rate: stream_info.sample_rate.get() as f64,
        });

        sampler.set_sequence(
            &mut self.sequence.clone(),
            self.config.channels.get().get() as usize,
        );

        sampler
    }
}

pub struct SamplerProcessor {
    config: SamplerConfig,
    sequence: Option<SequenceType>,
    shared_state: Arc<SharedState>,

    loaded_sample_state: Option<LoadedSampleState>,

    declicker: Declicker,
    playback_state: PlaybackState,

    stop_declicker_buffers: Option<InstanceBuffer<f32, MAX_OUT_CHANNELS>>,
    stop_declickers: SmallVec<[StopDeclickerState; DEFAULT_NUM_DECLICKERS]>,
    num_active_stop_declickers: usize,

    playback_start_time_seconds: f64,
    playback_pause_time_seconds: f64,
    playback_start_time_frames: u64,
    playback_pause_time_frames: u64,

    sample_rate: f64,
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
    ) -> (bool, usize) {
        // TODO: effects like pitch (doppler) shifting

        let (finished_playing, mut channels_filled) =
            self.copy_from_sample(buffers, 0..frames, looping);

        let Some(state) = self.loaded_sample_state.as_ref() else {
            return (true, 0);
        };

        if !self.declicker.is_settled() {
            self.declicker
                .process(buffers, 0..frames, declick_values, state.gain);
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
                let second_copy_frames = block_frames - first_copy_frames;

                state.sample.fill_buffers(
                    buffers,
                    range_in_buffer.start + first_copy_frames..range_in_buffer.end,
                    0,
                );

                state.playhead = second_copy_frames as u64;
                state.num_times_looped_back += 1;
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
        if self.sequence.is_none() {
            false
        } else {
            self.playback_state == PlaybackState::Playing
                || (self.playback_state == PlaybackState::Paused && !self.declicker.is_settled())
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

                    self.process_internal(&mut tmp_buffers, fade_out_frames, false, declick_values);

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
    }

    fn set_playhead(
        &mut self,
        playhead_frames: u64,
        declick_values: &DeclickValues,
        num_out_channels: usize,
    ) {
        if let Some(SequenceType::SingleSample { .. }) = &self.sequence {
            let state = self.loaded_sample_state.as_ref().unwrap();

            let playhead_frames = playhead_frames.min(state.sample_len_frames);

            if state.playhead == playhead_frames {
                return;
            }

            let playback_state = self.playback_state;

            self.stop(declick_values, num_out_channels);

            let state = self.loaded_sample_state.as_mut().unwrap();

            state.playhead = playhead_frames;
            self.playback_state = playback_state;

            self.shared_state
                .playhead_frames
                .store(playhead_frames, Ordering::Relaxed);

            if playhead_frames > 0 {
                // Fade in to declick.
                self.declicker.reset_to_0();

                if self.playback_state == PlaybackState::Playing {
                    self.declicker.fade_to_1(declick_values);
                }
            }
        }
    }

    fn set_sequence(&mut self, sequence: &mut Option<SequenceType>, num_out_channels: usize) {
        // Return the old sequence to the main thread to be deallocated.
        std::mem::swap(&mut self.sequence, sequence);

        self.loaded_sample_state = None;

        match &self.sequence {
            None => {
                self.playback_state = PlaybackState::Stopped;
                self.shared_state
                    .playback_state
                    .store(PlaybackState::Stopped);
            }
            Some(SequenceType::SingleSample {
                sample,
                normalized_volume,
                repeat_mode,
            }) => {
                self.load_sample(
                    Arc::clone(sample),
                    *normalized_volume,
                    *repeat_mode,
                    num_out_channels,
                );
            }
            _ => {}
        }
    }

    fn load_sample(
        &mut self,
        sample: Arc<dyn SampleResource>,
        normalized_volume: f32,
        repeat_mode: RepeatMode,
        num_out_channels: usize,
    ) {
        let mut gain = normalized_volume_to_raw_gain(normalized_volume);
        if gain < 0.00001 {
            gain = 0.0;
        }
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
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        events.for_each(|event| match event {
            NodeEventType::SequenceCommand(command) => {
                if self.sequence.is_none() {
                    self.shared_state
                        .playback_state
                        .store(PlaybackState::Stopped);

                    return;
                }

                match command {
                    SequenceCommand::StartOrRestart { delay } => {
                        self.stop(proc_info.declick_values, outputs.len());

                        self.playback_state = PlaybackState::Playing;

                        // Crossfade with the previous sample.
                        if self.config.crossfade_on_restart && self.num_active_stop_declickers > 0 {
                            self.declicker.reset_to_0();
                            self.declicker.fade_to_1(proc_info.declick_values);
                        }

                        if *delay == EventDelay::Immediate {
                            self.playback_start_time_seconds = proc_info.clock_seconds.start.0;
                            self.playback_start_time_frames = proc_info.clock_samples.0;
                        } else {
                            todo!()
                        }
                    }
                    SequenceCommand::Pause => {
                        if self.playback_state == PlaybackState::Playing {
                            self.playback_state = PlaybackState::Paused;

                            self.declicker.fade_to_0(proc_info.declick_values);

                            self.playback_pause_time_seconds = proc_info.clock_seconds.start.0;
                            self.playback_pause_time_frames = proc_info.clock_samples.0;
                        }
                    }
                    SequenceCommand::Resume => {
                        if self.playback_state == PlaybackState::Paused {
                            self.playback_state = PlaybackState::Playing;

                            self.declicker.fade_to_1(proc_info.declick_values);

                            self.playback_start_time_seconds +=
                                proc_info.clock_seconds.start.0 - self.playback_pause_time_seconds;
                            self.playback_start_time_frames +=
                                proc_info.clock_samples.0 - self.playback_pause_time_frames;
                        }
                    }
                    SequenceCommand::Stop => {
                        self.stop(proc_info.declick_values, outputs.len());

                        self.playback_state = PlaybackState::Stopped;
                    }
                }

                self.shared_state.playback_state.store(self.playback_state);
            }
            NodeEventType::Custom(event) => {
                let Some(event) = event.downcast_mut::<SamplerEvent>() else {
                    return;
                };

                match event {
                    SamplerEvent::SetSequence {
                        sequence,
                        start_immediately,
                    } => {
                        self.stop(proc_info.declick_values, outputs.len());

                        self.set_sequence(sequence, outputs.len());

                        if self.sequence.is_none() {
                            return;
                        }

                        if *start_immediately {
                            self.playback_state = PlaybackState::Playing;
                            self.shared_state
                                .playback_state
                                .store(PlaybackState::Playing);

                            // Crossfade with the previous sample.
                            if self.config.crossfade_on_restart
                                && self.num_active_stop_declickers > 0
                            {
                                self.declicker.reset_to_0();
                                self.declicker.fade_to_1(proc_info.declick_values);
                            }

                            self.playback_start_time_seconds = proc_info.clock_seconds.start.0;
                            self.playback_start_time_frames = proc_info.clock_samples.0;
                        } else {
                            self.shared_state
                                .playback_state
                                .store(PlaybackState::Stopped);
                        }
                    }
                    SamplerEvent::SetPlayheadSeconds(seconds) => {
                        let playhead_frames = if *seconds <= 0.0 {
                            0
                        } else {
                            (seconds.floor() as u64 * self.sample_rate as u64)
                                + (seconds.fract() * self.sample_rate).round() as u64
                        };

                        self.set_playhead(playhead_frames, proc_info.declick_values, outputs.len());
                    }
                    SamplerEvent::SetPlayheadSamples(playhead_frames) => {
                        self.set_playhead(
                            *playhead_frames,
                            proc_info.declick_values,
                            outputs.len(),
                        );
                    }
                }
            }
            _ => {}
        });

        let currently_processing_sample = self.currently_processing_sample();

        if !currently_processing_sample && self.num_active_stop_declickers == 0 {
            return ProcessStatus::ClearAllOutputs;
        }

        let mut num_filled_channels = 0;

        if currently_processing_sample {
            match &self.sequence {
                None => {}
                Some(SequenceType::SingleSample { .. }) => {
                    let sample_state = self.loaded_sample_state.as_ref().unwrap();
                    let looping = sample_state
                        .repeat_mode
                        .do_loop(sample_state.num_times_looped_back);

                    let (finished, n_channels) = self.process_internal(
                        outputs,
                        proc_info.frames,
                        looping,
                        proc_info.declick_values,
                    );

                    num_filled_channels = n_channels;

                    self.shared_state.playhead_frames.store(
                        self.loaded_sample_state.as_ref().unwrap().playhead,
                        Ordering::Relaxed,
                    );

                    if finished {
                        self.playback_state = PlaybackState::Stopped;
                        self.shared_state
                            .playback_state
                            .store(PlaybackState::Stopped);
                    }
                }
                Some(SequenceType::Sequence { sequence, timing }) => {
                    todo!()
                }
            }
        }

        for (i, out_buf) in outputs.iter_mut().enumerate().skip(num_filled_channels) {
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
            self.sequence = None;
            self.loaded_sample_state = None;
            self.shared_state
                .playback_state
                .store(PlaybackState::Stopped);
        }
    }
}

struct LoadedSampleState {
    sample: Arc<dyn SampleResource>,
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
