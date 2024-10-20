use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    StreamInfo,
};

pub struct MonoToStereoNode;

impl AudioNode for MonoToStereoNode {
    fn debug_name(&self) -> &'static str {
        "mono_to_stereo"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: 1,
            num_max_supported_inputs: 1,
            num_min_supported_outputs: 2,
            num_max_supported_outputs: 2,
            updates: false,
        }
    }

    fn activate(
        &mut self,
        _stream_info: StreamInfo,
        _num_inputs: usize,
        _num_outputs: usize,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(MonoToStereoProcessor))
    }
}

struct MonoToStereoProcessor;

impl AudioNodeProcessor for MonoToStereoProcessor {
    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        if proc_info.in_silence_mask.is_channel_silent(0) {
            return ProcessStatus::NoOutputsModified;
        }

        let input = inputs[0];
        outputs[0][..frames].copy_from_slice(&input[..frames]);
        outputs[1][..frames].copy_from_slice(&input[..frames]);

        ProcessStatus::all_outputs_filled()
    }
}

impl Into<Box<dyn AudioNode>> for MonoToStereoNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
