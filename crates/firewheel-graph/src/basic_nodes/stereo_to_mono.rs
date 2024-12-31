use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    node::{AudioNodeProcessor, NodeEventIter, NodeHandle, NodeID, ProcInfo, ProcessStatus},
};

use crate::FirewheelCtx;

pub struct StereoToMonoNode {
    handle: NodeHandle,
}

impl StereoToMonoNode {
    pub fn new(cx: &mut FirewheelCtx) -> Self {
        let handle = cx.add_node(
            "stereo_to_mono",
            ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::MONO,
            },
            false,
            Box::new(StereoToMonoProcessor {}),
        );

        Self { handle }
    }

    /// The ID of this node
    pub fn id(&self) -> NodeID {
        self.handle.id
    }
}

struct StereoToMonoProcessor;

impl AudioNodeProcessor for StereoToMonoProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _events: NodeEventIter,
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
