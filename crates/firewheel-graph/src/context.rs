use std::{
    error::Error,
    time::{Duration, Instant},
};

use firewheel_core::{ChannelCount, StreamInfo};
use rtrb::PushError;

use crate::{
    error::{ActivateCtxError, CompileGraphError},
    graph::AudioGraph,
    processor::{ContextToProcessorMsg, FirewheelProcessor, ProcessorToContextMsg},
};

const CLOSE_STREAM_TIMEOUT: Duration = Duration::from_secs(3);
const CLOSE_STREAM_SLEEP_INTERVAL: Duration = Duration::from_millis(2);

/// The configuration of a Firewheel context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FirewheelConfig {
    /// The number of input channels in the audio graph.
    pub num_graph_inputs: ChannelCount,
    /// The number of output channels in the audio graph.
    pub num_graph_outputs: ChannelCount,
    /// If `true`, then all outputs will be hard clipped at 0db to help
    /// protect the system's speakers.
    ///
    /// By default this is set to `true`.
    pub hard_clip_outputs: bool,
    /// An initial capacity to allocate for the nodes in the audio graph.
    ///
    /// By default this is set to `64`.
    pub initial_node_capacity: u32,
    /// An initial capacity to allocate for the edges in the audio graph.
    ///
    /// By default this is set to `256`.
    pub initial_edge_capacity: u32,
    /// The initial capacity for a group of events.
    ///
    /// By default this is set to `128`.
    pub initial_event_group_capacity: u32,
    /// The capacity of the engine's internal message channel.
    ///
    /// By default this is set to `64`.
    pub channel_capacity: u32,
    /// The capacity of an event queue in the engine (one event queue per node).
    ///
    /// By default this is set to `128`.
    pub event_queue_capacity: u32,
}

impl Default for FirewheelConfig {
    fn default() -> Self {
        Self {
            num_graph_inputs: ChannelCount::ZERO,
            num_graph_outputs: ChannelCount::STEREO,
            hard_clip_outputs: true,
            initial_node_capacity: 64,
            initial_edge_capacity: 256,
            initial_event_group_capacity: 128,
            channel_capacity: 64,
            event_queue_capacity: 128,
        }
    }
}

struct ActiveState {
    // TODO: Do research on whether `rtrb` is compatible with
    // webassembly. If not, use conditional compilation to
    // use a different channel type when targeting webassembly.
    to_executor_tx: rtrb::Producer<ContextToProcessorMsg>,
    from_executor_rx: rtrb::Consumer<ProcessorToContextMsg>,

    stream_info: StreamInfo,
}

impl ActiveState {
    fn send_message_to_processor(
        &mut self,
        msg: ContextToProcessorMsg,
    ) -> Result<(), ContextToProcessorMsg> {
        if let Err(e) = self.to_executor_tx.push(msg) {
            let PushError::Full(msg) = e;

            log::error!("Firewheel message channel is full!");

            Err(msg)
        } else {
            Ok(())
        }
    }
}

/// A firewheel context with no audio backend.
///
/// The generic is a custom global processing context that is available to
/// node processors.
pub struct FirewheelGraphCtx {
    graph: AudioGraph,
    config: FirewheelConfig,

    active_state: Option<ActiveState>,
}

impl FirewheelGraphCtx {
    pub fn new(config: FirewheelConfig) -> Self {
        Self {
            graph: AudioGraph::new(&config),
            config,
            active_state: None,
        }
    }

    /// Activate the context and return the processor to send to the audio thread.
    ///
    /// Returns an error if the context is already active.
    pub fn activate(
        &mut self,
        stream_info: StreamInfo,
    ) -> Result<FirewheelProcessor, ActivateCtxError> {
        // TODO: Return an error instead of panicking.
        assert_ne!(stream_info.sample_rate, 0);
        assert!(stream_info.max_block_samples > 0);
        assert!(stream_info.num_stream_in_channels <= 64);
        assert!(stream_info.num_stream_out_channels <= 64);

        if self.active_state.is_some() {
            return Err(ActivateCtxError::AlreadyActivated);
        }

        let main_thread_clock_start_instant = Instant::now();

        if let Err(e) = self
            .graph
            .activate(stream_info, main_thread_clock_start_instant)
        {
            return Err(ActivateCtxError::NodeFailedToActived(e));
        }

        let (to_executor_tx, from_graph_rx) =
            rtrb::RingBuffer::<ContextToProcessorMsg>::new(self.config.channel_capacity as usize);
        let (to_graph_tx, from_executor_rx) = rtrb::RingBuffer::<ProcessorToContextMsg>::new(
            self.config.channel_capacity as usize * 4,
        );

        self.active_state = Some(ActiveState {
            to_executor_tx,
            from_executor_rx,
            stream_info,
        });

        Ok(FirewheelProcessor::new(
            from_graph_rx,
            to_graph_tx,
            main_thread_clock_start_instant,
            self.graph.current_node_capacity(),
            stream_info,
            self.config.hard_clip_outputs,
        ))
    }

    /// Get an immutable reference to the audio graph.
    pub fn graph(&self) -> &AudioGraph {
        &self.graph
    }

    /// Get a mutable reference to the audio graph.
    ///
    /// Returns `None` if the context is not currently activated.
    pub fn graph_mut(&mut self) -> Option<&mut AudioGraph> {
        if self.is_activated() {
            Some(&mut self.graph)
        } else {
            None
        }
    }

    /// Returns whether or not this context is currently activated.
    pub fn is_activated(&self) -> bool {
        self.active_state.is_some()
    }

    /// Get info about the running audio stream.
    ///
    /// Returns `None` if the context is not activated.
    pub fn stream_info(&self) -> Option<&StreamInfo> {
        self.active_state.as_ref().map(|s| &s.stream_info)
    }

    /// Whether or not outputs are being hard clipped at 0dB.
    pub fn hard_clip_outputs(&self) -> bool {
        self.config.hard_clip_outputs
    }

    /// Set whether or not outputs should be hard clipped at 0dB to
    /// help protect the system's speakers.
    pub fn set_hard_clip_outputs(&mut self, hard_clip_outputs: bool) {
        if self.config.hard_clip_outputs == hard_clip_outputs {
            return;
        }
        self.config.hard_clip_outputs = hard_clip_outputs;

        if let Some(state) = &mut self.active_state {
            let _ = state.send_message_to_processor(ContextToProcessorMsg::HardClipOutputs(
                hard_clip_outputs,
            ));
        };
    }

    /// Update the firewheel context.
    ///
    /// This must be called reguarly (i.e. once every frame).
    #[must_use]
    pub fn update(&mut self) -> UpdateStatus {
        self.graph.update();

        if self.active_state.is_none() {
            return UpdateStatus::Inactive;
        }

        let mut dropped = false;

        self.update_internal(&mut dropped);

        if dropped {
            self.graph.deactivate();
            self.active_state = None;
            return UpdateStatus::Deactivated { error: None };
        }

        let Some(state) = &mut self.active_state else {
            return UpdateStatus::Inactive;
        };

        if self.graph.needs_compile() {
            match self.graph.compile(state.stream_info) {
                Ok(schedule_data) => {
                    if let Err(msg) = state.send_message_to_processor(
                        ContextToProcessorMsg::NewSchedule(Box::new(schedule_data)),
                    ) {
                        if let ContextToProcessorMsg::NewSchedule(schedule_data) = msg {
                            self.graph.on_schedule_returned(schedule_data);
                        }
                    }
                }
                Err(e) => {
                    return UpdateStatus::Active {
                        graph_error: Some(e),
                    };
                }
            }
        }

        UpdateStatus::Active { graph_error: None }
    }

    /// Flush the event queue.
    ///
    /// If the context is not currently activated, then this will do
    /// nothing.
    pub fn flush_events(&mut self) {
        let Some(state) = &mut self.active_state else {
            return;
        };

        let Some(event_group) = self.graph.flush_events() else {
            return;
        };

        if let Err(msg) =
            state.send_message_to_processor(ContextToProcessorMsg::EventGroup(event_group))
        {
            if let ContextToProcessorMsg::EventGroup(event_group) = msg {
                self.graph.return_event_group(event_group);
            }
        }
    }

    /// Deactivate the firewheel context.
    ///
    /// On native platforms, this will block the thread until either
    /// the processor has been successfully dropped or a timeout has
    /// been reached.
    ///
    /// On WebAssembly, this will *NOT* wait for the processor to be
    /// successfully dropped.
    ///
    /// If the context is already deactivated, then this will do
    /// nothing and return `false`.
    pub fn deactivate(&mut self, stream_is_running: bool) -> bool {
        let Some(state) = &mut self.active_state else {
            return false;
        };

        let start = Instant::now();

        let mut dropped = false;

        #[cfg(not(target_family = "wasm"))]
        {
            if stream_is_running {
                loop {
                    if let Err(_) = state.to_executor_tx.push(ContextToProcessorMsg::Stop) {
                        log::error!(
                            "Failed to send stop signal: Firewheel message channel is full"
                        );

                        std::thread::sleep(CLOSE_STREAM_SLEEP_INTERVAL);

                        if start.elapsed() > CLOSE_STREAM_TIMEOUT {
                            log::error!(
                                "Timed out trying to send stop signal to firewheel processor"
                            );
                            dropped = true;
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }

            while !dropped {
                self.update_internal(&mut dropped);

                if !dropped {
                    std::thread::sleep(CLOSE_STREAM_SLEEP_INTERVAL);

                    if start.elapsed() > CLOSE_STREAM_TIMEOUT {
                        log::error!("Timed out waiting for firewheel processor to drop");
                        dropped = true;
                    }
                }
            }
        }

        #[cfg(target_family = "wasm")]
        {
            self.update_internal(&mut dropped, &mut dropped_user_cx);
        }

        self.graph.deactivate();
        self.active_state = None;

        true
    }

    fn update_internal(&mut self, dropped: &mut bool) {
        let Some(state) = &mut self.active_state else {
            return;
        };

        while let Ok(msg) = state.from_executor_rx.pop() {
            match msg {
                ProcessorToContextMsg::ReturnCustomEvent(event) => {
                    let _ = event;
                }
                ProcessorToContextMsg::ReturnEventGroup(event_group) => {
                    self.graph.return_event_group(event_group);
                }
                ProcessorToContextMsg::ReturnSchedule(schedule_data) => {
                    self.graph.on_schedule_returned(schedule_data);
                }
                ProcessorToContextMsg::Dropped { nodes, .. } => {
                    self.graph.on_processor_dropped(nodes);
                    *dropped = true;
                }
            }
        }
    }
}

impl Drop for FirewheelGraphCtx {
    fn drop(&mut self) {
        if self.is_activated() {
            self.deactivate(true);
        }
    }
}

pub enum UpdateStatus {
    Inactive,
    Active {
        graph_error: Option<CompileGraphError>,
    },
    Deactivated {
        error: Option<Box<dyn Error>>,
    },
}
