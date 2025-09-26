use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount, NonZeroChannelCount},
    diff::{Diff, Patch},
    dsp::{
        fade::FadeCurve,
        filter::smoothing_filter::DEFAULT_SMOOTH_SECONDS,
        volume::{Volume, DEFAULT_AMP_EPSILON},
    },
    event::ProcEvents,
    mask::{MaskType, SilenceMask},
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, ConstructProcessorContext, ProcBuffers,
        ProcExtra, ProcInfo, ProcessStatus,
    },
    param::smoother::{SmoothedParam, SmootherConfig},
};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
pub struct CrossfadeNodeConfig {
    /// The number of input channels for a single input. This will also be
    /// the total number of output channels.
    ///
    /// ## Panics
    ///
    /// This will cause a panic if this value is greater than `32`.
    pub channels: NonZeroChannelCount,
}

impl Default for CrossfadeNodeConfig {
    fn default() -> Self {
        Self {
            channels: NonZeroChannelCount::STEREO,
        }
    }
}

#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
pub struct CrossfadeNode {
    /// The overall volume.
    pub volume: Volume,

    /// The crossfade amount in the range `[-1.0, 1.0]`, where `-1.0` is fully
    /// the first input, `1.0` is fully the second input, and `0.0` is an equal
    /// mix of both inputs.
    pub crossfade: f32,

    /// The algorithm used to map the normalized panning value in the range
    /// `[-1.0, 1.0]` to the corresponding gain values for the left and right
    /// channels.
    pub fade_curve: FadeCurve,

    /// The time in seconds of the internal smoothing filter.
    ///
    /// By default this is set to `0.015` (15ms).
    pub smooth_seconds: f32,
    /// If the resutling gain (in raw amplitude, not decibels) is less
    /// than or equal to this value, then the gain will be clamped to
    /// `0.0` (silence).
    ///
    /// By default this is set to `0.00001` (-100 decibels).
    pub min_gain: f32,
}

impl CrossfadeNode {
    pub const fn from_volume_crossfade(volume: Volume, crossfade: f32) -> Self {
        Self {
            volume,
            crossfade,
            fade_curve: FadeCurve::EqualPower3dB,
            smooth_seconds: DEFAULT_SMOOTH_SECONDS,
            min_gain: DEFAULT_AMP_EPSILON,
        }
    }

    pub const fn from_crossfade(crossfade: f32) -> Self {
        Self {
            volume: Volume::UNITY_GAIN,
            crossfade,
            fade_curve: FadeCurve::EqualPower3dB,
            smooth_seconds: DEFAULT_SMOOTH_SECONDS,
            min_gain: DEFAULT_AMP_EPSILON,
        }
    }

    /// Set the given volume in a linear scale, where `0.0` is silence and
    /// `1.0` is unity gain.
    ///
    /// These units are suitable for volume sliders (simply convert percent
    /// volume to linear volume by diving the percent volume by 100).
    pub const fn set_volume_linear(&mut self, linear: f32) {
        self.volume = Volume::Linear(linear);
    }

    /// Set the given volume in decibels, where `0.0` is unity gain and
    /// `f32::NEG_INFINITY` is silence.
    pub const fn set_volume_decibels(&mut self, decibels: f32) {
        self.volume = Volume::Decibels(decibels);
    }

    pub fn compute_gains(&self, amp_epsilon: f32) -> (f32, f32) {
        let global_gain = self.volume.amp_clamped(amp_epsilon);

        let (mut gain_0, mut gain_1) = self.fade_curve.compute_gains(self.crossfade);

        gain_0 *= global_gain;
        gain_1 *= global_gain;

        if gain_0 > 0.99999 && gain_0 < 1.00001 {
            gain_0 = 1.0;
        }
        if gain_1 > 0.99999 && gain_1 < 1.00001 {
            gain_1 = 1.0;
        }

        (gain_0, gain_1)
    }
}

impl Default for CrossfadeNode {
    fn default() -> Self {
        Self {
            volume: Volume::default(),
            crossfade: -1.0,
            fade_curve: FadeCurve::default(),
            smooth_seconds: DEFAULT_SMOOTH_SECONDS,
            min_gain: DEFAULT_AMP_EPSILON,
        }
    }
}

impl AudioNode for CrossfadeNode {
    type Configuration = CrossfadeNodeConfig;

    fn info(&self, config: &Self::Configuration) -> AudioNodeInfo {
        let num_channels = config.channels.get().get();

        AudioNodeInfo::new()
            .debug_name("crossfade")
            .channel_config(ChannelConfig {
                num_inputs: ChannelCount::new(num_channels * 2).unwrap_or_else(|| {
                    panic!(
                        "CrossfadeNodeConfig::channels cannot be greater than 32, got {}",
                        num_channels
                    )
                }),
                num_outputs: config.channels.get(),
            })
    }

    fn construct_processor(
        &self,
        _config: &Self::Configuration,
        cx: ConstructProcessorContext,
    ) -> impl AudioNodeProcessor {
        let min_gain = self.min_gain.max(0.0);

        let (gain_0, gain_1) = self.compute_gains(self.min_gain);

        Processor {
            gain_0: SmoothedParam::new(
                gain_0,
                SmootherConfig {
                    smooth_seconds: self.smooth_seconds,
                    ..Default::default()
                },
                cx.stream_info.sample_rate,
            ),
            gain_1: SmoothedParam::new(
                gain_1,
                SmootherConfig {
                    smooth_seconds: self.smooth_seconds,
                    ..Default::default()
                },
                cx.stream_info.sample_rate,
            ),
            params: *self,
            prev_block_was_silent: true,
            min_gain,
        }
    }
}

struct Processor {
    gain_0: SmoothedParam,
    gain_1: SmoothedParam,

    params: CrossfadeNode,

    prev_block_was_silent: bool,
    min_gain: f32,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        info: &ProcInfo,
        buffers: ProcBuffers,
        events: &mut ProcEvents,
        extra: &mut ProcExtra,
    ) -> ProcessStatus {
        let mut updated = false;
        for mut patch in events.drain_patches::<CrossfadeNode>() {
            match &mut patch {
                CrossfadeNodePatch::Crossfade(p) => {
                    if *p <= -0.99999 {
                        *p = -1.0;
                    } else if *p >= 0.99999 {
                        *p = 1.0;
                    }
                }
                CrossfadeNodePatch::SmoothSeconds(seconds) => {
                    self.gain_0.set_smooth_seconds(*seconds, info.sample_rate);
                    self.gain_1.set_smooth_seconds(*seconds, info.sample_rate);
                }
                CrossfadeNodePatch::MinGain(min_gain) => {
                    self.min_gain = (*min_gain).max(0.0);
                }
                _ => {}
            }

            self.params.apply(patch);
            updated = true;
        }

        if updated {
            let (gain_0, gain_1) = self.params.compute_gains(self.min_gain);
            self.gain_0.set_value(gain_0);
            self.gain_1.set_value(gain_1);

            if self.prev_block_was_silent {
                // Previous block was silent, so no need to smooth.
                self.gain_0.reset();
                self.gain_1.reset();
            }
        }

        self.prev_block_was_silent = false;

        let channels = buffers.outputs.len();

        let gain_0_silent = self.gain_0.has_settled_at_or_below(self.min_gain);
        let gain_1_silent = self.gain_1.has_settled_at_or_below(self.min_gain);
        let has_settled = self.gain_0.has_settled() && self.gain_1.has_settled();

        if (gain_0_silent && gain_1_silent)
            || info
                .in_silence_mask
                .all_channels_silent(buffers.inputs.len())
        {
            self.gain_0.reset();
            self.gain_1.reset();
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        let mut out_silence_mask = SilenceMask::NONE_SILENT;

        if has_settled {
            if self.params.crossfade == -1.0 && self.gain_0.target_value() == 1.0 {
                // Simply copy input 0 to output
                for (ch_i, (in_ch, out_ch)) in buffers.inputs[..channels]
                    .iter()
                    .zip(buffers.outputs.iter_mut())
                    .enumerate()
                {
                    if info.in_silence_mask.is_channel_silent(ch_i) {
                        out_silence_mask.set_channel(ch_i, true);

                        if !info.out_silence_mask.is_channel_silent(ch_i) {
                            out_ch.fill(0.0);
                        }
                    } else {
                        out_ch.copy_from_slice(in_ch);
                    }
                }

                return ProcessStatus::OutputsModifiedWithMask(MaskType::Silence(out_silence_mask));
            } else if self.params.crossfade == 1.0 && self.gain_1.target_value() == 1.0 {
                // Simply copy input 1 to output
                for (ch_i, (in_ch, out_ch)) in buffers.inputs[channels..]
                    .iter()
                    .zip(buffers.outputs.iter_mut())
                    .enumerate()
                {
                    if info.in_silence_mask.is_channel_silent(channels + ch_i) {
                        out_silence_mask.set_channel(ch_i, true);

                        if !info.out_silence_mask.is_channel_silent(ch_i) {
                            out_ch.fill(0.0);
                        }
                    } else {
                        out_ch.copy_from_slice(in_ch);
                    }
                }

                return ProcessStatus::OutputsModifiedWithMask(MaskType::Silence(out_silence_mask));
            }
        }

        match channels {
            1 => {
                // Provide an optimized loop for mono

                if has_settled {
                    for ((&in0_s, &in1_s), out_s) in buffers.inputs[0]
                        .iter()
                        .zip(buffers.inputs[1].iter())
                        .zip(buffers.outputs[0].iter_mut())
                    {
                        *out_s = (in0_s * self.gain_0.target_value())
                            + (in1_s * self.gain_1.target_value());
                    }
                } else {
                    for ((&in0_s, &in1_s), out_s) in buffers.inputs[0]
                        .iter()
                        .zip(buffers.inputs[1].iter())
                        .zip(buffers.outputs[0].iter_mut())
                    {
                        let gain_0 = self.gain_0.next_smoothed();
                        let gain_1 = self.gain_1.next_smoothed();

                        *out_s = (in0_s * gain_0) + (in1_s * gain_1);
                    }

                    self.gain_0.settle();
                    self.gain_1.settle();
                }
            }
            2 => {
                // Provide an optimized loop for stereo

                let in0_l = &buffers.inputs[0][..info.frames];
                let in0_r = &buffers.inputs[1][..info.frames];
                let in1_l = &buffers.inputs[2][..info.frames];
                let in1_r = &buffers.inputs[3][..info.frames];

                let (out_l, out_r) = buffers.outputs.split_first_mut().unwrap();
                let out_l = &mut out_l[..info.frames];
                let out_r = &mut out_r[0][..info.frames];

                if has_settled {
                    for i in 0..info.frames {
                        out_l[i] = (in0_l[i] * self.gain_0.target_value())
                            + (in1_l[i] * self.gain_1.target_value());
                        out_r[i] = (in0_r[i] * self.gain_0.target_value())
                            + (in1_r[i] * self.gain_1.target_value());
                    }
                } else {
                    for i in 0..info.frames {
                        let gain_0 = self.gain_0.next_smoothed();
                        let gain_1 = self.gain_1.next_smoothed();

                        out_l[i] = (in0_l[i] * gain_0) + (in1_l[i] * gain_1);
                        out_r[i] = (in0_r[i] * gain_0) + (in1_r[i] * gain_1);
                    }

                    self.gain_0.settle();
                    self.gain_1.settle();
                }
            }
            _ => {
                if has_settled {
                    for (ch_i, ((in0_ch, in1_ch), out_ch)) in buffers.inputs[0..channels]
                        .iter()
                        .zip(buffers.inputs[channels..].iter())
                        .zip(buffers.outputs.iter_mut())
                        .enumerate()
                    {
                        let in0_ch_silent = info.in_silence_mask.is_channel_silent(ch_i);
                        let in1_ch_silent = info.in_silence_mask.is_channel_silent(channels + ch_i);

                        let channel_silent = (in0_ch_silent && in1_ch_silent)
                            || (gain_0_silent && in1_ch_silent)
                            || (gain_1_silent && in0_ch_silent);

                        if channel_silent {
                            out_silence_mask.set_channel(ch_i, true);

                            if !info.out_silence_mask.is_channel_silent(ch_i) {
                                out_ch.fill(0.0);
                            }
                        } else {
                            for ((&in0_s, &in1_s), out_s) in
                                in0_ch.iter().zip(in1_ch.iter()).zip(out_ch.iter_mut())
                            {
                                *out_s = (in0_s * self.gain_0.target_value())
                                    + (in1_s * self.gain_1.target_value());
                            }
                        }
                    }
                } else {
                    let [gain_0_buf, gain_1_buf] = extra.scratch_buffers.channels_mut::<2>();
                    self.gain_0
                        .process_into_buffer(&mut gain_0_buf[..info.frames]);
                    self.gain_1
                        .process_into_buffer(&mut gain_1_buf[..info.frames]);

                    for (ch_i, ((in0_ch, in1_ch), out_ch)) in buffers.inputs[0..channels]
                        .iter()
                        .zip(buffers.inputs[channels..].iter())
                        .zip(buffers.outputs.iter_mut())
                        .enumerate()
                    {
                        let in0_ch_silent = info.in_silence_mask.is_channel_silent(ch_i);
                        let in1_ch_silent = info.in_silence_mask.is_channel_silent(channels + ch_i);

                        let channel_silent = (in0_ch_silent && in1_ch_silent)
                            || (gain_0_silent && in1_ch_silent)
                            || (gain_1_silent && in0_ch_silent);

                        if channel_silent {
                            out_silence_mask.set_channel(ch_i, true);

                            if !info.out_silence_mask.is_channel_silent(ch_i) {
                                out_ch.fill(0.0);
                            }
                        } else {
                            for ((((&in0_s, &in1_s), &gain0_s), &gain1_s), out_s) in in0_ch
                                .iter()
                                .zip(in1_ch.iter())
                                .zip(gain_0_buf.iter())
                                .zip(gain_1_buf.iter())
                                .zip(out_ch.iter_mut())
                            {
                                *out_s = (in0_s * gain0_s) + (in1_s * gain1_s);
                            }
                        }
                    }

                    self.gain_0.settle();
                    self.gain_1.settle();
                }
            }
        }

        return ProcessStatus::OutputsModifiedWithMask(MaskType::Silence(out_silence_mask));
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        self.gain_0.update_sample_rate(stream_info.sample_rate);
        self.gain_1.update_sample_rate(stream_info.sample_rate);
    }
}
