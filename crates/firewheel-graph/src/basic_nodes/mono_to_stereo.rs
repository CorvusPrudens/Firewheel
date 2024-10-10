use firewheel_core::node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo};

pub struct MonoToStereoNode;

impl<C> AudioNode<C> for MonoToStereoNode {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: 1,
            num_max_supported_inputs: 1,
            num_min_supported_outputs: 2,
            num_max_supported_outputs: 2,
        }
    }

    fn activate(
        &mut self,
        _sample_rate: u32,
        _max_block_frames: usize,
        _num_inputs: usize,
        _num_outputs: usize,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn std::error::Error>> {
        Ok(Box::new(MonoToStereoProcessor))
    }
}

struct MonoToStereoProcessor;

impl<C> AudioNodeProcessor<C> for MonoToStereoProcessor {
    fn process(
        &mut self,
        _frames: usize,
        proc_info: ProcInfo<C>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) {
        if proc_info.in_silence_mask.is_channel_silent(0) {
            firewheel_core::util::clear_all_outputs(outputs, proc_info.out_silence_mask);
            return;
        }

        let input = inputs[0];
        outputs[0].copy_from_slice(input);
        outputs[1].copy_from_slice(input);
    }
}