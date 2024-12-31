use crate::FirewheelCtx;
use firewheel_core::{
    channel_config::ChannelConfig,
    node::{AudioNodeProcessor, NodeEventIter, NodeHandle, NodeID, ProcInfo, ProcessStatus},
};

pub struct DummyAudioNode {
    handle: NodeHandle,
}

impl DummyAudioNode {
    pub fn new(channel_config: ChannelConfig, cx: &mut FirewheelCtx) -> Self {
        let handle = cx.add_node(
            "dummy",
            channel_config,
            false,
            Box::new(DummyAudioNodeProcessor),
        );

        Self { handle }
    }

    /// The ID of this node
    pub fn id(&self) -> NodeID {
        self.handle.id
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
