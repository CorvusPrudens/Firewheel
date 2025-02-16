use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::{decibel::normalized_volume_to_raw_gain, pan_law::PanLaw},
    event::NodeEventList,
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    param::smoother::{SmoothedParam, SmootherConfig},
    SilenceMask,
};

pub use super::volume::VolumeNodeConfig;

// TODO: Option for true stereo panning.

#[derive(Diff, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct VolumePanParams {
    /// The normalized volume where `0.0` is mute and `1.0` is unity gain.
    pub normalized_volume: f32,
    /// The pan amount, where `0.0` is center, `-1.0` is fully left, and `1.0` is
    /// fully right.
    pub pan: f32,
    /// The algorithm to use to map a normalized panning value in the range `[-1.0, 1.0]`
    /// to the corresponding gain values for the left and right channels.
    pub pan_law: PanLaw,
}

impl Patch for VolumePanParams {
    fn patch(
        &mut self,
        data: &firewheel_core::event::ParamData,
        path: &[u32],
    ) -> Result<(), firewheel_core::diff::PatchError> {
        match path.first() {
            Some(0) => {
                let volume: f32 = data.try_into()?;
                self.normalized_volume = volume.max(0.0);

                if self.normalized_volume < 0.00001 {
                    self.normalized_volume = 0.0;
                }

                Ok(())
            }
            Some(1) => {
                let pan: f32 = data.try_into()?;
                self.pan = pan.clamp(-1.0, 1.0);
                Ok(())
            }
            Some(2) => self.pan_law.patch(data, &path[1..]),
            _ => Err(firewheel_core::diff::PatchError::InvalidPath),
        }
    }
}

impl VolumePanParams {
    /// Create a volume pan node constructor using these parameters.
    pub fn constructor(&self, config: VolumeNodeConfig) -> Constructor {
        Constructor {
            params: *self,
            config,
        }
    }

    pub fn compute_gains(&self) -> (f32, f32) {
        let global_gain = normalized_volume_to_raw_gain(self.normalized_volume);

        let (gain_l, gain_r) = self.pan_law.compute_gains(self.pan);

        (gain_l * global_gain, gain_r * global_gain)
    }
}

impl Default for VolumePanParams {
    fn default() -> Self {
        Self {
            normalized_volume: 1.0,
            pan: 0.0,
            pan_law: PanLaw::default(),
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Constructor {
    pub params: VolumePanParams,
    pub config: VolumeNodeConfig,
}

impl AudioNodeConstructor for Constructor {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "volume_pan",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::STEREO,
            },
            uses_events: true,
        }
    }

    fn processor(
        &mut self,
        stream_info: &firewheel_core::StreamInfo,
    ) -> Box<dyn AudioNodeProcessor> {
        let (gain_l, gain_r) = self.params.compute_gains();

        Box::new(Processor {
            gain_l: SmoothedParam::new(
                gain_l,
                SmootherConfig {
                    smooth_secs: self.config.smooth_secs,
                    ..Default::default()
                },
                stream_info.sample_rate,
            ),
            gain_r: SmoothedParam::new(
                gain_r,
                SmootherConfig {
                    smooth_secs: self.config.smooth_secs,
                    ..Default::default()
                },
                stream_info.sample_rate,
            ),
            params: self.params,
            prev_block_was_silent: true,
        })
    }
}

struct Processor {
    gain_l: SmoothedParam,
    gain_r: SmoothedParam,

    params: VolumePanParams,

    prev_block_was_silent: bool,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        let mut params_changed = false;

        events.for_each(|event| {
            self.params.patch_params(event);
            params_changed = true;
        });

        if params_changed {
            let (gain_l, gain_r) = self.params.compute_gains();
            self.gain_l.set_value(gain_l);
            self.gain_r.set_value(gain_r);

            if self.prev_block_was_silent {
                // Previous block was silent, so no need to smooth.
                self.gain_l.reset();
                self.gain_r.reset();
            }
        }

        self.prev_block_was_silent = false;

        if proc_info.in_silence_mask.all_channels_silent(2) {
            self.gain_l.reset();
            self.gain_r.reset();
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        let in1 = &inputs[0][..proc_info.frames];
        let in2 = &inputs[1][..proc_info.frames];
        let (out1, out2) = outputs.split_first_mut().unwrap();
        let out1 = &mut out1[..proc_info.frames];
        let out2 = &mut out2[0][..proc_info.frames];

        if !self.gain_l.is_smoothing() && !self.gain_r.is_smoothing() {
            if self.gain_l.target_value() == 0.0 && self.gain_r.target_value() == 0.0 {
                self.gain_l.reset();
                self.gain_r.reset();
                self.prev_block_was_silent = true;

                ProcessStatus::ClearAllOutputs
            } else {
                for i in 0..proc_info.frames {
                    out1[i] = in1[i] * self.gain_l.target_value();
                    out2[i] = in2[i] * self.gain_r.target_value();
                }

                ProcessStatus::outputs_modified(proc_info.in_silence_mask)
            }
        } else {
            for i in 0..proc_info.frames {
                let gain_l = self.gain_l.next_smoothed();
                let gain_r = self.gain_r.next_smoothed();

                out1[i] = in1[i] * gain_l;
                out2[i] = in2[i] * gain_r;
            }

            ProcessStatus::outputs_modified(SilenceMask::NONE_SILENT)
        }
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        self.gain_l.update_sample_rate(stream_info.sample_rate);
        self.gain_r.update_sample_rate(stream_info.sample_rate);
    }
}
