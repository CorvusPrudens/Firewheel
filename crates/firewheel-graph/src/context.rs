use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};

use firewheel_core::{
    clock::{SampleTime, SampleTimeShared, SecondsShared},
    ChannelCount, StreamInfo,
};
use rtrb::PushError;

use crate::{
    error::{ActivateCtxError, CompileGraphError},
    graph::AudioGraph,
    processor::{ContextToProcessorMsg, FirewheelProcessor, ProcessorToContextMsg},
};

const CHANNEL_CAPACITY: usize = 32;
const CLOSE_STREAM_TIMEOUT: Duration = Duration::from_secs(3);
const CLOSE_STREAM_SLEEP_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FirewheelConfig {
    /// The number of input channels in the audio graph.
    pub num_graph_inputs: ChannelCount,
    /// The number of output channels in the audio graph.
    pub num_graph_outputs: ChannelCount,
    pub initial_node_capacity: usize,
    pub initial_edge_capacity: usize,
}

impl Default for FirewheelConfig {
    fn default() -> Self {
        Self {
            num_graph_inputs: ChannelCount::ZERO,
            num_graph_outputs: ChannelCount::STEREO,
            initial_node_capacity: 64,
            initial_edge_capacity: 256,
        }
    }
}

struct ActiveState<C: Send + 'static> {
    // TODO: Do research on whether `rtrb` is compatible with
    // webassembly. If not, use conditional compilation to
    // use a different channel type when targeting webassembly.
    to_executor_tx: rtrb::Producer<ContextToProcessorMsg<C>>,
    from_executor_rx: rtrb::Consumer<ProcessorToContextMsg<C>>,

    stream_info: StreamInfo,
}

/// A firewheel context with no audio backend.
///
/// The generic is a custom global processing context that is available to
/// node processors.
pub struct FirewheelGraphCtx<C: Send + 'static> {
    graph: AudioGraph<C>,

    active_state: Option<ActiveState<C>>,
}

impl<C: Send + 'static> FirewheelGraphCtx<C> {
    pub fn new(config: FirewheelConfig) -> Self {
        Self {
            graph: AudioGraph::new(&config),
            active_state: None,
        }
    }

    /// Activate the context and return the processor to send to the audio thread.
    ///
    /// Returns an error if the context is already active.
    pub fn activate(
        &mut self,
        stream_info: StreamInfo,
        user_cx: C,
    ) -> Result<FirewheelProcessor<C>, (ActivateCtxError, C)> {
        // TODO: Return an error instead of panicking.
        assert_ne!(stream_info.sample_rate, 0);
        assert!(stream_info.max_block_samples > 0);
        assert!(stream_info.num_stream_in_channels <= 64);
        assert!(stream_info.num_stream_out_channels <= 64);

        if self.active_state.is_some() {
            return Err((ActivateCtxError::AlreadyActivated, user_cx));
        }

        let stream_time_samples_shared = Arc::new(SampleTimeShared::new(SampleTime::default()));
        let stream_time_secs_shared = Arc::new(SecondsShared::new(0.0));

        if let Err(e) = self.graph.activate(
            stream_info,
            Arc::clone(&stream_time_samples_shared),
            Arc::clone(&stream_time_secs_shared),
        ) {
            return Err((ActivateCtxError::NodeFailedToActived(e), user_cx));
        }

        let (to_executor_tx, from_graph_rx) =
            rtrb::RingBuffer::<ContextToProcessorMsg<C>>::new(CHANNEL_CAPACITY);
        let (to_graph_tx, from_executor_rx) =
            rtrb::RingBuffer::<ProcessorToContextMsg<C>>::new(CHANNEL_CAPACITY);

        self.active_state = Some(ActiveState {
            to_executor_tx,
            from_executor_rx,
            stream_info,
        });

        Ok(FirewheelProcessor::new(
            from_graph_rx,
            to_graph_tx,
            stream_time_samples_shared,
            stream_time_secs_shared,
            self.graph.current_node_capacity(),
            stream_info,
            user_cx,
        ))
    }

    /// Get an immutable reference to the audio graph.
    pub fn graph(&self) -> &AudioGraph<C> {
        &self.graph
    }

    /// Get a mutable reference to the audio graph.
    ///
    /// Returns `None` if the context is not currently activated.
    pub fn graph_mut(&mut self) -> Option<&mut AudioGraph<C>> {
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

    /// Update the firewheel context.
    ///
    /// This must be called reguarly (i.e. once every frame).
    pub fn update(&mut self) -> UpdateStatus<C> {
        self.graph.update();

        if self.active_state.is_none() {
            return UpdateStatus::Inactive;
        }

        let mut dropped = false;
        let mut dropped_user_cx = None;

        self.update_internal(&mut dropped, &mut dropped_user_cx);

        if dropped {
            self.graph.deactivate();
            self.active_state = None;
            return UpdateStatus::Deactivated {
                returned_user_cx: dropped_user_cx,
                error: None,
            };
        }

        let Some(state) = &mut self.active_state else {
            return UpdateStatus::Inactive;
        };

        if self.graph.needs_compile() {
            match self.graph.compile(state.stream_info) {
                Ok(schedule_data) => {
                    if let Err(e) = state
                        .to_executor_tx
                        .push(ContextToProcessorMsg::NewSchedule(Box::new(schedule_data)))
                    {
                        let PushError::Full(msg) = e;

                        log::error!(
                            "Failed to send new schedule: Firewheel message channel is full"
                        );

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

    /// Deactivate the firewheel context.
    ///
    /// This will block the thread until either the processor has
    /// been successfully dropped or a timeout has been reached.
    ///
    /// If the stream is still currently running, then the context
    /// will attempt to cleanly deactivate the processor. If not,
    /// then the context will wait for either the processor to be
    /// dropped or a timeout being reached.
    ///
    /// If the context is already deactivated, then this will do
    /// nothing and return `None`.
    pub fn deactivate(&mut self, stream_is_running: bool) -> Option<C> {
        let Some(state) = &mut self.active_state else {
            return None;
        };

        let start = Instant::now();

        let mut dropped = false;
        let mut dropped_user_cx = None;

        if stream_is_running {
            loop {
                if let Err(_) = state.to_executor_tx.push(ContextToProcessorMsg::Stop) {
                    log::error!("Failed to send stop signal: Firewheel message channel is full");

                    // TODO: I don't think sleep is supported in WASM, so we will
                    // need to figure out something if that's the case.
                    std::thread::sleep(CLOSE_STREAM_SLEEP_INTERVAL);

                    if start.elapsed() > CLOSE_STREAM_TIMEOUT {
                        log::error!("Timed out trying to send stop signal to firewheel processor");
                        dropped = true;
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        while !dropped {
            self.update_internal(&mut dropped, &mut dropped_user_cx);

            if !dropped {
                // TODO: I don't think sleep is supported in WASM, so we will
                // need to figure out something if that's the case.
                std::thread::sleep(CLOSE_STREAM_SLEEP_INTERVAL);

                if start.elapsed() > CLOSE_STREAM_TIMEOUT {
                    log::error!("Timed out waiting for firewheel processor to drop");
                    dropped = true;
                }
            }
        }

        self.graph.deactivate();
        self.active_state = None;

        dropped_user_cx
    }

    fn update_internal(&mut self, dropped: &mut bool, dropped_user_cx: &mut Option<C>) {
        let Some(state) = &mut self.active_state else {
            return;
        };

        while let Ok(msg) = state.from_executor_rx.pop() {
            match msg {
                ProcessorToContextMsg::ReturnSchedule(schedule_data) => {
                    self.graph.on_schedule_returned(schedule_data);
                }
                ProcessorToContextMsg::Dropped { nodes, user_cx, .. } => {
                    self.graph.on_processor_dropped(nodes);
                    *dropped = true;
                    *dropped_user_cx = user_cx;
                }
            }
        }
    }
}

impl<C: Send + 'static> Drop for FirewheelGraphCtx<C> {
    fn drop(&mut self) {
        if self.is_activated() {
            self.deactivate(true);
        }
    }
}

pub enum UpdateStatus<C: Send + 'static> {
    Inactive,
    Active {
        graph_error: Option<CompileGraphError>,
    },
    Deactivated {
        error: Option<Box<dyn Error>>,
        returned_user_cx: Option<C>,
    },
}
