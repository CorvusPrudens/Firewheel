use firewheel_core::{
    channel_config::{ChannelConfig, NonZeroChannelCount},
    diff::{Diff, Patch},
    dsp::volume::{Volume, DEFAULT_AMP_EPSILON},
    event::NodeEventList,
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus, ScratchBuffers},
    param::smoother::{SmoothedParam, SmootherConfig},
    SilenceMask,
};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct VolumeNodeConfig {
    /// The time in seconds of the internal smoothing filter.
    ///
    /// By default this is set to `0.01` (10ms).
    pub smooth_secs: f32,

    /// The number of input and output channels.
    pub channels: NonZeroChannelCount,

    /// If the resutling amplitude of the volume is less than or equal to this
    /// value, then the amplitude will be clamped to `0.0` (silence).
    pub amp_epsilon: f32,
}

impl Default for VolumeNodeConfig {
    fn default() -> Self {
        Self {
            smooth_secs: 10.0 / 1_000.0,
            channels: NonZeroChannelCount::STEREO,
            amp_epsilon: DEFAULT_AMP_EPSILON,
        }
    }
}

#[derive(Default, Diff, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct VolumeNode {
    pub volume: Volume,
}

impl Patch for VolumeNode {
    fn patch(
        &mut self,
        data: &firewheel_core::event::ParamData,
        _path: &[u32],
    ) -> Result<(), firewheel_core::diff::PatchError> {
        self.volume = data.try_into()?;

        Ok(())
    }
}

impl AudioNode for VolumeNode {
    type Configuration = VolumeNodeConfig;

    fn info(&self, config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo::new()
            .debug_name("volume")
            .channel_config(ChannelConfig {
                num_inputs: config.channels.get(),
                num_outputs: config.channels.get(),
            })
            .uses_events(true)
    }

    fn processor(
        &self,
        config: &Self::Configuration,
        stream_info: &firewheel_core::StreamInfo,
    ) -> impl AudioNodeProcessor {
        let gain = self.volume.amp_clamped(config.amp_epsilon);

        VolumeProcessor {
            params: *self,
            gain: SmoothedParam::new(
                gain,
                SmootherConfig {
                    smooth_secs: config.smooth_secs,
                    ..Default::default()
                },
                stream_info.sample_rate,
            ),
            prev_block_was_silent: true,
            amp_epsilon: config.amp_epsilon,
        }
    }
}

struct VolumeProcessor {
    gain: SmoothedParam,
    params: VolumeNode,

    prev_block_was_silent: bool,
    amp_epsilon: f32,
}

impl AudioNodeProcessor for VolumeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventList,
        proc_info: &ProcInfo,
        scratch_buffers: ScratchBuffers,
    ) -> ProcessStatus {
        if self.params.patch_list(events) {
            let mut gain = self.params.volume.amp_clamped(self.amp_epsilon);
            if gain > 0.99999 && gain < 1.00001 {
                gain = 1.0;
            }
            self.gain.set_value(gain);

            if self.prev_block_was_silent {
                // Previous block was silent, so no need to smooth.
                self.gain.reset();
            }
        }

        self.prev_block_was_silent = false;

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All channels are silent, so there is no need to process. Also reset
            // the filter since it doesn't need to smooth anything.
            self.gain.reset();
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        if !self.gain.is_smoothing() {
            if self.gain.target_value() == 0.0 {
                self.prev_block_was_silent = true;
                // Muted, so there is no need to process.
                return ProcessStatus::ClearAllOutputs;
            } else if self.gain.target_value() == 1.0 {
                // Unity gain, there is no need to process.
                return ProcessStatus::Bypass;
            } else {
                for (ch_i, (out_ch, in_ch)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
                    if proc_info.in_silence_mask.is_channel_silent(ch_i) {
                        if !proc_info.out_silence_mask.is_channel_silent(ch_i) {
                            out_ch.fill(0.0);
                        }
                    } else {
                        for (os, &is) in out_ch.iter_mut().zip(in_ch.iter()) {
                            *os = is * self.gain.target_value();
                        }
                    }
                }

                return ProcessStatus::OutputsModified {
                    out_silence_mask: proc_info.in_silence_mask,
                };
            }
        }

        if inputs.len() == 1 {
            // Provide an optimized loop for mono.
            for (os, &is) in outputs[0].iter_mut().zip(inputs[0].iter()) {
                *os = is * self.gain.next_smoothed();
            }
        } else if inputs.len() == 2 {
            // Provide an optimized loop for stereo.

            let in0 = &inputs[0][..proc_info.frames];
            let in1 = &inputs[1][..proc_info.frames];
            let (out0, out1) = outputs.split_first_mut().unwrap();
            let out0 = &mut out0[..proc_info.frames];
            let out1 = &mut out1[0][..proc_info.frames];

            for i in 0..proc_info.frames {
                let gain = self.gain.next_smoothed();

                out0[i] = in0[i] * gain;
                out1[i] = in1[i] * gain;
            }
        } else {
            self.gain
                .process_into_buffer(&mut scratch_buffers[0][..proc_info.frames]);

            for (ch_i, (out_ch, in_ch)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
                if proc_info.in_silence_mask.is_channel_silent(ch_i) {
                    if !proc_info.out_silence_mask.is_channel_silent(ch_i) {
                        out_ch.fill(0.0);
                    }
                    continue;
                }

                for ((os, &is), &g) in out_ch
                    .iter_mut()
                    .zip(in_ch.iter())
                    .zip(scratch_buffers[0][..proc_info.frames].iter())
                {
                    *os = is * g;
                }
            }
        }

        self.gain.settle();

        ProcessStatus::outputs_modified(SilenceMask::NONE_SILENT)
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        self.gain.update_sample_rate(stream_info.sample_rate);
    }
}
