use std::error::Error;

use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct DummyAudioNode;

impl<C> AudioNode<C> for DummyAudioNode {
    fn debug_name(&self) -> &'static str {
        "dummy"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_max_supported_inputs: ChannelCount::MAX,
            num_max_supported_outputs: ChannelCount::MAX,
            ..Default::default()
        }
    }

    fn activate(
        &mut self,
        _stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn Error>> {
        Ok(Box::new(DummyAudioNodeProcessor))
    }
}

pub struct DummyAudioNodeProcessor;

impl<C> AudioNodeProcessor<C> for DummyAudioNodeProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _proc_info: ProcInfo<C>,
    ) -> ProcessStatus {
        ProcessStatus::NoOutputsModified
    }
}

impl<C> Into<Box<dyn AudioNode<C>>> for DummyAudioNode {
    fn into(self) -> Box<dyn AudioNode<C>> {
        Box::new(self)
    }
}
