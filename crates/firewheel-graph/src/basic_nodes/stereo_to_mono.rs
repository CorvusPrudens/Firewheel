use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct StereoToMonoNode;

impl<C> AudioNode<C> for StereoToMonoNode {
    fn debug_name(&self) -> &'static str {
        "stereo_to_mono"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::STEREO,
            num_max_supported_inputs: ChannelCount::STEREO,
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::MONO,
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::MONO,
            },
            equal_num_ins_and_outs: false,
            updates: false,
        }
    }

    fn activate(
        &mut self,
        _stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn std::error::Error>> {
        Ok(Box::new(StereoToMonoProcessor))
    }
}

struct StereoToMonoProcessor;

impl<C> AudioNodeProcessor<C> for StereoToMonoProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo,
        _cx: &mut C,
    ) -> ProcessStatus {
        if proc_info.in_silence_mask.all_channels_silent(2)
            || inputs.len() < 2
            || outputs.is_empty()
        {
            return ProcessStatus::NoOutputsModified;
        }

        for (out_s, (&in1, &in2)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
        {
            *out_s = (in1 + in2) * 0.5;
        }

        ProcessStatus::all_outputs_filled()
    }
}

impl<C> Into<Box<dyn AudioNode<C>>> for StereoToMonoNode {
    fn into(self) -> Box<dyn AudioNode<C>> {
        Box::new(self)
    }
}
