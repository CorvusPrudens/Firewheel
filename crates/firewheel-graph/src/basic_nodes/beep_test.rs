use firewheel_core::{
    dsp::decibel::normalized_volume_to_raw_gain,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, NodeEventIter, NodeEventType, ProcInfo,
        ProcessStatus,
    },
    ChannelConfig, ChannelCount, StreamInfo,
};

/// A simple node that outputs a sine wave, used for testing purposes.
///
/// Note that because this node is for testing purposes, it does not
/// bother with parameter smoothing.
pub struct BeepTestNode {
    freq_hz: f32,
    normalized_volume: f32,
    enabled: bool,
}

impl BeepTestNode {
    /// The ID of the volume parameter.
    pub const PARAM_VOLUME: u32 = 0;
    /// The ID of the frequency parameter.
    pub const PARAM_FREQUENCY: u32 = 1;

    /// Create a new [`BeepTestNode`].
    ///
    /// * `normalized_volume` - The normalized volume where `.0` is mute and `1.0` is unity gain.
    /// NOTE, a sine wave at `1.0`` volume is *LOUD*, prefer to use a value like `0.25``.
    /// * `freq_hz` - The frequency of the sine wave in the range `[20.0, 20_000.0]`. A good
    /// value for testing is `440` (middle C).
    /// * `enabled` - Whether or not to start outputting a sine wave when the node is added
    /// to the graph.
    pub fn new(normalized_volume: f32, freq_hz: f32, enabled: bool) -> Self {
        Self {
            freq_hz: freq_hz.clamp(20.0, 20_000.0),
            normalized_volume: normalized_volume.max(0.0),
            enabled,
        }
    }

    /// Get the current normalized volume where `0.0` is mute and `1.0` is unity gain.
    pub fn normalized_volume(&self) -> f32 {
        self.normalized_volume
    }

    /// Return an event type to set the volume parameter.
    ///
    /// * `normalized_volume` - The normalized volume where `0.0` is mute and `1.0` is unity gain.
    ///
    /// NOTE, a sine wave at `1.0` volume is *LOUD*, prefer to use a value like`0.25`.
    pub fn set_volume(&mut self, normalized_volume: f32) -> NodeEventType {
        self.normalized_volume = normalized_volume;
        NodeEventType::F32Param {
            id: Self::PARAM_VOLUME,
            value: normalized_volume,
            smoothing: false,
        }
    }

    /// Get the frequency of the sine wave in the range `[20.0, 20_000.0]`
    pub fn freq_hz(&self) -> f32 {
        self.freq_hz
    }

    /// Return an event type to set the frequency parameter.
    ///
    /// * `freq_hz` - The frequency of the sine wave in the range `[20.0, 20_000.0]`.
    ///
    /// A good value for testing is `440` (middle C).
    pub fn set_freq_hz(&mut self, freq_hz: f32) -> NodeEventType {
        self.freq_hz = freq_hz;
        NodeEventType::F32Param {
            id: Self::PARAM_FREQUENCY,
            value: freq_hz,
            smoothing: false,
        }
    }

    /// Get whether or not this node is currently enabled.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Return an event type to enable/disable the node.
    pub fn set_enabled(&mut self, enabled: bool) -> NodeEventType {
        self.enabled = enabled;
        NodeEventType::SetEnabled(enabled)
    }
}

impl AudioNode for BeepTestNode {
    fn debug_name(&self) -> &'static str {
        "beep_test"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::MAX,
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: ChannelCount::STEREO,
            },
            ..Default::default()
        }
    }

    fn activate(
        &mut self,
        stream_info: &StreamInfo,
        _channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn std::error::Error>> {
        Ok(Box::new(BeepTestProcessor {
            phasor: 0.0,
            phasor_inc: self.freq_hz / stream_info.sample_rate as f32,
            gain: normalized_volume_to_raw_gain(self.normalized_volume),
            sample_rate_recip: (stream_info.sample_rate as f32).recip(),
            enabled: self.enabled,
        }))
    }
}

struct BeepTestProcessor {
    phasor: f32,
    phasor_inc: f32,
    gain: f32,
    sample_rate_recip: f32,
    enabled: bool,
}

impl AudioNodeProcessor for BeepTestProcessor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventIter,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let Some((out1, outputs)) = outputs.split_first_mut() else {
            return ProcessStatus::ClearAllOutputs;
        };

        for event in events {
            match event {
                NodeEventType::SetEnabled(enabled) => {
                    self.enabled = *enabled;
                }
                NodeEventType::F32Param { id, value, .. } => match *id {
                    BeepTestNode::PARAM_VOLUME => self.gain = normalized_volume_to_raw_gain(*value),
                    BeepTestNode::PARAM_FREQUENCY => {
                        self.phasor_inc = value.clamp(20.0, 20_000.0) * self.sample_rate_recip
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if !self.enabled {
            return ProcessStatus::ClearAllOutputs;
        }

        for s in out1[..proc_info.samples].iter_mut() {
            *s = (self.phasor * std::f32::consts::TAU).sin() * self.gain;
            self.phasor = (self.phasor + self.phasor_inc).fract();
        }

        for out2 in outputs.iter_mut() {
            out2[..proc_info.samples].copy_from_slice(&out1[..proc_info.samples]);
        }

        ProcessStatus::outputs_not_silent()
    }
}

impl Into<Box<dyn AudioNode>> for BeepTestNode {
    fn into(self) -> Box<dyn AudioNode> {
        Box::new(self)
    }
}
