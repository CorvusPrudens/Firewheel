use atomic_float::AtomicF32;
use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    param::{range::percent_volume_to_raw_gain, smoother::ParamSmoother},
    ChannelConfig, ChannelCount, StreamInfo,
};
use std::sync::{atomic::Ordering, Arc};

pub struct VolumeNode {
    // TODO: Find a good solution for webassembly.
    raw_gain: Arc<AtomicF32>,
    percent_volume: f32,
}

impl VolumeNode {
    pub fn new(percent_volume: f32) -> Self {
        let percent_volume = percent_volume.max(0.0);

        Self {
            raw_gain: Arc::new(AtomicF32::new(percent_volume_to_raw_gain(percent_volume))),
            percent_volume,
        }
    }

    pub fn percent_volume(&self) -> f32 {
        self.percent_volume
    }

    pub fn set_percent_volume(&mut self, percent_volume: f32) {
        self.raw_gain.store(
            percent_volume_to_raw_gain(percent_volume),
            Ordering::Relaxed,
        );
        self.percent_volume = percent_volume.max(0.0);
    }

    pub fn raw_gain(&self) -> f32 {
        self.raw_gain.load(Ordering::Relaxed)
    }
}

impl<C> AudioNode<C> for VolumeNode {
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
        }
    }

    fn activate(
        &mut self,
        stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn std::error::Error>> {
        Ok(Box::new(VolumeProcessor {
            raw_gain: Arc::clone(&self.raw_gain),
            gain_smoother: ParamSmoother::new(
                self.raw_gain(),
                stream_info.sample_rate,
                stream_info.max_block_frames as usize,
                Default::default(),
            ),
        }))
    }
}

struct VolumeProcessor {
    raw_gain: Arc<AtomicF32>,
    gain_smoother: ParamSmoother,
}

impl<C> AudioNodeProcessor<C> for VolumeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo<C>,
    ) -> ProcessStatus {
        let frames = proc_info.frames;

        let raw_gain = self.raw_gain.load(Ordering::Relaxed);

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All channels are silent, so there is no need to process. Also reset
            // the filter since it doesn't need to smooth anything.
            self.gain_smoother.reset(raw_gain);

            return ProcessStatus::NoOutputsModified;
        }

        let gain = self.gain_smoother.set_and_process(raw_gain, frames);

        if !gain.is_smoothing() && gain.values[0] < 0.00001 {
            // Muted, so there is no need to process.
            return ProcessStatus::NoOutputsModified;
        }

        // Hint to the compiler to optimize loop.
        assert!(frames <= gain.values.len());

        // Provide an optimized loop for stereo.
        if inputs.len() == 2 && outputs.len() == 2 {
            // Hint to the compiler to optimize loop.
            assert!(frames <= outputs[0].len());
            assert!(frames <= outputs[1].len());
            assert!(frames <= inputs[0].len());
            assert!(frames <= inputs[1].len());

            for i in 0..frames {
                outputs[0][i] = inputs[0][i] * gain[i];
                outputs[1][i] = inputs[1][i] * gain[i];
            }

            return ProcessStatus::outputs_modified(proc_info.in_silence_mask);
        }

        for (i, (output, input)) in outputs.iter_mut().zip(inputs.iter()).enumerate() {
            if proc_info.in_silence_mask.is_channel_silent(i) {
                if !proc_info.out_silence_mask.is_channel_silent(i) {
                    output[..frames].fill(0.0);
                }
                continue;
            }

            // Hint to the compiler to optimize loop.
            assert!(frames <= input.len());

            for i in 0..frames {
                output[i] = input[i] * gain[i];
            }
        }

        ProcessStatus::outputs_modified(proc_info.in_silence_mask)
    }
}

impl<C> Into<Box<dyn AudioNode<C>>> for VolumeNode {
    fn into(self) -> Box<dyn AudioNode<C>> {
        Box::new(self)
    }
}
