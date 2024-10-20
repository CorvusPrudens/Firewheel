use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    StreamInfo,
};

pub struct StereoToMonoNode;

impl AudioNode for StereoToMonoNode {
    fn debug_name(&self) -> &'static str {
        "stereo_to_mono"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: 2,
            num_max_supported_inputs: 2,
            num_min_supported_outputs: 1,
            num_max_supported_outputs: 1,
            updates: false,
        }
    }

    fn activate(
        &mut self,
        _stream_info: StreamInfo,
        _num_inputs: usize,
        _num_outputs: usize,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(StereoToMonoProcessor))
    }
}

struct StereoToMonoProcessor;

impl AudioNodeProcessor for StereoToMonoProcessor {
    fn process(
        &mut self,
        _frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo,
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

impl Into<Box<dyn AudioNode>> for StereoToMonoNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
