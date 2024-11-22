use std::error::Error;

use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, ProcInfo, ProcessStatus},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct DummyAudioNode;

impl AudioNode for DummyAudioNode {
    fn debug_name(&self) -> &'static str {
        "dummy"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_max_supported_inputs: ChannelCount::MAX,
            num_max_supported_outputs: ChannelCount::MAX,
            uses_events: false,
            ..Default::default()
        }
    }

    fn activate(
        &mut self,
        _stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn Error>> {
        Ok(Box::new(DummyAudioNodeProcessor))
    }
}

pub struct DummyAudioNodeProcessor;

impl AudioNodeProcessor for DummyAudioNodeProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _events: NodeEventIter,
        _proc_info: ProcInfo,
    ) -> ProcessStatus {
        ProcessStatus::ClearAllOutputs
    }
}

impl Into<Box<dyn AudioNode>> for DummyAudioNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
