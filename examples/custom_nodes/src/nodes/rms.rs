//! A simple node that demonstrates having a handle with shared state.
//!
//! This node calculates the RMS (root-mean-square) of a mono signal.

use std::sync::atomic::{AtomicBool, Ordering};

use atomic_float::AtomicF32;
use firewheel::{
    channel_config::{ChannelConfig, ChannelCount},
    collector::ArcGc,
    event::NodeEventList,
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        ScratchBuffers,
    },
    StreamInfo,
};

struct SharedState {
    rms_value: AtomicF32,
    enabled: AtomicBool,
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

// The parameter struct holds all of the parameters of the node as plain values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RmsParams {
    /// Whether or not this node is enabled.
    pub enabled: bool,
}

impl Default for RmsParams {
    fn default() -> Self {
        Self { enabled: true }
    }
}

// A handle to the internal shared state.
#[derive(Clone)]
pub struct RmsHandle {
    // `ArcGc` is a simple wrapper around `Arc` that automatically collects
    // dropped resources from the audio thread and drops them on another
    // thread.
    shared_state: ArcGc<SharedState>,
}

impl RmsHandle {
    pub fn new(params: RmsParams) -> Self {
        Self {
            shared_state: ArcGc::new(SharedState {
                rms_value: AtomicF32::new(0.0),
                enabled: AtomicBool::new(params.enabled),
            }),
        }
    }

    // Add a method to create a new node constructor using these parameters.
    //
    // You may also pass any additional configuration for the node here.
    pub fn constructor(&self, config: RmsConfig) -> Constructor {
        Constructor {
            shared_state: ArcGc::clone(&self.shared_state),
            config,
        }
    }

    pub fn sync_params(&mut self, params: RmsParams) {
        self.shared_state
            .enabled
            .store(params.enabled, Ordering::Relaxed);
    }

    pub fn rms_value(&self) -> f32 {
        self.shared_state.rms_value.load(Ordering::Relaxed)
    }
}

// This struct holds information to construct the node in the audio graph.
#[derive(Clone)]
pub struct Constructor {
    shared_state: ArcGc<SharedState>,
    config: RmsConfig,
}

// Derive the AudioNodeConstructor type for your constructor.
impl AudioNodeConstructor for Constructor {
    // Return information about your node. This method is only ever called
    // once.
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            // A static name used for debugging purposes.
            debug_name: "example_rms",
            // The configuration of the input/output ports.
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::MONO,
                num_outputs: ChannelCount::ZERO,
            },
            // Wether or not our node uses events. If it does not, then setting
            // this to `false` will save a bit of memory by not allocating an
            // event buffer for this node.
            uses_events: false,
        }
    }

    // Construct the realtime processor counterpart using the given information
    // about the audio stream.
    //
    // This method is called before the node processor is sent to the realtime
    // thread, so it is safe to do non-realtime things here like allocating.
    fn processor(&mut self, stream_info: &StreamInfo) -> Box<dyn AudioNodeProcessor> {
        let window_frames =
            (self.config.window_size_secs * stream_info.sample_rate.get() as f32).round() as usize;

        Box::new(Processor {
            shared_state: ArcGc::clone(&self.shared_state),
            squares: 0.0,
            num_squared_values: 0,
            window_frames,
            config: self.config,
        })
    }
}

// The realtime processor counterpart to your node.
struct Processor {
    shared_state: ArcGc<SharedState>,
    squares: f32,
    num_squared_values: usize,
    window_frames: usize,
    config: RmsConfig,
}

impl AudioNodeProcessor for Processor {
    // The realtime process method.
    fn process(
        &mut self,
        // The list of input buffers. This will always be equal to the number we
        // gave in `info()`.`
        inputs: &[&[f32]],
        // The list of output buffers. This will always be equal to the number we
        // gave in `info()`.`
        _outputs: &mut [&mut [f32]],
        // The list of events for our node to process.
        _events: NodeEventList,
        // Additional information about the process.
        proc_info: &ProcInfo,
        // Optional scratch buffers that can be used for processing.
        _scratch_buffers: ScratchBuffers,
    ) -> ProcessStatus {
        if !self.shared_state.enabled.load(Ordering::Relaxed) {
            self.shared_state.rms_value.store(0.0, Ordering::Relaxed);

            self.squares = 0.0;
            self.num_squared_values = 0;

            return ProcessStatus::Bypass;
        }

        let mut frames_processed = 0;
        while frames_processed < proc_info.frames {
            let process_frames = (proc_info.frames - frames_processed)
                .min(self.window_frames - self.num_squared_values);

            for &s in inputs[0][frames_processed..frames_processed + process_frames].iter() {
                self.squares += s * s;
            }

            self.num_squared_values += process_frames;
            frames_processed += process_frames;

            if self.num_squared_values == self.window_frames {
                let mean = self.squares / self.window_frames as f32;
                let rms = mean.sqrt();

                self.shared_state.rms_value.store(rms, Ordering::Relaxed);

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
