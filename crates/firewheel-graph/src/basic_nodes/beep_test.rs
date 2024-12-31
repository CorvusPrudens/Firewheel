use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    clock::EventDelay,
    dsp::decibel::normalized_volume_to_raw_gain,
    node::{
        AudioNodeProcessor, NodeEventIter, NodeEventType, NodeHandle, NodeID, ProcInfo,
        ProcessStatus,
    },
};

use crate::FirewheelCtx;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
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

impl Default for Params {
    fn default() -> Self {
        Self {
            freq_hz: 440.0,
            normalized_volume: 0.5,
            enabled: true,
        }
    }
}

/// A simple node that outputs a sine wave, used for testing purposes.
///
/// Note that because this node is for testing purposes, it does not
/// bother with parameter smoothing.
pub struct BeepTestNode {
    params: Params,
    handle: NodeHandle,
}

impl BeepTestNode {
    /// The ID of the volume parameter.
    pub const PARAM_VOLUME: u32 = 0;
    /// The ID of the frequency parameter.
    pub const PARAM_FREQUENCY: u32 = 1;

    /// Create a new [`BeepTestNode`].
    pub fn new(params: Params, cx: &mut FirewheelCtx) -> Self {
        let sample_rate = cx.stream_info().sample_rate;
        let sample_rate_recip = cx.stream_info().sample_rate_recip;

        let handle = cx.add_node(
            "beep_test",
            ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: ChannelCount::MONO,
            },
            true,
            Box::new(BeepTestProcessor {
                phasor: 0.0,
                phasor_inc: params.freq_hz.clamp(20.0, 20_000.0) * sample_rate_recip as f32,
                gain: normalized_volume_to_raw_gain(params.normalized_volume),
                sample_rate_recip: (sample_rate.get() as f32).recip(),
                enabled: params.enabled,
            }),
        );

        Self { params, handle }
    }

    /// The ID of this node
    pub fn id(&self) -> NodeID {
        self.handle.id
    }

    /// Get the current parameters.
    pub fn params(&self) -> &Params {
        &self.params
    }

    /// Set the volume parameter.
    ///
    /// * `normalized_volume` - The normalized volume where `0.0` is mute and `1.0` is unity gain.
    ///
    /// NOTE, a sine wave at `1.0` volume is *LOUD*, prefer to use a value like`0.5`.
    pub fn set_volume(&mut self, normalized_volume: f32, delay: EventDelay) {
        self.params.normalized_volume = normalized_volume;
        self.handle.queue_event(
            NodeEventType::F32Param {
                id: Self::PARAM_VOLUME,
                value: normalized_volume,
                smoothing: false,
            },
            delay,
        );
    }

    /// Set the frequency parameter.
    ///
    /// * `freq_hz` - The frequency of the sine wave in the range `[20.0, 20_000.0]`.
    ///
    /// A good value for testing is `440` (middle C).
    pub fn set_freq_hz(&mut self, freq_hz: f32, delay: EventDelay) {
        self.params.freq_hz = freq_hz;
        self.handle.queue_event(
            NodeEventType::F32Param {
                id: Self::PARAM_VOLUME,
                value: freq_hz,
                smoothing: false,
            },
            delay,
        );
    }

    /// Enable/disable this node.
    pub fn set_enabled(&mut self, enabled: bool, delay: EventDelay) {
        self.params.enabled = enabled;
        self.handle
            .queue_event(NodeEventType::SetEnabled(enabled), delay);
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

        for s in out1[..proc_info.frames].iter_mut() {
            *s = (self.phasor * std::f32::consts::TAU).sin() * self.gain;
            self.phasor = (self.phasor + self.phasor_inc).fract();
        }

        for out2 in outputs.iter_mut() {
            out2[..proc_info.frames].copy_from_slice(&out1[..proc_info.frames]);
        }

        ProcessStatus::outputs_not_silent()
    }
}
