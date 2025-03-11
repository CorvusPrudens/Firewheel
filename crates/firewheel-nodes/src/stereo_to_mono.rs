use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    event::NodeEventList,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, EmptyConfig, ProcBuffers, ProcInfo,
        ProcessStatus,
    },
};

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct StereoToMonoNode;

impl AudioNode for StereoToMonoNode {
    type Configuration = EmptyConfig;

    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo::new()
            .debug_name("stereo_to_mono")
            .channel_config(ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::MONO,
            })
            .uses_events(false)
    }

    fn processor(
        &self,
        _config: &Self::Configuration,
        _stream_info: &firewheel_core::StreamInfo,
    ) -> impl AudioNodeProcessor {
        StereoToMonoProcessor
    }
}

struct StereoToMonoProcessor;

impl AudioNodeProcessor for StereoToMonoProcessor {
    fn process(
        &mut self,
        buffers: ProcBuffers,
        proc_info: &ProcInfo,
        _events: NodeEventList,
    ) -> ProcessStatus {
        if proc_info.in_silence_mask.all_channels_silent(2)
            || buffers.inputs.len() < 2
            || buffers.outputs.is_empty()
        {
            return ProcessStatus::ClearAllOutputs;
        }

        for (out_s, (&in1, &in2)) in buffers.outputs[0]
            .iter_mut()
            .zip(buffers.inputs[0].iter().zip(buffers.inputs[1].iter()))
        {
            *out_s = (in1 + in2) * 0.5;
        }

        ProcessStatus::outputs_not_silent()
    }
}
