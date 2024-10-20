use std::error::Error;

use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    StreamInfo,
};

pub struct DummyAudioNode;

impl AudioNode for DummyAudioNode {
    fn debug_name(&self) -> &'static str {
        "dummy"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_max_supported_inputs: 64,
            num_max_supported_outputs: 64,
            ..Default::default()
        }
    }

    fn activate(
        &mut self,
        _stream_info: StreamInfo,
        _num_inputs: usize,
        _num_outputs: usize,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn Error>> {
        Ok(Box::new(DummyAudioNodeProcessor))
    }
}

pub struct DummyAudioNodeProcessor;

impl AudioNodeProcessor for DummyAudioNodeProcessor {
    fn process(
        &mut self,
        _frames: usize,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _proc_info: ProcInfo,
    ) -> ProcessStatus {
        ProcessStatus::NoOutputsModified
    }
}

impl Into<Box<dyn AudioNode>> for DummyAudioNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
