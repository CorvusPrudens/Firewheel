use firewheel_core::{
    clock::ClockSeconds,
    dsp::decibel::normalized_volume_to_raw_gain,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, NodeEventType, ProcInfo,
        ProcessStatus,
    },
    param::{AudioParam, ParamEvent, Timeline},
    ChannelConfig, ChannelCount, StreamInfo,
};

#[derive(AudioParam, Clone)]
#[cfg_attr(feature = "bevy_ecs", derive(bevy_ecs::component::Component))]
pub struct VolumeNode(pub Timeline<f32>);

impl VolumeNode {
    pub fn new(volume: f32) -> Self {
        Self(Timeline::new(volume))
    }
}

impl AudioNode for VolumeNode {
    fn debug_name(&self) -> &'static str {
        "volume"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::MONO,
            num_max_supported_inputs: ChannelCount::MAX,
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::MAX,
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::STEREO,
            },
            equal_num_ins_and_outs: true,
            updates: false,
            uses_events: true,
        }
    }

    fn activate(
        &mut self,
        _: &StreamInfo,
        _: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(VolumeProcessor {
            params: self.clone(),
        }))
    }
}

struct VolumeProcessor {
    params: VolumeNode,
}

impl AudioNodeProcessor for VolumeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let frames = proc_info.frames;

        for msg in events {
            if let NodeEventType::Custom(custom) = msg {
                if let Some(param) = custom.downcast_ref::<ParamEvent>() {
                    let _ = self.params.patch(&param.data, &param.path);
                }
            }
        }

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            return ProcessStatus::ClearAllOutputs;
        }

        let now = proc_info.clock_seconds;
        let end = now + ClockSeconds(proc_info.sample_rate_recip * proc_info.frames as f64);

        if !self.params.0.active_within(now, end) {
            if self.params.0.get() == 0.0 {
                return ProcessStatus::ClearAllOutputs;
            } else if self.params.0.get() == 1.0 {
                return ProcessStatus::Bypass;
            }
        }

        for (i, (output, input)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
            // Hint to the compiler to optimize loop.
            let samples = frames.min(output.len()).min(input.len());

            if proc_info.in_silence_mask.is_channel_silent(i) {
                if !proc_info.out_silence_mask.is_channel_silent(i) {
                    output[..samples].fill(0.0);
                }
                continue;
            }

            let mut gain = 0f32;

            for i in 0..samples {
                if i % 32 == 0 {
                    let seconds = now
                        + firewheel_core::clock::ClockSeconds(
                            i as f64 * proc_info.sample_rate_recip,
                        );
                    self.params.tick(seconds);

                    gain = normalized_volume_to_raw_gain(self.params.0.get());
                }

                output[i] = input[i] * gain;
            }
        }

        ProcessStatus::outputs_modified(proc_info.in_silence_mask)
    }
}

impl Into<Box<dyn AudioNode>> for VolumeNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
