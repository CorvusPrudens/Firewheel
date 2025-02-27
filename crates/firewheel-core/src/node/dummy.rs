use crate::{channel_config::ChannelConfig, event::NodeEventList, StreamInfo};

use super::{
    AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus, ScratchBuffers,
};

/// A "dummy" [`AudioNode`], a node which does nothing.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DummyNode;

/// The configuration for a [`DummyNode`], a node which does nothing.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DummyNodeConfig {
    pub channel_config: ChannelConfig,
}

impl AudioNode for DummyNode {
    type Configuration = DummyNodeConfig;

    fn info(&self, config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo::new()
            .debug_name("dummy")
            .channel_config(config.channel_config)
            .uses_events(false)
    }

    fn processor(
        &self,
        _config: &Self::Configuration,
        _stream_info: &StreamInfo,
    ) -> impl AudioNodeProcessor {
        DummyProcessor
    }
}

struct DummyProcessor;

impl AudioNodeProcessor for DummyProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        _proc_info: &ProcInfo,
        _scratch_buffers: ScratchBuffers,
    ) -> ProcessStatus {
        ProcessStatus::Bypass
    }
}
