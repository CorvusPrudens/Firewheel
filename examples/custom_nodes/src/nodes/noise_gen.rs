//! A simple node that generates white noise.

use firewheel::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::decibel::normalized_volume_to_raw_gain,
    event::NodeEventList,
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    SilenceMask, StreamInfo,
};

// The parameter struct holds all of the parameters of the node as plain values.
#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
pub struct NoiseGenParams {
    /// The normalized volume where `0.0` is mute and `1.0` is unity gain.
    ///
    /// White noise is really loud, so use something like `0.4`.
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

// The configuration allows users to provide
// one-time initialization settings for your
// processors.
//
// Here we provide a "seed" for the random number generator
#[derive(Debug, Clone, Copy)]
pub struct NoiseGenConfig {
    pub seed: u32,
}

impl Default for NoiseGenConfig {
    fn default() -> Self {
        Self { seed: 17 }
    }
}

// Implement the AudioNodeConstructor type for your node.
impl AudioNodeConstructor for NoiseGenParams {
    type Configuration = NoiseGenConfig;

    // Return information about your node. This method is only ever called
    // once.
    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
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
    fn processor(
        &self,
        config: &Self::Configuration,
        _stream_info: &StreamInfo,
    ) -> impl AudioNodeProcessor {
        // Seed cannot be zero.
        let seed = if config.seed == 0 { 17 } else { config.seed };

        Processor {
            fpd: seed,
            gain: normalized_volume_to_raw_gain(self.normalized_volume),
            params: *self,
        }
    }
}

// The realtime processor counterpart to your node.
struct Processor {
    fpd: u32,
    params: NoiseGenParams,
    gain: f32,
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
        events: NodeEventList,
        // Additional information about the process.
        _proc_info: &ProcInfo,
        // Optional scratch buffers that can be used for processing.
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        // Process the events.
        if self.params.patch_list(events) {
            self.gain = normalized_volume_to_raw_gain(self.params.normalized_volume);
        }

        if !self.params.enabled {
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
