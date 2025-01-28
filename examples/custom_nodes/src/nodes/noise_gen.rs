//! A simple node that generates white noise.

use firewheel::{
    channel_config::{ChannelConfig, ChannelCount},
    dsp::decibel::normalized_volume_to_raw_gain,
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    SilenceMask, StreamInfo,
};

// The parameter struct holds all of the parameters of the node as plain values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseGenParams {
    /// The normalized volume where `0.0` is mute and `1.0` is unity gain.
    ///
    /// White noise is really loud, so this be set to something like `0.4`.
    pub normalized_volume: f32,
    /// Whether or not this node is enabled.
    pub enabled: bool,
}

impl Default for NoiseGenParams {
    fn default() -> Self {
        Self {
            normalized_volume: 0.4,
            enabled: true,
        }
    }
}

impl NoiseGenParams {
    // Store the IDs of your parameters as constants.
    pub const ID_VOLUME: u32 = 0;
    pub const ID_ENABLED: u32 = 1;

    // Add a method to create a new node constructor using these parameters.
    //
    // You may also pass any additional configuration for the node here. Here
    // we pass a "seed" argument for the random number generator.
    pub fn constructor(&self, seed: Option<u32>) -> Constructor {
        Constructor {
            params: *self,
            seed: seed.unwrap_or(17),
        }
    }

    // A helper method to generate an event type to sync the new value of the
    // volume parameter.
    pub fn sync_volume_event(&self) -> NodeEventType {
        NodeEventType::F32Param {
            id: Self::ID_VOLUME,
            value: self.normalized_volume,
        }
    }

    // A helper method to generate an event type to sync the new value of the
    // enabled parameter.
    pub fn sync_enabled_event(&self) -> NodeEventType {
        NodeEventType::BoolParam {
            id: Self::ID_ENABLED,
            value: self.enabled,
        }
    }
}

// This struct holds information to construct the node in the audio graph.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Constructor {
    pub params: NoiseGenParams,
    pub seed: u32,
}

// Derive the AudioNodeConstructor type for your constructor.
impl AudioNodeConstructor for Constructor {
    // Return information about your node. This method is only ever called
    // once.
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            // A static name used for debugging purposes.
            debug_name: "example_nosie_gen",
            // The configuration of the input/output ports.
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: ChannelCount::MONO,
            },
            // Wether or not our node uses events. If it does not, then setting
            // this to `false` will save a bit of memory by not allocating an
            // event buffer for this node.
            uses_events: true,
        }
    }

    // Construct the realtime processor counterpart using the given information
    // about the audio stream.
    //
    // This method is called before the node processor is sent to the realtime
    // thread, so it is safe to do non-realtime things here like allocating.
    fn processor(&mut self, _stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        // Seed cannot be zero.
        let seed = if self.seed == 0 { 17 } else { self.seed };

        Box::new(Processor {
            fpd: seed,
            gain: normalized_volume_to_raw_gain(self.params.normalized_volume),
            enabled: self.params.enabled,
        })
    }
}

// The realtime processor counterpart to your node.
struct Processor {
    fpd: u32,
    gain: f32,
    enabled: bool,
}

impl AudioNodeProcessor for Processor {
    // The realtime process method.
    fn process(
        &mut self,
        // The list of input buffers. This will always be equal to the number we
        // gave in `info()`.`
        _inputs: &[&[f32]],
        // The list of output buffers. This will always be equal to the number we
        // gave in `info()`.`
        outputs: &mut [&mut [f32]],
        // The list of events for our node to process.
        mut events: NodeEventList,
        // Additional information about the process.
        _proc_info: &ProcInfo,
        // Optional scratch buffers that can be used for processing.
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        // Process the events.
        events.for_each(|event| {
            match event {
                NodeEventType::F32Param { id, value } => {
                    if *id == NoiseGenParams::ID_VOLUME {
                        // Note, while parameter smoothing doesn't really matter for white
                        // noise, you will generally want to smooth parameters. See the
                        // custom filter node for examples of how to do that.
                        self.gain = normalized_volume_to_raw_gain(*value);
                    }
                }
                NodeEventType::BoolParam { id, value } => {
                    if *id == NoiseGenParams::ID_ENABLED {
                        // Note, while declicking doesn't matter for white noise, you may
                        // want to declick the output when turning on/off a generator
                        // node. See the custom filter node for an example of how to do
                        // that.
                        self.enabled = *value;
                    }
                }
                _ => {}
            }
        });

        if !self.enabled {
            // Tell the engine to automatically and efficiently clear the output buffers
            // for us. This is equivalent to doing:
            // ```
            // for (i, out_ch) in outputs.iter_mut().enumerate() {
            //    if !proc_info.out_silence_mask.is_channel_silent(i) {
            //        out_ch.fill(0.0);
            //    } // otherwise buffer is already silent
            // }
            //
            // return ProcessStatus::OutputsModified { out_silence_mask: SilenceMask::new_all_silent(outputs.len()) }
            // ```
            return ProcessStatus::ClearAllOutputs;
        }

        for s in outputs[0].iter_mut() {
            // Tick the random number generator.
            self.fpd ^= self.fpd << 13;
            self.fpd ^= self.fpd >> 17;
            self.fpd ^= self.fpd << 5;

            // Get a random normalized value in the range `[-1.0, 1.0]`.
            let r = self.fpd as f32 * (2.0 / 4_294_967_295.0) - 1.0;

            *s = r * self.gain;
        }

        // Notify the engine that we have modified the output buffers.
        ProcessStatus::OutputsModified {
            out_silence_mask: SilenceMask::NONE_SILENT,
        }
    }
}
