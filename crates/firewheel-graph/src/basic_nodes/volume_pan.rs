use firewheel_core::{
    dsp::{
        decibel::normalized_volume_to_raw_gain,
        pan_law::PanLaw,
        smoothing_filter::{self, DEFAULT_SETTLE_EPSILON, DEFAULT_SMOOTH_SECONDS},
    },
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, NodeEventType, ProcInfo,
        ProcessStatus,
    },
    ChannelConfig, ChannelCount, StreamInfo,
};

// TODO: Option for true stereo panning.

pub struct VolumePanNode {
    normalized_volume: f32,
    pan: f32,
    pan_law: PanLaw,
}

impl VolumePanNode {
    /// The ID of the volume parameter.
    pub const PARAM_VOLUME: u32 = 0;
    /// The ID of the pan parameter.
    pub const PARAM_PAN: u32 = 1;

    /// Create a new volume node.
    ///
    /// * `normalized_volume` - The percent volume where `0.0` is mute and `1.0` is unity gain.
    /// * `pan` - The pan amount, where `0.0` is center, `-1.0` is fully left, and `1.0` is
    /// fully right.
    pub fn new(normalized_volume: f32, pan: f32, pan_law: PanLaw) -> Self {
        let normalized_volume = normalized_volume.max(0.0);
        let pan = pan.clamp(-1.0, 1.0);

        Self {
            normalized_volume,
            pan,
            pan_law,
        }
    }

    /// Get the current percent volume where `0.0` is mute and `1.0` is unity gain.
    pub fn normalized_volume(&self) -> f32 {
        self.normalized_volume
    }

    /// Return an event type to set the volume parameter.
    ///
    /// * `normalized_volume` - The percent volume where `0.0` is mute and `1.0` is unity gain.
    /// * `smoothing` - Set this to `false` to have the node immediately jump to this new
    /// value without smoothing (may cause audible clicking or stair-stepping artifacts). This
    /// can be useful to preserve transients when playing a new sound at a different volume.
    pub fn set_volume(&mut self, normalized_volume: f32, smoothing: bool) -> NodeEventType {
        self.normalized_volume = normalized_volume.max(0.0);
        NodeEventType::F32Param {
            id: Self::PARAM_VOLUME,
            value: normalized_volume,
            smoothing,
        }
    }

    /// Return an event type to set the pan parameter.
    ///
    /// * `pan` - The pan amount, where `0.0` is center, `-1.0` is fully left, and `1.0` is
    /// fully right.
    /// * `smoothing` - Set this to `false` to have the node immediately jump to this new
    /// value without smoothing (may cause audible clicking or stair-stepping artifacts). This
    /// can be useful to preserve transients when playing a new sound at a different volume.
    pub fn set_pan(&mut self, pan: f32, smoothing: bool) -> NodeEventType {
        self.pan = pan.clamp(-1.0, 1.0);
        NodeEventType::F32Param {
            id: Self::PARAM_PAN,
            value: self.pan,
            smoothing,
        }
    }
}

impl AudioNode for VolumePanNode {
    fn debug_name(&self) -> &'static str {
        "volume_pan"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::STEREO,
            num_max_supported_inputs: ChannelCount::STEREO,
            num_min_supported_outputs: ChannelCount::STEREO,
            num_max_supported_outputs: ChannelCount::STEREO,
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
        stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        let (gain_l, gain_r) = compute_gains(self.normalized_volume, self.pan, self.pan_law);

        Ok(Box::new(VolumePanProcessor {
            smooth_filter_coeff: smoothing_filter::Coeff::new(
                stream_info.sample_rate,
                DEFAULT_SMOOTH_SECONDS,
            ),
            gain_l,
            gain_r,
            l_filter_target: gain_l,
            r_filter_target: gain_r,
            normalized_volume: self.normalized_volume,
            pan: self.pan,
            pan_law: self.pan_law,
        }))
    }
}

struct VolumePanProcessor {
    smooth_filter_coeff: smoothing_filter::Coeff,
    l_filter_target: f32,
    r_filter_target: f32,

    gain_l: f32,
    gain_r: f32,

    normalized_volume: f32,
    pan: f32,
    pan_law: PanLaw,
}

impl AudioNodeProcessor for VolumePanProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let mut params_changed = false;
        let mut do_smooth = false;

        for msg in events {
            if let NodeEventType::F32Param {
                id,
                value,
                smoothing,
            } = msg
            {
                match *id {
                    VolumePanNode::PARAM_VOLUME => {
                        self.normalized_volume = value.max(0.0);
                        params_changed = true;

                        do_smooth = *smoothing;
                    }
                    VolumePanNode::PARAM_PAN => {
                        self.pan = value.clamp(-1.0, 1.0);
                        params_changed = true;

                        do_smooth = *smoothing;
                    }
                    _ => {}
                }
            }
        }

        if params_changed {
            let (gain_l, gain_r) = compute_gains(self.normalized_volume, self.pan, self.pan_law);
            self.l_filter_target = gain_l;
            self.r_filter_target = gain_r;

            if !do_smooth {
                self.gain_l = self.l_filter_target;
                self.gain_r = self.r_filter_target;
            }
        }

        if proc_info.in_silence_mask.all_channels_silent(2) {
            self.gain_l = self.l_filter_target;
            self.gain_r = self.r_filter_target;

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

            return ProcessStatus::ClearAllOutputs;
        } else {
            for i in 0..proc_info.frames {
                out1[i] = in1[i] * self.gain_l;
                out2[i] = in2[i] * self.gain_r;
            }
        }

        return ProcessStatus::outputs_modified(proc_info.in_silence_mask);
    }
}

impl Into<Box<dyn AudioNode>> for VolumePanNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}

fn compute_gains(normalized_volume: f32, pan: f32, pan_law: PanLaw) -> (f32, f32) {
    let global_gain = normalized_volume_to_raw_gain(normalized_volume);

    let (gain_l, gain_r) = pan_law.compute_gains(pan);

    (gain_l * global_gain, gain_r * global_gain)
}
