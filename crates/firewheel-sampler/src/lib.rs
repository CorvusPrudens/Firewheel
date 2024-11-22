/*
use std::{
    fmt::Debug,
    num::NonZeroUsize,
    ops::Range,
    sync::{atomic::Ordering, Arc},
};

use firewheel_core::{
    clock::{ClockTime, EventDelay},
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    param::{range::percent_volume_to_raw_gain, smoother::ParamSmoother},
    sample_resource::SampleResource,
    ChannelConfig, ChannelCount, SilenceMask, StreamInfo,
};

/// Configuration of a [`SamplerNode`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerConfig {
    /// The size of the command queue.
    ///
    /// By default this is set to `128`.
    pub command_queue_size: NonZeroUsize,

    /// The maximum number of samples (voiced) that can be played
    /// concurrently in this node.
    ///
    /// By default this is set to `4`.
    pub voice_limit: NonZeroUsize,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            command_queue_size: NonZeroUsize::new(128).unwrap(),
            voice_limit: NonZeroUsize::new(4).unwrap(),
        }
    }
}

/// Additional options for playing a sample.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaySampleOpts {
    /// The range of the sample to play.
    ///
    /// By default this is set to [`SampleRange::FullSample`].
    pub range: SampleRange,

    /// If `true`, then all previously playing samples on this node will
    /// be stopped.
    ///
    /// By default this is set to `false`.
    pub stop_previous: bool,

    /// The percent volume to play this sample at (where `0.0` is silence
    /// and `100.0` is unity gain).
    ///
    /// Note, this is more for creating advanced sequences when chaining
    /// commands together, not for simple sample playback. It is
    /// recommended to use the `Volume` node instead in that case as it
    /// can be dynamically changed.
    ///
    /// If this value is less than or equal to `0.0`, then this command
    /// will be ignored.
    ///
    /// By default this is set to `100.0`.
    pub percent_volume: f32,

    /// The stereo panning of this sample. (Where `0.0` is center, `-1.0`
    /// is full-left, and `1.0` is full-right).
    ///
    /// This has no effect if the sample is not stereo.
    ///
    /// Note, this is more for creating advanced sequences when chaining
    /// commands together, not for simple sample playback. It is
    /// recommended to use the `Volume` node instead in that case as it
    /// can be dynamically changed.
    ///
    /// By default this is set to `0.0`.
    pub pan: f32,
    // TODO: Pitch (doppler stretching) parameter.
}

impl Default for PlaySampleOpts {
    fn default() -> Self {
        Self {
            range: SampleRange::FullSample,
            stop_previous: false,
            percent_volume: 100.0,
            pan: 0.0,
        }
    }
}

/// Additional options for looping a sample.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSampleOpts {
    /// The range in the sample to loop in.
    ///
    /// By default this is set to [`SampleRange::FullSample`].
    pub range: SampleRange,

    /// How many times to repeat the loop.
    ///
    /// By default this is set to [`LoopMode::Endless`].
    pub mode: LoopMode,

    /// Where to begin playing in the sample relative to the start of the
    /// loop range in units of seconds. If the given time it outside the
    /// loop range, then the beginning of the loop range will be used
    /// instead.
    ///
    /// By default this is set to `0.0`.
    pub start_from_secs: f64,

    /// If `true`, then the previous loop iteration will be stopped (choked)
    /// when the range is less than the length of the sample.
    ///
    /// If `false`, then the previous loop iteration will continue playing
    /// until completion, overlapping with the next loop iteration. This
    /// can be useful for example making a "rapid fire" of gunshot sounds.
    ///
    /// This has no effect when the range is [`SampleRange::FullSample`] or
    /// when it is longer than the length of the sample.
    ///
    /// By default this is set to `true`.
    pub choke: bool,

    /// The percent volume to play this sample at (where `0.0` is silence
    /// and `100.0` is unity gain).
    ///
    /// Note that gain and pan cannot be changed later for this sample
    /// playback. Dynamic gain and pan are done using separate audio
    /// nodes.
    ///
    /// If this value is less than or equal to `0.0`, then this command
    /// will be ignored.
    ///
    /// By default this is set to `100.0`.
    pub percent_volume: f32,

    /// The stereo panning of this sample. (Where `0.0` is center, `-1.0`
    /// is full-left, and `1.0` is full-right).
    ///
    /// This has no effect if the the output of this node is not stereo.
    ///
    /// Note that gain and pan cannot be changed later for this sample
    /// playback. Dynamic gain and pan are done using separate audio
    /// nodes.
    ///
    /// By default this is set to `0.0`.
    pub pan: f32,
    // TODO: Pitch (doppler stretching) parameter.
}

impl Default for LoopSampleOpts {
    fn default() -> Self {
        Self {
            range: SampleRange::FullSample,
            mode: LoopMode::Endless,
            start_from_secs: 0.0,
            choke: true,
            percent_volume: 100.0,
            pan: 0.0,
        }
    }
}

/// A command for a [`SamplerNode`].
#[derive(Debug, Clone, PartialEq)]
pub struct SamplerCommand<S: SampleResource> {
    /// When the command should occur.
    pub delay: EventDelay,
    /// The type of command to execute.
    pub command: SamplerCommandType<S>,
}

/// The type of command for a [`SamplerNode`].
#[derive(Debug, Clone, PartialEq)]
pub enum SamplerCommandType<S: SampleResource> {
    /// Play a sample to completion.
    ///
    /// If the number of samples being played in this node exceeds the sample
    /// (voice) limit, the the oldest voice will be stopped and replaced with
    /// this one.
    PlaySample {
        /// The sample to play.
        ///
        /// If this is `None`, then the previously played sample will be played
        /// again. If there was no previously played sample, then this command
        /// will be ignored.
        sample: Option<S>,
        /// Additional options for playing a sample.
        opts: PlaySampleOpts,
    },
    /// Play a sample, seamlessly looping back to the start of the range.
    ///
    /// If the number of samples being played in this node exceeds the sample
    /// (voice) limit, the the oldest voice will be stopped and replaced with
    /// this one.
    LoopSample {
        /// The sample to play.
        ///
        /// If this is `None`, then the previously played sample will be played
        /// again. If there was no previously played sample, then this command
        /// will be ignored.
        sample: Option<S>,
        /// Additional options for looping a sample.
        opts: LoopSampleOpts,
    },
    /// Pause sample playback.
    Pause,
    /// Resume sample playback.
    Resume,
    /// Stop all currently playing samples on this node.
    ///
    /// Sending [`SamplerCommandType::Resume` afterwards will *NOT* resume
    /// playback of the previously playing samples.
    Stop,
}

/// The range in a sample resource [`SampleResource`]
#[derive(Default, Debug, Clone, PartialEq)]
pub enum SampleRange {
    /// Use the full length of the sample.
    #[default]
    FullSample,
    /// Use only a section of the sample (units are in seconds).
    ///
    /// The start of the range must be greater than or equal to 0.0.
    ///
    /// The end of the range may extend past the length of the sample,
    /// in which case silence will be played to fill in the gaps.
    RangeSecs(Range<f64>),
}

/// The number of times to loop an audio sample.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    /// Loop endlessly. Useful for music playback.
    #[default]
    Endless,
    /// Loop the given number of times
    Count(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandType<S: SampleResource> {
    Single(SamplerCommand<S>),
    Group(Vec<SamplerCommand<S>>),
}

enum ProcessorToNodeMsg<S: SampleResource> {
    ReturnSample(S),
    ReturnCommandGroup(Vec<SamplerCommand<S>>),
}

pub struct ActiveSamplerNode<S: SampleResource> {
    // TODO: Find a good solution for webassembly.
    to_processor_tx: rtrb::Producer<CommandType<S>>,
    from_processor_rx: rtrb::Consumer<ProcessorToNodeMsg<S>>,
}

impl<S: SampleResource> ActiveSamplerNode<S> {
    /// Play a sample to completion.
    ///
    /// If the number of samples being played in this node exceeds the sample
    /// (voice) limit, the the oldest voice will be stopped and replaced with
    /// this one.
    ///
    /// * `sample` - The sample to play. If this is `None`, then the previously
    /// played sample will be played again. If there was no previously played
    /// sample, then this command will be ignored.
    /// * `delay` - When the command should occur.
    /// * `opts` - Additional options for playing a sample.
    ///
    /// Returns an error if the command queue is full.
    pub fn play_sample(
        &mut self,
        sample: Option<S>,
        delay: EventDelay,
        opts: PlaySampleOpts,
    ) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.push_command(SamplerCommand {
            delay,
            command: SamplerCommandType::PlaySample { sample, opts },
        })
    }

    /// Play a sample, seamlessly looping back to the start of the range.
    ///
    /// If the number of samples being played in this node exceeds the sample
    /// (voice) limit, the the oldest voice will be stopped and replaced with
    /// this one.
    ///
    /// * `sample` - The sample to loop. If this is `None`, then the previously
    /// played sample will be played again. If there was no previously played
    /// sample, then this command will be ignored.
    /// * `delay` - When the command should occur.
    /// * `opts` - Additional options for looping a sample.
    ///
    /// Returns an error if the command queue is full.
    pub fn loop_sample(
        &mut self,
        sample: Option<S>,
        delay: EventDelay,
        opts: LoopSampleOpts,
    ) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.push_command(SamplerCommand {
            delay,
            command: SamplerCommandType::LoopSample { sample, opts },
        })
    }

    /// Pause sample playback.
    ///
    /// * `delay` - When the command should occur.
    ///
    /// Returns an error if the command queue is full.
    pub fn pause(&mut self, delay: EventDelay) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.push_command(SamplerCommand {
            delay,
            command: SamplerCommandType::Pause,
        })
    }

    /// Resume sample playback.
    ///
    /// * `delay` - When the command should occur.
    ///
    /// Returns an error if the command queue is full.
    pub fn resume(&mut self, delay: EventDelay) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.push_command(SamplerCommand {
            delay,
            command: SamplerCommandType::Resume,
        })
    }

    /// Stop all currently playing samples on this node.
    ///
    /// Calling [`ActiveSamplerNode::resume()`] afterwards will *NOT* resume
    /// playback of the previously playing samples.
    ///
    /// * `delay` - When the command should occur.
    ///
    /// Returns an error if the command queue is full.
    pub fn stop(&mut self, delay: EventDelay) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.push_command(SamplerCommand {
            delay,
            command: SamplerCommandType::Stop,
        })
    }

    /// Push a new [`SamplerCommand`] to execute.
    ///
    /// Returns an error if the command queue is full.
    pub fn push_command(
        &mut self,
        command: SamplerCommand<S>,
    ) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.to_processor_tx.push(CommandType::Single(command))
    }

    /// Push a new group of [`SamplerCommand`]s to execute.
    ///
    /// Returns an error if the command queue is full.
    pub fn push_command_group(
        &mut self,
        commands: Vec<SamplerCommand<S>>,
    ) -> Result<(), rtrb::PushError<CommandType<S>>> {
        self.to_processor_tx.push(CommandType::Group(commands))
    }
}

pub struct SamplerNode<S: SampleResource> {
    active_state: Option<ActiveSamplerNode<S>>,
    config: SamplerConfig,
}

impl<S: SampleResource> SamplerNode<S> {
    pub fn new(config: SamplerConfig) -> Self {
        Self {
            active_state: None,
            config,
        }
    }

    /// Get an immutable reference the active context.
    ///
    /// Returns `None` if this node is not currently activated.
    pub fn get(&self) -> Option<&ActiveSamplerNode<S>> {
        self.active_state.as_ref()
    }

    /// Get a mutable reference the active context.
    ///
    /// Returns `None` if this node is not currently activated.
    pub fn get_mut(&mut self) -> Option<&mut ActiveSamplerNode<S>> {
        self.active_state.as_mut()
    }
}

impl<S: SampleResource> AudioNode for SamplerNode<S> {
    fn debug_name(&self) -> &'static str {
        "sampler"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::MAX,
            updates: true,
            ..Default::default()
        }
    }

    fn activate(
        &mut self,
        stream_info: StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        let (to_processor_tx, from_node_rx) =
            rtrb::RingBuffer::<CommandType<S>>::new(self.config.command_queue_size.get());
        let (to_node_tx, from_processor_rx) =
            rtrb::RingBuffer::<ProcessorToNodeMsg<S>>::new(self.config.command_queue_size.get());

        self.active_state = Some(ActiveSamplerNode {
            to_processor_tx,
            from_processor_rx,
        });

        Ok(Box::new(SamplerProcessor::new(
            stream_info.sample_rate,
            stream_info.stream_latency_samples as usize,
            stream_info.max_block_samples as usize,
            from_node_rx,
            to_node_tx,
        )))
    }

    fn update(&mut self) {
        if let Some(active_state) = &mut self.active_state {
            while let Ok(msg) = active_state.from_processor_rx.pop() {
                // Clean up resources.
                match msg {
                    ProcessorToNodeMsg::ReturnSample(_smp) => {}
                    ProcessorToNodeMsg::ReturnCommandGroup(_cmds) => {}
                }
            }
        }
    }
}

struct SamplerProcessor<S: SampleResource> {
    playing: bool,
    sample_rate: u32,
    stream_latency_samples: usize,
    playhead: u64,
    loop_range: Option<ProcSampleRange>,

    sample: Option<S>,

    from_node_rx: rtrb::Consumer<CommandType<S>>,
    to_node_tx: rtrb::Producer<ProcessorToNodeMsg<S>>,
}

impl<S: SampleResource> SamplerProcessor<S> {
    fn new(
        sample_rate: u32,
        stream_latency_samples: usize,
        max_block_samples: usize,
        from_node_rx: rtrb::Consumer<CommandType<S>>,
        to_node_tx: rtrb::Producer<ProcessorToNodeMsg<S>>,
    ) -> Self {
        Self {
            playing: false,
            sample_rate,
            stream_latency_samples,
            playhead: 0,
            loop_range: None,
            sample: None,
            from_node_rx,
            to_node_tx,
        }
    }
}

impl<S: SampleResource> AudioNodeProcessor for SamplerProcessor<S> {
    fn process(
        &mut self,
        samples: usize,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _proc_info: ProcInfo,
    ) -> ProcessStatus {
        while let Ok(msg) = self.from_node_rx.pop() {
            match msg {
                NodeToProcessorMsg::SetSample {
                    sample,
                    stop_playback,
                } => {
                    if let Some(old_sample) = self.sample.take() {
                        let _ = self
                            .to_node_tx
                            .push(ProcessorToNodeMsg::ReturnSample(old_sample));
                    }

                    self.sample = Some(sample);

                    if let Some(loop_range) = &mut self.loop_range {
                        loop_range.update_sample(&self.sample);
                    }

                    if stop_playback {
                        self.playhead = self
                            .loop_range
                            .as_ref()
                            .map(|l| l.playhead_range.start)
                            .unwrap_or(0);

                        if self.playing {
                            self.playing = false;

                            // TODO
                        }
                    }

                    // TODO: Declick
                }
                NodeToProcessorMsg::Play => {
                    if !self.playing {
                        self.playing = true;

                        // TODO: Declick
                    }
                }
                NodeToProcessorMsg::Pause => {
                    if self.playing {
                        self.playing = false;

                        // TODO: Declick
                    }
                }
                NodeToProcessorMsg::Stop => {
                    self.playhead = self
                        .loop_range
                        .as_ref()
                        .map(|l| l.playhead_range.start)
                        .unwrap_or(0);

                    if self.playing {
                        self.playing = false;

                        // TODO: Declick
                    }
                }
                NodeToProcessorMsg::SetPlayheadSecs(playhead_secs) => {
                    let sample = (playhead_secs * f64::from(self.sample_rate)).round() as u64;

                    if sample != self.playhead {
                        self.playhead = sample;
                        // TODO: Declick
                    }
                }
                NodeToProcessorMsg::SetSampleRange(loop_range) => {
                    self.loop_range = loop_range.map(|loop_range| {
                        ProcSampleRange::new(loop_range, self.sample_rate, &self.sample)
                    });

                    if let Some(loop_range) = &self.loop_range {
                        if loop_range.playhead_range.contains(&self.playhead) {
                            self.playhead = loop_range.playhead_range.start;

                            // TODO: Declick
                        }
                    }
                }
            }
        }

        let Some(sample) = &self.sample else {
            // TODO: Declick

            // No sample data, output silence.
            return ProcessStatus::ClearAllOutputs;
        };

        if !self.playing {
            // TODO: Declick

            // Not playing, output silence.
            return ProcessStatus::ClearAllOutputs;
        }

        let raw_gain = self.raw_gain.load(Ordering::Relaxed);
        let gain = self.gain_smoother.set_and_process(raw_gain, samples);
        // Hint to the compiler to optimize loop.
        assert_eq!(gain.values.len(), samples);

        if !gain.is_smoothing() && gain.values[0] < 0.00001 {
            // TODO: Reset declick.

            // Muted, so there is no need to process.
            return ProcessStatus::ClearAllOutputs;
        }

        if let Some(loop_range) = &self.loop_range {
            if self.playhead >= loop_range.playhead_range.end {
                // Playhead is out of range. Return to the start.
                self.playhead = self
                    .loop_range
                    .as_ref()
                    .map(|l| l.playhead_range.start)
                    .unwrap_or(0);
            }

            // Copy first block of samples.

            let samples_left = if loop_range.playhead_range.end - self.playhead <= usize::MAX as u64
            {
                (loop_range.playhead_range.end - self.playhead) as usize
            } else {
                usize::MAX
            };
            let first_copy_samples = samples.min(samples_left);

            sample.fill_buffers(outputs, 0..first_copy_samples, self.playhead);

            if first_copy_samples < samples {
                // Loop back to the start.
                self.playhead = self
                    .loop_range
                    .as_ref()
                    .map(|l| l.playhead_range.start)
                    .unwrap_or(0);

                // Copy second block of samples.

                let second_copy_samples = samples - first_copy_samples;

                sample.fill_buffers(outputs, first_copy_samples..samples, self.playhead);

                self.playhead += second_copy_samples as u64;
            } else {
                self.playhead += samples as u64;
            }
        } else {
            if self.playhead >= sample.len_samples() {
                // Playhead is out of range. Output silence.
                return ProcessStatus::ClearAllOutputs;

                // TODO: Notify node that sample has finished.
            }

            let copy_samples = samples.min((sample.len_samples() - self.playhead) as usize);

            sample.fill_buffers(outputs, 0..copy_samples, self.playhead);

            if copy_samples < samples {
                // Finished playing sample.
                self.playing = false;
                self.playhead = 0;

                // Fill any remaining samples with zeros
                for out_ch in outputs.iter_mut() {
                    out_ch[copy_samples..].fill(0.0);
                }

                // TODO: Notify node that sample has finished.
            } else {
                self.playhead += samples as u64;
            }
        }

        let sample_channels = sample.num_channels().get();

        // Apply gain and declick
        // TODO: Declick
        if outputs.len() >= 2 && sample_channels == 2 {
            // Provide an optimized stereo loop.

            // Hint to the compiler to optimize loop.
            assert_eq!(outputs[0].len(), samples);
            assert_eq!(outputs[1].len(), samples);

            for i in 0..samples {
                outputs[0][i] *= gain.values[i];
                outputs[1][i] *= gain.values[i];
            }
        } else {
            for (out_ch, _) in outputs.iter_mut().zip(0..sample_channels) {
                // Hint to the compiler to optimize loop.
                assert_eq!(out_ch.len(), samples);

                for i in 0..samples {
                    out_ch[i] *= gain.values[i];
                }
            }
        }

        let mut out_silence_mask = SilenceMask::NONE_SILENT;

        if outputs.len() > sample_channels {
            if outputs.len() == 2 && sample_channels == 1 {
                // If the output of this node is stereo and the sample is mono,
                // assume that the user wants both channels filled with the
                // sample data.
                let (out_first, outs) = outputs.split_first_mut().unwrap();
                outs[0].copy_from_slice(out_first);
            } else {
                // Fill the rest of the channels with zeros.
                for (i, out_ch) in outputs.iter_mut().enumerate().skip(sample_channels) {
                    out_ch.fill(0.0);
                    out_silence_mask.set_channel(i, true);
                }
            }
        }

        ProcessStatus::outputs_modified(out_silence_mask)
    }
}

impl<S: SampleResource> Drop for SamplerProcessor<S> {
    fn drop(&mut self) {
        if let Some(sample) = self.sample.take() {
            let _ = self
                .to_node_tx
                .push(ProcessorToNodeMsg::ReturnSample(sample));
        }
    }
}

impl<S: SampleResource> Into<Box<dyn AudioNode>> for SamplerNode<S> {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}

struct Voice<S: SampleResource> {
    sample: S,

    raw_gain_l: f32,
    raw_gain_r: Option<f32>,
}

impl<S: SampleResource> Voice<S> {

}

struct ProcSampleRange {
    playhead_range: Range<u64>,
    full_range: bool,
}

impl ProcSampleRange {
    fn new<S: SampleResource>(
        loop_range: SampleRange,
        sample_rate: u32,
        sample: &Option<S>,
    ) -> Self {
        let (start_frame, end_frame, full_range) = match &loop_range {
            SampleRange::FullSample => {
                let end_frame = if let Some(sample) = sample {
                    sample.len_samples()
                } else {
                    0
                };

                (0, end_frame, true)
            }
            SampleRange::RangeSecs(range) => (
                (range.start * f64::from(sample_rate)).round() as u64,
                (range.end * f64::from(sample_rate)).round() as u64,
                false,
            ),
        };

        Self {
            playhead_range: start_frame..end_frame,
            full_range,
        }
    }

    fn update_sample<S: SampleResource>(&mut self, sample: &Option<S>) {
        let Some(sample) = sample else {
            return;
        };

        if !self.full_range {
            return;
        }

        let end_frame = sample.len_samples();

        self.playhead_range = 0..end_frame;
    }
}
*/
