use firewheel_core::{
    channel_config::ChannelConfig,
    event::NodeEventList,
    node::{AudioNodeConstructor, AudioNodeProcessor, ProcInfo, ProcessStatus},
    StreamInfo,
};

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DummyConfig {
    pub channel_config: ChannelConfig,
}

impl AudioNodeConstructor for DummyConfig {
    fn debug_name(&self) -> &'static str {
        "dummy"
    }

    fn channel_config(&self) -> ChannelConfig {
        self.channel_config
    }

    fn uses_events(&self) -> bool {
        false
    }

    fn processor(&self, _stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(DummyProcessor)
    }
}

pub struct DummyProcessor;

impl AudioNodeProcessor for DummyProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        _proc_info: ProcInfo,
    ) -> ProcessStatus {
        ProcessStatus::Bypass
    }
}
