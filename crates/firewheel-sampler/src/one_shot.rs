use std::{num::NonZeroUsize, sync::Arc};

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

pub const DEFAULT_MAX_VOICES: usize = 8;

const MAX_OUT_CHANNELS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OneShotSamplerConfig {
    pub declick_duration_seconds: f32,
    pub mono_to_stereo: bool,
}

impl Default for OneShotSamplerConfig {
    fn default() -> Self {
        Self {
            declick_duration_seconds: smoothing_filter::DEFAULT_SMOOTH_SECONDS,
            mono_to_stereo: true,
        }
    }
}

pub struct OneShotSamplerNode<const MAX_VOICES: usize> {
    config: OneShotSamplerConfig,
}

impl<const MAX_VOICES: usize> OneShotSamplerNode<MAX_VOICES> {
    pub fn new(config: OneShotSamplerConfig) -> Self {
        Self { config }
    }
}

impl<const MAX_VOICES: usize> AudioNode for OneShotSamplerNode<MAX_VOICES> {
    fn debug_name(&self) -> &'static str {
        "one_shot_sampler"
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
        channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(OneShotSamplerProcessor::<MAX_VOICES> {
            declick_filter_coeff: smoothing_filter::Coeff::new(
                stream_info.sample_rate,
                self.config.declick_duration_seconds,
            ),
            voices: std::array::from_fn(|_| Voice::new()),
            voices_free_slots: (0..MAX_VOICES).collect(),
            active_voices: ArrayVec::new(),
            tmp_active_voices: ArrayVec::new(),
            mono_to_stereo: self.config.mono_to_stereo,
            tmp_buffer: (0..channel_config.num_outputs.get())
                .map(|_| vec![0.0; stream_info.max_block_samples as usize])
                .collect(),
            tmp_declick_buffer: vec![0.0; stream_info.max_block_samples as usize],
        }))
    }
}

impl<const MAX_VOICES: usize> Into<Box<dyn AudioNode>> for OneShotSamplerNode<MAX_VOICES> {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}

struct OneShotSamplerProcessor<const MAX_VOICES: usize> {
    voices: [Voice; MAX_VOICES],
    voices_free_slots: ArrayVec<usize, MAX_VOICES>,
    active_voices: ArrayVec<usize, MAX_VOICES>,
    tmp_active_voices: ArrayVec<usize, MAX_VOICES>,

    declick_filter_coeff: smoothing_filter::Coeff,
    mono_to_stereo: bool,

    tmp_buffer: Vec<Vec<f32>>,
    tmp_declick_buffer: Vec<f32>,
}

impl<const MAX_VOICES: usize> AudioNodeProcessor for OneShotSamplerProcessor<MAX_VOICES> {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        for msg in events {
            match msg {
                NodeEventType::Pause => {
                    for &voice_i in self.active_voices.iter() {
                        self.voices[voice_i].pause();
                    }
                }
                NodeEventType::Resume => {
                    for &voice_i in self.active_voices.iter() {
                        self.voices[voice_i].resume();
                    }
                }
                NodeEventType::Stop => {
                    for &voice_i in self.active_voices.iter() {
                        self.voices[voice_i].stop();
                    }
                }
                NodeEventType::PlaySample {
                    sample,
                    normalized_volume,
                    stop_other_voices,
                } => {
                    if *stop_other_voices {
                        for &voice_i in self.active_voices.iter() {
                            self.voices[voice_i].stop();
                        }
                    }

                    let mut gain = normalized_volume_to_raw_gain(*normalized_volume);
                    if gain < 0.00001 {
                        continue;
                    }
                    if gain > 0.99999 && gain < 1.00001 {
                        gain = 1.0;
                    }

                    let voice = if let Some(voice_i) = self.voices_free_slots.pop() {
                        self.active_voices.push(voice_i);
                        &mut self.voices[voice_i]
                    } else {
                        // Steal the oldest voice.
                        self.voices
                            .iter_mut()
                            .max_by(|a, b| a.playhead.cmp(&b.playhead))
                            .unwrap()
                    };

                    voice.start(sample, gain);
                }
                _ => {}
            }
        }

        if self.active_voices.is_empty() {
            return ProcessStatus::ClearAllOutputs;
        }

        let mut num_filled_channels = 0;
        self.tmp_active_voices.clear();

        if let Some(&voice_i) = self.active_voices.first() {
            let voice = &mut self.voices[voice_i];

            num_filled_channels = if (voice.paused || !voice.is_playing) && !voice.is_declicking {
                0
            } else {
                voice.process(
                    outputs,
                    None,
                    &mut self.tmp_declick_buffer,
                    proc_info.samples,
                    self.mono_to_stereo,
                    self.declick_filter_coeff,
                )
            };

            if voice.is_playing {
                if !voice.paused && voice.playhead >= voice.len_samples {
                    // Voice has finished playing, so remove it.
                    voice.sample = None;
                    self.voices_free_slots.push(voice_i);
                } else {
                    self.tmp_active_voices.push(voice_i);
                }
            } else {
                if voice.is_declicking {
                    self.tmp_active_voices.push(voice_i);
                } else {
                    // Voice has finished being stopped, so remove it.
                    voice.sample = None;
                    self.voices_free_slots.push(voice_i);
                }
            }
        }

        for (i, out_buf) in outputs.iter_mut().enumerate().skip(num_filled_channels) {
            if !proc_info.out_silence_mask.is_channel_silent(i) {
                out_buf.fill(0.0);
            }
        }

        if self.active_voices.len() > 1 {
            let mut tmp_buffer: ArrayVec<&mut [f32], MAX_OUT_CHANNELS> = self
                .tmp_buffer
                .iter_mut()
                .map(|b| b.as_mut_slice())
                .collect();

            for &voice_i in self.active_voices.iter().skip(1) {
                let voice = &mut self.voices[voice_i];

                if (voice.paused || !voice.is_playing) && !voice.is_declicking {
                    continue;
                }

                num_filled_channels = num_filled_channels.max(voice.process(
                    outputs,
                    Some(&mut tmp_buffer),
                    &mut self.tmp_declick_buffer,
                    proc_info.samples,
                    self.mono_to_stereo,
                    self.declick_filter_coeff,
                ));

                if voice.is_playing {
                    if !voice.paused && voice.playhead >= voice.len_samples {
                        // Voice has finished playing, so remove it.
                        voice.sample = None;
                        self.voices_free_slots.push(voice_i);
                    } else {
                        self.tmp_active_voices.push(voice_i);
                    }
                } else {
                    if voice.is_declicking {
                        self.tmp_active_voices.push(voice_i);
                    } else {
                        // Voice has finished being stopped, so remove it.
                        voice.sample = None;
                        self.voices_free_slots.push(voice_i);
                    }
                }
            }
        }

        self.active_voices = self.tmp_active_voices.clone();

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

struct Voice {
    sample: Option<Arc<dyn SampleResource>>,
    playhead: u64,
    paused_playhead: u64,
    len_samples: u64,
    num_channels: NonZeroUsize,
    gain: f32,
    is_playing: bool,
    paused: bool,
    is_declicking: bool,
    declick_filter_state: f32,
    declick_filter_target: f32,
}

impl Voice {
    fn new() -> Self {
        Self {
            sample: None,
            playhead: 0,
            paused_playhead: 0,
            len_samples: 0,
            num_channels: NonZeroUsize::MIN,
            gain: 1.0,
            is_playing: false,
            paused: false,
            is_declicking: false,
            declick_filter_state: 1.0,
            declick_filter_target: 1.0,
        }
    }

    fn start(&mut self, sample: &Arc<dyn SampleResource>, gain: f32) {
        self.len_samples = sample.len_samples();
        self.num_channels = sample.num_channels();
        self.sample = Some(Arc::clone(&sample));
        self.gain = gain;
        self.is_playing = true;
        self.paused = false;
        self.playhead = 0;
        self.paused_playhead = 0;
        self.is_declicking = false;
        self.declick_filter_state = 1.0;
        self.declick_filter_target = 1.0;
    }

    fn pause(&mut self) {
        if !self.is_playing || self.paused {
            return;
        }

        self.paused = true;
        self.paused_playhead = self.playhead;
        self.declick_filter_target = 0.0;

        if self.playhead == 0 {
            // The sample hasn't event begun playing yet, so no need to declick.
            self.declick_filter_state = 0.0;
        }

        self.is_declicking = self.declick_filter_state != self.declick_filter_target;
    }

    fn resume(&mut self) {
        if !self.is_playing || !self.paused {
            return;
        }

        self.paused = false;
        self.playhead = self.paused_playhead;
        self.declick_filter_target = 1.0;

        if self.playhead == 0 {
            // The sample hasn't event begun playing yet, so no need to declick.
            self.declick_filter_state = 1.0;
        }

        self.is_declicking = self.declick_filter_state != self.declick_filter_target;
    }

    fn stop(&mut self) {
        if !self.is_playing {
            return;
        }

        self.pause();
        self.is_playing = false;
    }

    fn process(
        &mut self,
        outputs: &mut [&mut [f32]],
        mut tmp_buffer: Option<&mut [&mut [f32]]>,
        tmp_declick_buffer: &mut [f32],
        block_samples: usize,
        mono_to_stereo: bool,
        declick_filter_coeff: smoothing_filter::Coeff,
    ) -> usize {
        let Some(sample) = self.sample.as_ref() else {
            return 0;
        };

        let copy_samples = block_samples.min((self.len_samples - self.playhead) as usize);

        if let Some(tmp_buffer) = &mut tmp_buffer {
            sample.fill_buffers(tmp_buffer, 0..copy_samples, self.playhead);
        } else {
            sample.fill_buffers(outputs, 0..copy_samples, self.playhead);
        }

        if copy_samples < block_samples {
            if let Some(tmp_buffer) = &mut tmp_buffer {
                for (_, b) in (0..self.num_channels.get()).zip(tmp_buffer.iter_mut()) {
                    b[copy_samples..].fill(0.0);
                }
            } else {
                for (_, b) in (0..self.num_channels.get()).zip(outputs.iter_mut()) {
                    b[copy_samples..].fill(0.0);
                }
            }
        }

        self.playhead += block_samples as u64;

        let num_filled_channels =
            if outputs.len() > 1 && self.num_channels.get() == 1 && mono_to_stereo {
                if let Some(tmp_buffer) = &mut tmp_buffer {
                    let (b1, b2) = tmp_buffer.split_first_mut().unwrap();
                    b2[0].copy_from_slice(b1);
                } else {
                    let (b1, b2) = outputs.split_first_mut().unwrap();
                    b2[0].copy_from_slice(b1);
                }

                2
            } else {
                self.num_channels.get().min(outputs.len())
            };

        if self.is_declicking {
            self.declick_filter_state = smoothing_filter::process_into_buffer(
                &mut tmp_declick_buffer[..block_samples],
                self.declick_filter_state,
                self.declick_filter_target,
                declick_filter_coeff,
            );

            if let Some(tmp_buffer) = &mut tmp_buffer {
                for (_, (out_buf, in_buf)) in
                    (0..num_filled_channels).zip(outputs.iter_mut().zip(tmp_buffer.iter()))
                {
                    for ((os, &ts), &g) in out_buf
                        .iter_mut()
                        .zip(in_buf.iter())
                        .zip(tmp_declick_buffer.iter())
                    {
                        *os += ts * self.gain * g;
                    }
                }
            } else {
                for (_, out_buf) in (0..num_filled_channels).zip(outputs.iter_mut()) {
                    for (os, &g) in out_buf.iter_mut().zip(tmp_declick_buffer.iter()) {
                        *os *= self.gain * g;
                    }
                }
            }

            if smoothing_filter::has_settled(
                self.declick_filter_state,
                self.declick_filter_target,
                smoothing_filter::DEFAULT_SETTLE_EPSILON,
            ) {
                self.declick_filter_state = self.declick_filter_target;
                self.is_declicking = false;
            }
        } else {
            if let Some(tmp_buffer) = &mut tmp_buffer {
                for (_, (out_buf, in_buf)) in
                    (0..num_filled_channels).zip(outputs.iter_mut().zip(tmp_buffer.iter()))
                {
                    for (os, &is) in out_buf.iter_mut().zip(in_buf.iter()) {
                        *os += is * self.gain;
                    }
                }
            } else if self.gain != 1.0 {
                for (_, out_buf) in (0..num_filled_channels).zip(outputs.iter_mut()) {
                    for s in out_buf.iter_mut() {
                        *s *= self.gain;
                    }
                }
            }
        }

        num_filled_channels
    }
}
