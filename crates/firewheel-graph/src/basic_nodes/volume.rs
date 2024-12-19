use firewheel_core::{
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, EventData, NodeEventIter, ProcInfo,
        ProcessStatus,
    },
    param::{AudioParam, Continuous},
    ChannelConfig, ChannelCount, StreamInfo,
};

#[derive(AudioParam, Clone)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct VolumeNode(pub Continuous<f32>);

impl VolumeNode {
    pub fn new(level: f32) -> Self {
        Self(Continuous::new(level))
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
        _stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(VolumeProcessor {
            params: self.clone(),
        }))
    }
}

struct VolumeProcessor {
    params: VolumeNode,
}

impl AudioNodeProcessor for VolumeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let samples = proc_info.samples;

        for msg in events {
            if let EventData::Parameter(param) = msg {
                let _ = self.params.patch(&mut param.data, &param.path);
            }
        }

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All channels are silent, so there is no need to process. Also reset
            // the filter since it doesn't need to smooth anything.
            // self.gain_smoother.reset(self.gain_smoother.target_value());

            return ProcessStatus::ClearAllOutputs;
        }

        let seconds = proc_info.clock_seconds;
        let gain = self.params.0.value_at(seconds);
        let is_active = self.params.0.is_active(seconds);

        if !is_active {
            if gain < 0.00001 {
                // Muted, so there is no need to process.
                return ProcessStatus::ClearAllOutputs;
            } else if gain > 0.99999 && gain < 1.00001 {
                // Unity gain, there is no need to process.
                return ProcessStatus::Bypass;
            }
        }

        // // Hint to the compiler to optimize loop.
        // let samples = samples.min(gain.values.len());

        // Provide an optimized loop for stereo.
        if inputs.len() == 2 && outputs.len() == 2 {
            // Hint to the compiler to optimize loop.
            let samples = samples
                .min(outputs[0].len())
                .min(outputs[1].len())
                .min(inputs[0].len())
                .min(inputs[1].len());

            for i in 0..inputs[0].len() {
                let seconds = seconds
                    + firewheel_core::clock::ClockSeconds(i as f64 * proc_info.sample_rate_recip);
                self.params.tick(seconds);

                let gain = self.params.0.get();

                outputs[0][i] = inputs[0][i] * gain;
                outputs[1][i] = inputs[1][i] * gain;
            }

            return ProcessStatus::outputs_modified(proc_info.in_silence_mask);
        }

        for (i, (output, input)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
            // Hint to the compiler to optimize loop.
            let samples = samples.min(output.len()).min(input.len());

            if proc_info.in_silence_mask.is_channel_silent(i) {
                if !proc_info.out_silence_mask.is_channel_silent(i) {
                    output[..samples].fill(0.0);
                }
                continue;
            }

            for i in 0..samples {
                let seconds = seconds
                    + firewheel_core::clock::ClockSeconds(i as f64 * proc_info.sample_rate_recip);
                self.params.tick(seconds);

                let gain = self.params.0.get();

                output[i] = input[i] * gain;
            }
        }

        ProcessStatus::outputs_modified(proc_info.in_silence_mask)
    }
}

impl Into<Box<dyn AudioNode>> for VolumeNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
