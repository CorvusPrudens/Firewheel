use std::{num::NonZeroU32, sync::Arc};

use firewheel_core::{
    dsp::decibel::normalized_volume_to_raw_gain,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, EventData, NodeEventIter, ProcInfo,
        ProcessStatus,
    },
    sample_resource::SampleResource,
    ChannelConfig, ChannelCount, SilenceMask, StreamInfo,
};
use smallvec::SmallVec;

use crate::{voice::SamplerVoice, MAX_OUT_CHANNELS};

pub const STATIC_ALLOC_VOICES: usize = 8;

#[derive(Clone)]
pub struct Sample {
    pub sample: Arc<dyn SampleResource + 'static>,
    pub normalized_volume: f32,
    pub stop_other_voices: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OneShotSamplerConfig {
    pub max_voices: NonZeroU32,
    pub declick_duration_seconds: f32,
    pub mono_to_stereo: bool,
}

impl Default for OneShotSamplerConfig {
    fn default() -> Self {
        Self {
            max_voices: NonZeroU32::new(STATIC_ALLOC_VOICES as u32).unwrap(),
            declick_duration_seconds: 3.0 / 1_000.0,
            mono_to_stereo: true,
        }
    }
}

pub struct OneShotSamplerNode {
    config: OneShotSamplerConfig,
}

impl OneShotSamplerNode {
    pub fn new(config: OneShotSamplerConfig) -> Self {
        Self { config }
    }
}

impl AudioNode for OneShotSamplerNode {
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
        Ok(Box::new(OneShotSamplerProcessor::new(
            stream_info,
            channel_config.num_outputs.get() as usize,
            &self.config,
        )))
    }
}

impl Into<Box<dyn AudioNode>> for OneShotSamplerNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}

struct OneShotSamplerProcessor {
    voices: SmallVec<[OneShotVoiceState; STATIC_ALLOC_VOICES]>,
    voices_free_slots: SmallVec<[usize; STATIC_ALLOC_VOICES]>,
    active_voices: SmallVec<[usize; STATIC_ALLOC_VOICES]>,
    tmp_active_voices: SmallVec<[usize; STATIC_ALLOC_VOICES]>,

    mono_to_stereo: bool,
}

impl OneShotSamplerProcessor {
    fn new(
        _stream_info: &StreamInfo,
        _num_out_channels: usize,
        config: &OneShotSamplerConfig,
    ) -> Self {
        let max_voices = config.max_voices.get() as usize;

        Self {
            voices: (0..max_voices).map(|_| OneShotVoiceState::new()).collect(),
            voices_free_slots: (0..max_voices).collect(),
            active_voices: SmallVec::with_capacity(max_voices),
            tmp_active_voices: SmallVec::with_capacity(max_voices),
            mono_to_stereo: config.mono_to_stereo,
        }
    }
}

impl AudioNodeProcessor for OneShotSamplerProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        for msg in events {
            match msg {
                EventData::Pause => {
                    for &voice_i in self.active_voices.iter() {
                        self.voices[voice_i].voice.pause(proc_info.declick_values);
                    }
                }
                EventData::Resume => {
                    for &voice_i in self.active_voices.iter() {
                        self.voices[voice_i].voice.resume(proc_info.declick_values);
                    }
                }
                EventData::Stop => {
                    for &voice_i in self.active_voices.iter() {
                        self.voices[voice_i].voice.stop(proc_info.declick_values);
                    }
                }
                EventData::Custom(sample) => {
                    let Some(Sample {
                        sample,
                        normalized_volume,
                        stop_other_voices,
                    }) = sample.downcast_ref()
                    else {
                        continue;
                    };

                    if *stop_other_voices {
                        for &voice_i in self.active_voices.iter() {
                            self.voices[voice_i].voice.stop(proc_info.declick_values);
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
                            .max_by(|a, b| a.voice.playhead().cmp(&b.voice.playhead()))
                            .unwrap()
                    };

                    voice.gain = gain;
                    voice.voice.init_with_sample(sample, 0);
                    voice.voice.resume(proc_info.declick_values);
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

            num_filled_channels =
                voice
                    .voice
                    .process(outputs, proc_info.samples, false, proc_info.declick_values);

            if voice.gain != 1.0 {
                for b in outputs[..num_filled_channels].iter_mut() {
                    for s in b[..proc_info.samples].iter_mut() {
                        *s *= voice.gain;
                    }
                }
            }

            if self.mono_to_stereo && outputs.len() > 1 && num_filled_channels == 1 {
                let (b1, b2) = outputs.split_first_mut().unwrap();
                b2[0][..proc_info.samples].copy_from_slice(&b1[..proc_info.samples]);
                num_filled_channels = 2;
            }

            if voice.voice.is_finished() {
                voice.voice.clear_sample();
                self.voices_free_slots.push(voice_i);
            } else {
                self.tmp_active_voices.push(voice_i);
            }
        }

        for (i, out_buf) in outputs.iter_mut().enumerate().skip(num_filled_channels) {
            if !proc_info.out_silence_mask.is_channel_silent(i) {
                out_buf[..proc_info.samples].fill(0.0);
            }
        }

        if self.active_voices.len() > 1 {
            for &voice_i in self.active_voices.iter().skip(1) {
                let voice = &mut self.voices[voice_i];

                let mut n_channels = voice.voice.process(
                    &mut proc_info.scratch_buffers[..outputs.len()],
                    proc_info.samples,
                    false,
                    proc_info.declick_values,
                );

                if self.mono_to_stereo && outputs.len() > 1 && n_channels == 1 {
                    let (b1, b2) = proc_info.scratch_buffers.split_first_mut().unwrap();
                    b2[0][..proc_info.samples].copy_from_slice(&b1[..proc_info.samples]);
                    n_channels = 2;
                }

                for (out_buf, s_buf) in outputs[..n_channels]
                    .iter_mut()
                    .zip(proc_info.scratch_buffers[..n_channels].iter())
                {
                    for (os, &ss) in out_buf[..proc_info.samples]
                        .iter_mut()
                        .zip(s_buf[..proc_info.samples].iter())
                    {
                        *os += ss * voice.gain;
                    }
                }

                num_filled_channels = num_filled_channels.max(n_channels);

                if voice.voice.is_finished() {
                    voice.voice.clear_sample();
                    self.voices_free_slots.push(voice_i);
                } else {
                    self.tmp_active_voices.push(voice_i);
                }
            }
        }

        std::mem::swap(&mut self.active_voices, &mut self.tmp_active_voices);

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

struct OneShotVoiceState {
    voice: SamplerVoice,
    gain: f32,
}

impl OneShotVoiceState {
    fn new() -> Self {
        Self {
            voice: SamplerVoice::new(),
            gain: 1.0,
        }
    }
}
