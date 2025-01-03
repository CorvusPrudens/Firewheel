use firewheel_core::{
    dsp::{
        decibel::normalized_volume_to_raw_gain,
        smoothing_filter::{self, DEFAULT_SETTLE_EPSILON, DEFAULT_SMOOTH_SECONDS},
    },
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, NodeEventType, ProcInfo,
        ProcessStatus,
    },
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct VolumeNode {
    normalized_volume: f32,
}

impl VolumeNode {
    /// The ID of the volume parameter.
    pub const PARAM_VOLUME: u32 = 0;

    /// Create a new volume node.
    ///
    /// * `normalized_volume` - The percent volume where `0.0` is mute and `1.0` is unity gain.
    pub fn new(normalized_volume: f32) -> Self {
        let normalized_volume = normalized_volume.max(0.0);

        Self { normalized_volume }
    }

    /// Get the current percent volume where `0.0` is mute and `1.0` is unity gain.
    pub fn normalized_volume(&self) -> f32 {
        self.normalized_volume
    }

    /// Return an event type to set the volume parameter.
    ///
    /// * `normalized_volume` - The percent volume where `0.0` is mute and `1.0` is unity gain.
    /// * `smoothing` - Set this to `false` to have the node immediately jump to this new
    /// value without smoothing (may cause audible clicking or stair-stepping artifacts). This
    /// can be useful to preserve transients when playing a new sound at a different volume.
    pub fn set_volume(&mut self, normalized_volume: f32, smoothing: bool) -> NodeEventType {
        self.normalized_volume = normalized_volume.max(0.0);
        NodeEventType::F32Param {
            id: Self::PARAM_VOLUME,
            value: normalized_volume,
            smoothing,
        }
    }
}

impl AudioNode for VolumeNode {
    fn debug_name(&self) -> &'static str {
        "volume"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::MONO,
            num_max_supported_inputs: ChannelCount::MAX,
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::MAX,
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::STEREO,
            },
            equal_num_ins_and_outs: true,
            updates: false,
            uses_events: true,
        }
    }

    fn activate(
        &mut self,
        stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        let raw_gain = normalized_volume_to_raw_gain(self.normalized_volume);

        Ok(Box::new(VolumeProcessor {
            smooth_filter_coeff: smoothing_filter::Coeff::new(
                stream_info.sample_rate,
                DEFAULT_SMOOTH_SECONDS,
            ),
            filter_target: raw_gain,
            gain: raw_gain,
        }))
    }
}

struct VolumeProcessor {
    smooth_filter_coeff: smoothing_filter::Coeff,
    filter_target: f32,

    gain: f32,
}

impl AudioNodeProcessor for VolumeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        for msg in events {
            if let NodeEventType::F32Param {
                id,
                value,
                smoothing,
            } = msg
            {
                if *id != VolumeNode::PARAM_VOLUME {
                    continue;
                }

                self.filter_target = normalized_volume_to_raw_gain(*value);

                if self.filter_target < 0.00001 {
                    self.filter_target = 0.0;
                } else if self.filter_target > 0.99999 && self.filter_target < 1.00001 {
                    self.filter_target = 1.0
                }

                if !*smoothing {
                    self.gain = self.filter_target;
                }
            }
        }

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All channels are silent, so there is no need to process. Also reset
            // the filter since it doesn't need to smooth anything.
            self.gain = self.filter_target;

            return ProcessStatus::ClearAllOutputs;
        }

        if self.gain == self.filter_target {
            if self.gain == 0.0 {
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
                &mut proc_info.scratch_buffers[0][..proc_info.frames],
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
                    .zip(proc_info.scratch_buffers[0][..proc_info.frames].iter())
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
}

impl Into<Box<dyn AudioNode>> for VolumeNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
