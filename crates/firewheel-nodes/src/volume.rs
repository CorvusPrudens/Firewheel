use firewheel_core::{
    channel_config::{ChannelConfig, NonZeroChannelCount},
    dsp::{
        decibel::normalized_volume_to_raw_gain,
        smoothing_filter::{self, DEFAULT_SETTLE_EPSILON, DEFAULT_SMOOTH_SECONDS},
    },
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeParams {
    /// The percent volume where `0.0` is mute and `1.0` is unity gain.
    pub normalized_volume: f32,
    /// The number of channels in this node.
    pub channels: NonZeroChannelCount,
}

impl VolumeParams {
    /// The ID of the volume parameter.
    pub const ID_VOLUME: u32 = 0;

    /// Return an event type to sync the volume parameter.
    pub fn sync_volume_event(&self) -> NodeEventType {
        NodeEventType::F32Param {
            id: Self::ID_VOLUME,
            value: self.normalized_volume,
        }
    }
}

impl Default for VolumeParams {
    fn default() -> Self {
        Self {
            normalized_volume: 1.0,
            channels: NonZeroChannelCount::STEREO,
        }
    }
}

impl AudioNodeConstructor for VolumeParams {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "volume",
            channel_config: ChannelConfig {
                num_inputs: self.channels.get(),
                num_outputs: self.channels.get(),
            },
            uses_events: true,
        }
    }

    fn processor(
        &mut self,
        stream_info: &firewheel_core::StreamInfo,
    ) -> Box<dyn AudioNodeProcessor> {
        let gain = normalized_volume_to_raw_gain(self.normalized_volume);

        Box::new(VolumeProcessor {
            smooth_filter_coeff: smoothing_filter::Coeff::new(
                stream_info.sample_rate,
                DEFAULT_SMOOTH_SECONDS,
            ),
            filter_target: gain,
            gain,
            prev_block_was_silent: true,
        })
    }
}

struct VolumeProcessor {
    smooth_filter_coeff: smoothing_filter::Coeff,
    filter_target: f32,

    gain: f32,

    prev_block_was_silent: bool,
}

impl AudioNodeProcessor for VolumeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        events.for_each(|event| {
            if let NodeEventType::F32Param { id, value } = event {
                if *id == VolumeParams::ID_VOLUME {
                    self.filter_target = normalized_volume_to_raw_gain(*value);

                    if self.filter_target < 0.00001 {
                        self.filter_target = 0.0;
                    } else if self.filter_target > 0.99999 && self.filter_target < 1.00001 {
                        self.filter_target = 1.0
                    }

                    if self.prev_block_was_silent {
                        // Previous block was silent, so no need to smooth.
                        self.gain = self.filter_target;
                    }
                }
            }
        });

        self.prev_block_was_silent = false;

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All channels are silent, so there is no need to process. Also reset
            // the filter since it doesn't need to smooth anything.
            self.gain = self.filter_target;
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        if self.gain == self.filter_target {
            if self.gain == 0.0 {
                self.prev_block_was_silent = true;
                // Muted, so there is no need to process.
                return ProcessStatus::ClearAllOutputs;
            } else if self.gain == 1.0 {
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
                            *os = is * self.gain;
                        }
                    }
                }

                return ProcessStatus::OutputsModified {
                    out_silence_mask: proc_info.in_silence_mask,
                };
            }
        }

        let mut gain = self.gain;

        if inputs.len() == 1 {
            // Provide an optimized loop for mono.

            let target_times_a = self.filter_target * self.smooth_filter_coeff.a;

            for (os, &is) in outputs[0].iter_mut().zip(inputs[0].iter()) {
                gain = smoothing_filter::process_sample_a(
                    gain,
                    target_times_a,
                    self.smooth_filter_coeff.b,
                );

                *os = is * gain;
            }
        } else if inputs.len() == 2 {
            // Provide an optimized loop for stereo.

            let target_times_a = self.filter_target * self.smooth_filter_coeff.a;

            let in0 = &inputs[0][..proc_info.frames];
            let in1 = &inputs[1][..proc_info.frames];
            let (out0, out1) = outputs.split_first_mut().unwrap();
            let out0 = &mut out0[..proc_info.frames];
            let out1 = &mut out1[0][..proc_info.frames];

            for i in 0..proc_info.frames {
                gain = smoothing_filter::process_sample_a(
                    gain,
                    target_times_a,
                    self.smooth_filter_coeff.b,
                );

                out0[i] = in0[i] * gain;
                out1[i] = in1[i] * gain;
            }
        } else {
            gain = smoothing_filter::process_into_buffer(
                &mut scratch_buffers[0][..proc_info.frames],
                gain,
                self.filter_target,
                self.smooth_filter_coeff,
            );

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

        self.gain = gain;

        if smoothing_filter::has_settled(self.gain, self.filter_target, DEFAULT_SETTLE_EPSILON) {
            self.gain = self.filter_target;
        }

        ProcessStatus::outputs_modified(proc_info.in_silence_mask)
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        self.smooth_filter_coeff =
            smoothing_filter::Coeff::new(stream_info.sample_rate, DEFAULT_SMOOTH_SECONDS);
    }
}
