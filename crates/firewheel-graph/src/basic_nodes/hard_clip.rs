use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct HardClipNode {
    threshold_gain: f32,
}

impl HardClipNode {
    pub fn new(threshold_db: f32) -> Self {
        Self {
            threshold_gain: firewheel_core::util::db_to_gain_clamped_neg_100_db(threshold_db),
        }
    }
}

impl<C> AudioNode<C> for HardClipNode {
    fn debug_name(&self) -> &'static str {
        "hard_clip"
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
        }
    }

    fn activate(
        &mut self,
        _stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn std::error::Error>> {
        Ok(Box::new(HardClipProcessor {
            threshold_gain: self.threshold_gain,
        }))
    }
}

struct HardClipProcessor {
    threshold_gain: f32,
}

impl<C> AudioNodeProcessor<C> for HardClipProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo<C>,
    ) -> ProcessStatus {
        let samples = proc_info.samples;

        // Provide an optimized loop for stereo.
        if inputs.len() == 2
            && outputs.len() == 2
            && !proc_info.in_silence_mask.any_channel_silent(2)
        {
            // Hint to the compiler to optimize loop.
            assert!(samples <= outputs[0].len());
            assert!(samples <= outputs[1].len());
            assert!(samples <= inputs[0].len());
            assert!(samples <= inputs[1].len());

            for i in 0..samples {
                outputs[0][i] = inputs[0][i]
                    .min(self.threshold_gain)
                    .max(-self.threshold_gain);
                outputs[1][i] = inputs[1][i]
                    .min(self.threshold_gain)
                    .max(-self.threshold_gain);
            }

            return ProcessStatus::all_outputs_filled();
        }

        for (i, (output, input)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
            if proc_info.in_silence_mask.is_channel_silent(i) {
                if !proc_info.out_silence_mask.is_channel_silent(i) {
                    output[..samples].fill(0.0);
                }
                continue;
            }

            for (out_s, in_s) in output.iter_mut().zip(input.iter()) {
                *out_s = in_s.min(self.threshold_gain).max(-self.threshold_gain);
            }
        }

        ProcessStatus::outputs_modified(proc_info.in_silence_mask)
    }
}

impl<C> Into<Box<dyn AudioNode<C>>> for HardClipNode {
    fn into(self) -> Box<dyn AudioNode<C>> {
        Box::new(self)
    }
}
