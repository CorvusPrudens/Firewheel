use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct MonoToStereoNode;

impl<C> AudioNode<C> for MonoToStereoNode {
    fn debug_name(&self) -> &'static str {
        "mono_to_stereo"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::MONO,
            num_max_supported_inputs: ChannelCount::MONO,
            num_min_supported_outputs: ChannelCount::STEREO,
            num_max_supported_outputs: ChannelCount::STEREO,
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::MONO,
                num_outputs: ChannelCount::STEREO,
            },
            equal_num_ins_and_outs: false,
            updates: false,
        }
    }

    fn activate(
        &mut self,
        _stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn std::error::Error>> {
        Ok(Box::new(MonoToStereoProcessor))
    }
}

struct MonoToStereoProcessor;

impl<C> AudioNodeProcessor<C> for MonoToStereoProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo<C>,
    ) -> ProcessStatus {
        if proc_info.in_silence_mask.is_channel_silent(0) {
            return ProcessStatus::NoOutputsModified;
        }

        let input = inputs[0];
        outputs[0][..proc_info.frames].copy_from_slice(&input[..proc_info.frames]);
        outputs[1][..proc_info.frames].copy_from_slice(&input[..proc_info.frames]);

        ProcessStatus::all_outputs_filled()
    }
}

impl<C> Into<Box<dyn AudioNode<C>>> for MonoToStereoNode {
    fn into(self) -> Box<dyn AudioNode<C>> {
        Box::new(self)
    }
}
