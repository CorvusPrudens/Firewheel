use std::{
    any::Any,
    error::Error,
    time::{Duration, Instant},
};

use firewheel_core::{
    clock::{Clock, ClockID, ClockTime},
    StreamInfo,
};
use rtrb::PushError;
use thunderdome::Arena;

use crate::{
    graph::{AudioGraph, CompileGraphError},
    processor::{ContextToProcessorMsg, FirewheelProcessor, ProcessorToContextMsg},
};

const CHANNEL_CAPACITY: usize = 32;
const CLOSE_STREAM_TIMEOUT: Duration = Duration::from_secs(3);
const CLOSE_STREAM_SLEEP_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FirewheelConfig {
    pub num_graph_inputs: usize,
    pub num_graph_outputs: usize,
    pub initial_node_capacity: usize,
    pub initial_edge_capacity: usize,
    pub initial_clock_capacity: usize,
}

impl Default for FirewheelConfig {
    fn default() -> Self {
        Self {
            num_graph_inputs: 0,
            num_graph_outputs: 2,
            initial_node_capacity: 64,
            initial_edge_capacity: 256,
            initial_clock_capacity: 16,
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

pub struct FirewheelGraphCtx {
    pub graph: AudioGraph,
    clocks: Arena<Clock>,

    active_state: Option<ActiveState>,

    config: FirewheelConfig,
}

impl FirewheelGraphCtx {
    pub fn new(config: FirewheelConfig) -> Self {
        Self {
            graph: AudioGraph::new(&config),
            clocks: Arena::with_capacity(config.initial_clock_capacity),
            active_state: None,
            config,
        }
    }

    /// Activate the context and return the processor to send to the audio thread.
    ///
    /// Returns `None` if the context is already active.
    pub fn activate(
        &mut self,
        stream_info: StreamInfo,
        user_cx: Box<dyn Any + Send>,
    ) -> Option<FirewheelProcessor> {
        // TODO: Return an error instead of panicking.
        assert_ne!(stream_info.sample_rate, 0);
        assert!(stream_info.max_block_frames > 0);
        assert!(stream_info.num_stream_in_channels <= 64);
        assert!(stream_info.num_stream_out_channels <= 64);

        if self.active_state.is_some() {
            return None;
        }

        let (to_executor_tx, from_graph_rx) =
            rtrb::RingBuffer::<ContextToProcessorMsg>::new(CHANNEL_CAPACITY);
        let (to_graph_tx, from_executor_rx) =
            rtrb::RingBuffer::<ProcessorToContextMsg>::new(CHANNEL_CAPACITY);

        self.active_state = Some(ActiveState {
            to_executor_tx,
            from_executor_rx,
            stream_info,
        });

        Some(FirewheelProcessor::new(
            &self.config,
            from_graph_rx,
            to_graph_tx,
            self.graph.current_node_capacity(),
            stream_info,
            user_cx,
        ))
    }

    /// Returns whether or not this context is currently activated.
    pub fn is_activated(&self) -> bool {
        self.active_state.is_some()
    }

    /// Add a new clock to the system.
    ///
    /// Returns an error if the context is not activated.
    pub fn add_clock(&mut self) -> Result<ClockID, ()> {
        let Some(active_state) = &mut self.active_state else {
            // TODO: custom error
            return Err(());
        };

        let (clock, processor) = firewheel_core::clock::create_clock(
            Default::default(),
            active_state.stream_info.sample_rate,
        );

        let id = ClockID(self.clocks.insert(clock));

        if let Err(_e) = active_state
            .to_executor_tx
            .push(ContextToProcessorMsg::NewClock { id, processor })
        {
            // TODO: custom error
            return Err(());
        }

        Ok(id)
    }

    /// Remove a clock from the system.
    ///
    /// Returns `false` if the clock was already removed.
    pub fn remove_clock(&mut self, id: ClockID) -> Result<bool, ()> {
        let Some(active_state) = &mut self.active_state else {
            return Ok(false);
        };

        if self.clocks.remove(id.0).is_none() {
            return Ok(false);
        }

        if let Err(_e) = active_state
            .to_executor_tx
            .push(ContextToProcessorMsg::RemoveClock(id))
        {
            // TODO: custom error
            return Err(());
        }

        Ok(true)
    }

    /// Retrieve a clock
    ///
    /// Returns `None` if the clock no longer exists or the context is not
    /// activated.
    pub fn clock(&self, id: ClockID) -> Option<&Clock> {
        self.clocks.get(id.0)
    }

    /// Return an iterator over all of the existing clocks in the system.
    pub fn clocks_iter<'a>(&'a self) -> impl Iterator<Item = (ClockID, &'a Clock)> {
        self.clocks.iter().map(|(id, clock)| (ClockID(id), clock))
    }

    /// Retrieve a moment in time that occurs `Seconds` after the current
    /// time of the given clock.
    ///
    /// Returns `None` if the clock no longer exists or the context is not
    /// activated.
    pub fn seconds_after(&self, clock_id: ClockID, seconds: f64) -> Option<ClockTime> {
        self.clocks.get(clock_id.0).map(|clock| {
            clock
                .current_time()
                .add_secs_f64(seconds, self.stream_info().unwrap().sample_rate)
        })
    }

    /// Get info about the running audio stream.
    ///
    /// Returns `None` if the context is not activated.
    pub fn stream_info(&self) -> Option<&StreamInfo> {
        self.active_state.as_ref().map(|s| &s.stream_info)
    }

    /// Update the firewheel context.
    ///
    /// This must be called reguarly once the context has been activated
    /// (i.e. once every frame).
    pub fn update(&mut self) -> UpdateStatus {
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
            self.clocks.clear();
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
    pub fn deactivate(&mut self, stream_is_running: bool) -> Option<Box<dyn Any + Send>> {
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
        self.clocks.clear();

        dropped_user_cx
    }

    fn update_internal(
        &mut self,
        dropped: &mut bool,
        dropped_user_cx: &mut Option<Box<dyn Any + Send>>,
    ) {
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
        returned_user_cx: Option<Box<dyn Any + Send>>,
    },
}
