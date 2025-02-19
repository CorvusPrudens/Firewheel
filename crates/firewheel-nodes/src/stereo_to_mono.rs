use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    event::NodeEventList,
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
};

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StereoToMonoNode;

impl AudioNodeConstructor for StereoToMonoNode {
    type Configuration = ();

    fn info(&self, _: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "stereo_to_mono",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::MONO,
            },
            uses_events: false,
        }
    }

    fn processor(
        &self,
        _: &Self::Configuration,
        _stream_info: &firewheel_core::StreamInfo,
    ) -> impl AudioNodeProcessor {
        StereoToMonoProcessor
    }
}

struct StereoToMonoProcessor;

impl AudioNodeProcessor for StereoToMonoProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
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
