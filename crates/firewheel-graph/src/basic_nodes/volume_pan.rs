use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    dsp::{
        decibel::normalized_volume_to_raw_gain,
        pan_law::PanLaw,
        smoothing_filter::{self, DEFAULT_SETTLE_EPSILON, DEFAULT_SMOOTH_SECONDS},
    },
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
};

// TODO: Option for true stereo panning.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumePanParams {
    /// The percent volume where `0.0` is mute and `1.0` is unity gain.
    pub normalized_volume: f32,
    /// The pan amount, where `0.0` is center, `-1.0` is fully left, and `1.0` is
    /// fully right.
    pub pan: f32,
    /// The algorithm to use to map a normalized panning value in the range `[-1.0, 1.0]`
    /// to the corresponding gain values for the left and right channels.
    ///
    /// Use `NodeEventType::U32Param` for this parameter.
    pub pan_law: PanLaw,
}

impl VolumePanParams {
    /// The ID of the volume parameter.
    pub const ID_VOLUME: u32 = 0;
    /// The ID of the pan parameter.
    pub const ID_PAN: u32 = 1;
    /// The ID of the "pan law" parameter.
    pub const ID_PAN_LAW: u32 = 2;

    pub fn compute_gains(&self) -> (f32, f32) {
        let global_gain = normalized_volume_to_raw_gain(self.normalized_volume);

        let (gain_l, gain_r) = self.pan_law.compute_gains(self.pan);

        (gain_l * global_gain, gain_r * global_gain)
    }

    /// Return an event type to sync the volume parameter.
    pub fn sync_volume_event(&self) -> NodeEventType {
        NodeEventType::F32Param {
            id: Self::ID_VOLUME,
            value: self.normalized_volume,
        }
    }

    /// Return an event type to sync the pan parameter.
    pub fn sync_pan_event(&self) -> NodeEventType {
        NodeEventType::F32Param {
            id: Self::ID_PAN,
            value: self.pan,
        }
    }

    /// Return an event type to sync the pan law parameter.
    pub fn sync_pan_law_event(&self) -> NodeEventType {
        NodeEventType::U32Param {
            id: Self::ID_PAN_LAW,
            value: self.pan_law as u32,
        }
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

impl AudioNodeConstructor for VolumePanParams {
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

    fn processor(&self, stream_info: &firewheel_core::StreamInfo) -> Box<dyn AudioNodeProcessor> {
        let (gain_l, gain_r) = self.compute_gains();

        Box::new(VolumePanProcessor {
            smooth_filter_coeff: smoothing_filter::Coeff::new(
                stream_info.sample_rate,
                DEFAULT_SMOOTH_SECONDS,
            ),
            gain_l,
            gain_r,
            l_filter_target: gain_l,
            r_filter_target: gain_r,
            params: *self,
            prev_block_was_silent: true,
        })
    }
}

struct VolumePanProcessor {
    smooth_filter_coeff: smoothing_filter::Coeff,
    l_filter_target: f32,
    r_filter_target: f32,

    gain_l: f32,
    gain_r: f32,

    params: VolumePanParams,

    prev_block_was_silent: bool,
}

impl AudioNodeProcessor for VolumePanProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        let mut params_changed = false;

        events.for_each(|event| match event {
            NodeEventType::F32Param { id, value } => match *id {
                VolumePanParams::ID_VOLUME => {
                    self.params.normalized_volume = value.max(0.0);
                    params_changed = true;
                }
                VolumePanParams::ID_PAN => {
                    self.params.pan = value.clamp(-1.0, 1.0);
                    params_changed = true;
                }
                _ => {}
            },
            NodeEventType::U32Param { id, value } => {
                if *id == VolumePanParams::ID_PAN_LAW {
                    self.params.pan_law = PanLaw::from_u32(*value);
                    params_changed = true;
                }
            }
            _ => {}
        });

        if params_changed {
            let (gain_l, gain_r) = self.params.compute_gains();
            self.l_filter_target = gain_l;
            self.r_filter_target = gain_r;

            if self.prev_block_was_silent {
                // Previous block was silent, so no need to smooth.
                self.gain_l = self.l_filter_target;
                self.gain_r = self.r_filter_target;
            }
        }

        self.prev_block_was_silent = false;

        if proc_info.in_silence_mask.all_channels_silent(2) {
            self.gain_l = self.l_filter_target;
            self.gain_r = self.r_filter_target;
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        let in1 = &inputs[0][..proc_info.frames];
        let in2 = &inputs[1][..proc_info.frames];
        let (out1, out2) = outputs.split_first_mut().unwrap();
        let out1 = &mut out1[..proc_info.frames];
        let out2 = &mut out2[0][..proc_info.frames];

        if self.gain_l != self.l_filter_target || self.gain_r != self.r_filter_target {
            let l_target_times_a = self.l_filter_target * self.smooth_filter_coeff.a;
            let r_target_times_a = self.r_filter_target * self.smooth_filter_coeff.a;

            let mut l_state = self.gain_l;
            let mut r_state = self.gain_r;

            for i in 0..proc_info.frames {
                l_state = smoothing_filter::process_sample_a(
                    l_state,
                    l_target_times_a,
                    self.smooth_filter_coeff.b,
                );
                r_state = smoothing_filter::process_sample_a(
                    r_state,
                    r_target_times_a,
                    self.smooth_filter_coeff.b,
                );

                out1[i] = in1[i] * l_state;
                out2[i] = in2[i] * r_state;
            }

            self.gain_l = l_state;
            self.gain_r = r_state;

            if smoothing_filter::has_settled(
                self.gain_l,
                self.l_filter_target,
                DEFAULT_SETTLE_EPSILON,
            ) {
                self.gain_l = self.l_filter_target;
            }
            if smoothing_filter::has_settled(
                self.gain_r,
                self.r_filter_target,
                DEFAULT_SETTLE_EPSILON,
            ) {
                self.gain_r = self.r_filter_target;
            }
        } else if self.gain_l == 0.0 && self.gain_r == 0.0 {
            self.gain_l = self.l_filter_target;
            self.gain_r = self.r_filter_target;
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        } else {
            for i in 0..proc_info.frames {
                out1[i] = in1[i] * self.gain_l;
                out2[i] = in2[i] * self.gain_r;
            }
        }

        return ProcessStatus::outputs_modified(proc_info.in_silence_mask);
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        self.smooth_filter_coeff =
            smoothing_filter::Coeff::new(stream_info.sample_rate, DEFAULT_SMOOTH_SECONDS);
    }
}
