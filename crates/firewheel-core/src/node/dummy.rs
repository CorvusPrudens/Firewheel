use std::any::Any;

use crate::{channel_config::ChannelConfig, event::NodeEventList, StreamInfo};

use super::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcBuffers, ProcInfo, ProcessStatus};

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
        _custom_state: &mut Option<Box<dyn Any>>,
    ) -> impl AudioNodeProcessor {
        DummyProcessor
    }
}

struct DummyProcessor;

impl AudioNodeProcessor for DummyProcessor {
    fn process(
        &mut self,
        _buffers: ProcBuffers,
        _proc_info: &ProcInfo,
        _events: NodeEventList,
    ) -> ProcessStatus {
        ProcessStatus::Bypass
    }
}
