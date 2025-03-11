//! A simple node that demonstrates having a handle with shared state.
//!
//! This node calculates the RMS (root-mean-square) of a mono signal.

use std::sync::atomic::Ordering;

use atomic_float::AtomicF32;
use firewheel::{
    channel_config::{ChannelConfig, ChannelCount},
    collector::ArcGc,
    diff::{Diff, Patch},
    event::NodeEventList,
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcBuffers, ProcInfo, ProcessStatus},
    StreamInfo,
};

#[derive(Debug)]
struct SharedState {
    rms_value: AtomicF32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RmsConfig {
    pub window_size_secs: f32,
}

impl Default for RmsConfig {
    fn default() -> Self {
        Self {
            window_size_secs: 5.0 / 1_000.0,
        }
    }
}

// The parameter struct holds all of the parameters of the node.
#[derive(Debug, Diff, Patch, Clone)]
pub struct RmsNode {
    /// Whether or not this node is enabled.
    pub enabled: bool,
    // `ArcGc` is a simple wrapper around `Arc` that automatically collects
    // dropped resources from the audio thread and drops them on another
    // thread.
    //
    // Because this type is synchronized with atomics, it
    // doesn't need any diffing or patching.
    #[diff(skip)]
    shared_state: ArcGc<SharedState>,
}

impl Default for RmsNode {
    fn default() -> Self {
        Self::new()
    }
}

impl RmsNode {
    pub fn new() -> Self {
        Self {
            enabled: true,
            shared_state: ArcGc::new(SharedState {
                rms_value: AtomicF32::new(0.0),
            }),
        }
    }

    pub fn rms_value(&self) -> f32 {
        self.shared_state.rms_value.load(Ordering::Relaxed)
    }
}

// Implement the AudioNode type for your node.
impl AudioNode for RmsNode {
    type Configuration = RmsConfig;

    // Return information about your node. This method is only ever called
    // once.
    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
        // The builder pattern is used for future-proofness as it is likely that
        // more fields will be added in the future.
        AudioNodeInfo::new()
            // A static name used for debugging purposes.
            .debug_name("example_rms")
            // The configuration of the input/output ports.
            .channel_config(ChannelConfig {
                num_inputs: ChannelCount::MONO,
                num_outputs: ChannelCount::ZERO,
            })
            // Wether or not our node uses events. If it does not, then setting
            // this to `false` will save a bit of memory by not allocating an
            // event buffer for this node.
            .uses_events(true)
    }

    // Construct the realtime processor counterpart using the given information
    // about the audio stream.
    //
    // This method is called before the node processor is sent to the realtime
    // thread, so it is safe to do non-realtime things here like allocating.
    fn processor(
        &self,
        config: &Self::Configuration,
        stream_info: &StreamInfo,
    ) -> impl AudioNodeProcessor {
        let window_frames =
            (config.window_size_secs * stream_info.sample_rate.get() as f32).round() as usize;

        Processor {
            params: self.clone(),
            squares: 0.0,
            num_squared_values: 0,
            window_frames,
            config: *config,
        }
    }
}

// The realtime processor counterpart to your node.
struct Processor {
    params: RmsNode,
    squares: f32,
    num_squared_values: usize,
    window_frames: usize,
    config: RmsConfig,
}

impl AudioNodeProcessor for Processor {
    // The realtime process method.
    fn process(
        &mut self,
        // The buffers of data to process.
        buffers: ProcBuffers,
        // Additional information about the process.
        proc_info: &ProcInfo,
        // The list of events for our node to process.
        events: NodeEventList,
    ) -> ProcessStatus {
        self.params.patch_list(events);

        if !self.params.enabled {
            self.params
                .shared_state
                .rms_value
                .store(0.0, Ordering::Relaxed);

            self.squares = 0.0;
            self.num_squared_values = 0;

            return ProcessStatus::Bypass;
        }

        let mut frames_processed = 0;
        while frames_processed < proc_info.frames {
            let process_frames = (proc_info.frames - frames_processed)
                .min(self.window_frames - self.num_squared_values);

            for &s in buffers.inputs[0][frames_processed..frames_processed + process_frames].iter()
            {
                self.squares += s * s;
            }

            self.num_squared_values += process_frames;
            frames_processed += process_frames;

            if self.num_squared_values == self.window_frames {
                let mean = self.squares / self.window_frames as f32;
                let rms = mean.sqrt();

                self.params
                    .shared_state
                    .rms_value
                    .store(rms, Ordering::Relaxed);

                self.squares = 0.0;
                self.num_squared_values = 0;
            }
        }

        // There are no outputs in this node.
        ProcessStatus::Bypass
    }

    // Called when a new stream has been created. Because the new stream may have a
    // different sample rate from the old one, make sure to update any calculations
    // that depend on the sample rate.
    //
    // This gets called outside of the audio thread, so it is safe to allocate and
    // deallocate here.
    fn new_stream(&mut self, stream_info: &StreamInfo) {
        self.window_frames =
            (self.config.window_size_secs * stream_info.sample_rate.get() as f32).round() as usize;

        self.squares = 0.0;
        self.num_squared_values = 0;
    }
}
