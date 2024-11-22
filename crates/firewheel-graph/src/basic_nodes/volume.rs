use firewheel_core::{
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, NodeEventType, ProcInfo,
        ProcessStatus,
    },
    param::{range::percent_volume_to_raw_gain, smoother::ParamSmoother},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct VolumeNode {
    percent_volume: f32,
}

impl VolumeNode {
    /// Create a new volume node.
    ///
    /// * `percent_volume` - The percent volume where `0.0` is mute and `100.0` is unity gain.
    pub fn new(percent_volume: f32) -> Self {
        let percent_volume = percent_volume.max(0.0);

        Self { percent_volume }
    }

    /// Get the current percent volume where `0.0` is mute and `100.0` is unity gain.
    pub fn percent_volume(&self) -> f32 {
        self.percent_volume
    }

    /// Return an event type to set the volume parameter.
    ///
    /// * `percent_volume` - The percent volume where `0.0` is mute and `100.0` is unity gain.
    /// * `no_smoothing` - Set this to `true` to have the node immediately jump to this new
    /// value without smoothing (may cause audible clicking or stair-stepping artifacts). This
    /// can be useful to preserve transients when playing a new sound at a different volume.
    pub fn set_volume(&mut self, percent_volume: f32, no_smoothing: bool) -> NodeEventType {
        self.percent_volume = percent_volume.max(0.0);
        NodeEventType::FloatParam {
            id: 0,
            value: percent_volume,
            no_smoothing,
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
        let raw_gain = percent_volume_to_raw_gain(self.percent_volume);

        Ok(Box::new(VolumeProcessor {
            gain_smoother: ParamSmoother::new(
                raw_gain,
                stream_info.sample_rate,
                stream_info.max_block_samples as usize,
                Default::default(),
            ),
        }))
    }
}

struct VolumeProcessor {
    gain_smoother: ParamSmoother,
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
            if let NodeEventType::FloatParam {
                id,
                value,
                no_smoothing,
            } = msg
            {
                if *id != 0 {
                    continue;
                }
                let raw_gain = percent_volume_to_raw_gain(*value);
                self.gain_smoother
                    .set_with_smoothing(raw_gain, *no_smoothing);
            }
        }

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All channels are silent, so there is no need to process. Also reset
            // the filter since it doesn't need to smooth anything.
            self.gain_smoother.reset(self.gain_smoother.target_value());

            return ProcessStatus::ClearAllOutputs;
        }

        let gain = self.gain_smoother.process(samples);

        if !gain.is_smoothing() {
            if gain.values[0] < 0.00001 {
                // Muted, so there is no need to process.
                return ProcessStatus::ClearAllOutputs;
            } else if gain.values[0] > 0.99999 && gain.values[0] < 1.00001 {
                // Unity gain, there is no need to process.
                return ProcessStatus::Bypass;
            }
        }

        // Hint to the compiler to optimize loop.
        assert!(samples <= gain.values.len());

        // Provide an optimized loop for stereo.
        if inputs.len() == 2 && outputs.len() == 2 {
            // Hint to the compiler to optimize loop.
            assert!(samples <= outputs[0].len());
            assert!(samples <= outputs[1].len());
            assert!(samples <= inputs[0].len());
            assert!(samples <= inputs[1].len());

            for i in 0..samples {
                outputs[0][i] = inputs[0][i] * gain[i];
                outputs[1][i] = inputs[1][i] * gain[i];
            }

            return ProcessStatus::outputs_modified(proc_info.in_silence_mask);
        }

        for (i, (output, input)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
            if proc_info.in_silence_mask.is_channel_silent(i) {
                if !proc_info.out_silence_mask.is_channel_silent(i) {
                    output[..samples].fill(0.0);
                }
                continue;
            }

            // Hint to the compiler to optimize loop.
            assert!(samples <= input.len());

            for i in 0..samples {
                output[i] = input[i] * gain[i];
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
