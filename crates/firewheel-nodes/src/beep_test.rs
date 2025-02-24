use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::decibel::normalized_volume_to_raw_gain,
    event::NodeEventList,
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        ScratchBuffers,
    },
};

/// A simple node that outputs a sine wave, used for testing purposes.
///
/// Note that because this node is for testing purposes, it does not
/// bother with parameter smoothing.
#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
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
    /// Create a beep test node constructor using these parameters.
    pub fn constructor(&self) -> Constructor {
        Constructor { params: *self }
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
            params: self.params,
        })
    }
}

struct Processor {
    phasor: f32,
    phasor_inc: f32,
    gain: f32,
    sample_rate_recip: f32,
    params: BeepTestParams,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: NodeEventList,
        _proc_info: &ProcInfo,
        _scratch_buffers: ScratchBuffers,
    ) -> ProcessStatus {
        let Some(out) = outputs.first_mut() else {
            return ProcessStatus::ClearAllOutputs;
        };

        if self.params.patch_list(events) {
            self.phasor_inc = self.params.freq_hz.clamp(20.0, 20_000.0) * self.sample_rate_recip;
            self.gain = normalized_volume_to_raw_gain(self.params.normalized_volume);
        }

        if !self.params.enabled {
            return ProcessStatus::ClearAllOutputs;
        }

        for s in out.iter_mut() {
            *s = (self.phasor * std::f32::consts::TAU).sin() * self.gain;
            self.phasor = (self.phasor + self.phasor_inc).fract();
        }

        ProcessStatus::outputs_not_silent()
    }
}
