use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    event::NodeEventList,
    node::{AudioNodeConstructor, AudioNodeProcessor, ProcInfo, ProcessStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StereoToMonoNode;

impl AudioNodeConstructor for StereoToMonoNode {
    fn debug_name(&self) -> &'static str {
        "stereo_to_mono"
    }

    fn channel_config(&self) -> ChannelConfig {
        ChannelConfig {
            num_inputs: ChannelCount::STEREO,
            num_outputs: ChannelCount::MONO,
        }
    }

    fn uses_events(&self) -> bool {
        false
    }

    fn processor(&self, _stream_info: &firewheel_core::StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(StereoToMonoProcessor {})
    }
}

struct StereoToMonoProcessor;

impl AudioNodeProcessor for StereoToMonoProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        if proc_info.in_silence_mask.all_channels_silent(2)
            || inputs.len() < 2
            || outputs.is_empty()
        {
            return ProcessStatus::ClearAllOutputs;
        }

        for (out_s, (&in1, &in2)) in outputs[0]
            .iter_mut()
            .zip(inputs[0].iter().zip(inputs[1].iter()))
        {
            *out_s = (in1 + in2) * 0.5;
        }

        ProcessStatus::outputs_not_silent()
    }
}
