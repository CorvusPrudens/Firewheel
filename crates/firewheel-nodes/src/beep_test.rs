use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    dsp::decibel::normalized_volume_to_raw_gain,
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
};

/// A simple node that outputs a sine wave, used for testing purposes.
///
/// Note that because this node is for testing purposes, it does not
/// bother with parameter smoothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeepTestParams {
    /// The frequency of the sine wave in the range `[20.0, 20_000.0]`. A good
    /// value for testing is `440` (middle C).
    pub freq_hz: f32,

    /// The normalized volume where `.0` is mute and `1.0` is unity gain.
    /// NOTE, a sine wave at `1.0`` volume is *LOUD*, prefer to use a value
    /// like `0.5``.
    pub normalized_volume: f32,

    /// Whether or not the node is currently enabled.
    pub enabled: bool,
}

impl BeepTestParams {
    /// The ID of the volume parameter.
    pub const ID_VOLUME: u32 = 0;
    /// The ID of the frequency parameter.
    pub const ID_FREQUENCY: u32 = 1;
    /// The ID of the enabled parameter.
    pub const ID_ENABLED: u32 = 2;

    /// Create a beep test node constructor using these parameters.
    pub fn constructor(&self) -> Constructor {
        Constructor { params: *self }
    }

    /// Return an event type to sync the volume parameter.
    pub fn sync_volume_event(&self) -> NodeEventType {
        NodeEventType::F32Param {
            id: Self::ID_VOLUME,
            value: self.normalized_volume,
        }
    }

    /// Return an event type to sync the frequency parameter.
    pub fn sync_freq_hz_event(&self) -> NodeEventType {
        NodeEventType::F32Param {
            id: Self::ID_FREQUENCY,
            value: self.freq_hz,
        }
    }

    /// Return an event type to sync the enabled parameter.
    pub fn sync_enabled_event(&mut self) -> NodeEventType {
        NodeEventType::BoolParam {
            id: Self::ID_ENABLED,
            value: self.enabled,
        }
    }
}

impl Default for BeepTestParams {
    fn default() -> Self {
        Self {
            freq_hz: 440.0,
            normalized_volume: 0.5,
            enabled: true,
        }
    }
}

pub struct Constructor {
    pub params: BeepTestParams,
}

impl AudioNodeConstructor for Constructor {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "beep_test",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: ChannelCount::MONO,
            },
            uses_events: true,
        }
    }

    fn processor(
        &mut self,
        stream_info: &firewheel_core::StreamInfo,
    ) -> Box<dyn AudioNodeProcessor> {
        Box::new(Processor {
            phasor: 0.0,
            phasor_inc: self.params.freq_hz.clamp(20.0, 20_000.0)
                * stream_info.sample_rate_recip as f32,
            gain: normalized_volume_to_raw_gain(self.params.normalized_volume),
            sample_rate_recip: (stream_info.sample_rate.get() as f32).recip(),
            enabled: self.params.enabled,
        })
    }
}

struct Processor {
    phasor: f32,
    phasor_inc: f32,
    gain: f32,
    sample_rate_recip: f32,
    enabled: bool,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        _proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        let Some(out) = outputs.first_mut() else {
            return ProcessStatus::ClearAllOutputs;
        };

        events.for_each(|event| match event {
            NodeEventType::BoolParam { id, value } => {
                if *id == BeepTestParams::ID_ENABLED {
                    self.enabled = *value;
                }
            }
            NodeEventType::F32Param { id, value } => match *id {
                BeepTestParams::ID_VOLUME => self.gain = normalized_volume_to_raw_gain(*value),
                BeepTestParams::ID_FREQUENCY => {
                    self.phasor_inc = value.clamp(20.0, 20_000.0) * self.sample_rate_recip
                }
                _ => {}
            },
            _ => {}
        });

        if !self.enabled {
            return ProcessStatus::ClearAllOutputs;
        }

        for s in out.iter_mut() {
            *s = (self.phasor * std::f32::consts::TAU).sin() * self.gain;
            self.phasor = (self.phasor + self.phasor_inc).fract();
        }

        ProcessStatus::outputs_not_silent()
    }
}
