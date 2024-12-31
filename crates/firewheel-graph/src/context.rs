use std::{
    error::Error,
    num::NonZeroU32,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
};

use atomic_float::AtomicF64;
use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    clock::{ClockSamples, ClockSeconds},
    dsp::declick::DeclickValues,
    node::{AudioNodeProcessor, NodeEvent, NodeEventType, NodeHandle, NodeID},
    StreamInfo,
};
use rtrb::PushError;
use smallvec::SmallVec;

use crate::{
    error::{AddEdgeError, CompileGraphError},
    graph::{AudioGraph, Edge, EdgeID, NodeEntry, PortIdx},
    processor::{ContextToProcessorMsg, FirewheelProcessor, ProcessorToContextMsg},
};

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
    /// Note that most operating systems already hard clip the output,
    /// so this is usually not needed (TODO: Do research to see if this
    /// assumption is true.)
    ///
    /// By default this is set to `false`.
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
    /// The amount of time in seconds to fade in/out when pausing/resuming
    /// to avoid clicks and pops.
    ///
    /// By default this is set to `10.0 / 1_000.0`.
    pub declick_seconds: f32,
}

impl Default for FirewheelConfig {
    fn default() -> Self {
        Self {
            num_graph_inputs: ChannelCount::ZERO,
            num_graph_outputs: ChannelCount::STEREO,
            hard_clip_outputs: false,
            initial_node_capacity: 128,
            initial_edge_capacity: 256,
            initial_event_group_capacity: 128,
            channel_capacity: 64,
            event_queue_capacity: 128,
            declick_seconds: DeclickValues::DEFAULT_FADE_SECONDS,
        }
    }
}

/// A Firewheel context
pub struct FirewheelCtx {
    config: FirewheelConfig,

    graph: AudioGraph,

    to_executor_tx: rtrb::Producer<ContextToProcessorMsg>,
    from_executor_rx: rtrb::Consumer<ProcessorToContextMsg>,

    stream_info: StreamInfo,

    clock_shared: Arc<ClockValues>,

    // Re-use the allocations for groups of events.
    event_group_pool: Vec<Vec<NodeEvent>>,
    event_group: Vec<NodeEvent>,
    initial_event_group_capacity: usize,

    event_queue_tx: mpsc::Sender<NodeEvent>,
    event_queue_rx: mpsc::Receiver<NodeEvent>,

    stream_crashed: bool,
}

impl FirewheelCtx {
    /// Create a new Firewheel context and return the processor to send to the
    /// audio thread.
    pub fn new(config: FirewheelConfig, mut stream_info: StreamInfo) -> (Self, FirewheelProcessor) {
        // TODO: Return an error instead of panicking.
        assert!(stream_info.num_stream_in_channels <= 64);
        assert!(stream_info.num_stream_out_channels <= 64);

        stream_info.sample_rate_recip = (stream_info.sample_rate.get() as f64).recip();

        stream_info.declick_frames = NonZeroU32::new(
            (config.declick_seconds as f64 * stream_info.sample_rate.get() as f64).round() as u32,
        )
        .unwrap_or(NonZeroU32::MIN);

        let clock_shared = Arc::new(ClockValues {
            seconds: AtomicF64::new(0.0),
            samples: AtomicU64::new(0),
        });

        let (to_executor_tx, from_graph_rx) =
            rtrb::RingBuffer::<ContextToProcessorMsg>::new(config.channel_capacity as usize);
        let (to_graph_tx, from_executor_rx) =
            rtrb::RingBuffer::<ProcessorToContextMsg>::new(config.channel_capacity as usize * 4);

        let initial_event_group_capacity = config.initial_event_group_capacity as usize;
        let mut event_group_pool = Vec::with_capacity(16);
        for _ in 0..3 {
            event_group_pool.push(Vec::with_capacity(initial_event_group_capacity));
        }

        let (event_queue_tx, event_queue_rx) = mpsc::channel::<NodeEvent>();

        (
            Self {
                graph: AudioGraph::new(&config),
                to_executor_tx,
                from_executor_rx,
                stream_info,
                clock_shared: Arc::clone(&clock_shared),
                event_group_pool,
                event_group: Vec::with_capacity(initial_event_group_capacity),
                initial_event_group_capacity,
                event_queue_tx,
                event_queue_rx,
                config,
                stream_crashed: false,
            },
            FirewheelProcessor::new(
                from_graph_rx,
                to_graph_tx,
                clock_shared,
                config.initial_node_capacity as usize,
                stream_info,
                config.hard_clip_outputs,
            ),
        )
    }

    /// The ID of the graph input node
    pub fn graph_in_node(&self) -> NodeID {
        self.graph.graph_in_node()
    }

    /// The ID of the graph output node
    pub fn graph_out_node(&self) -> NodeID {
        self.graph.graph_out_node()
    }

    /// Information about the running audio stream.
    pub fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    /// Add a node to the audio graph.
    ///
    /// This method is intended to be used inside the constructors of nodes.
    ///
    /// * `debug_name` - The name of this type of audio node for debugging
    /// purposes.
    /// * `channel_config` - The channel configuration of this node.
    /// * `uses_events` - Whether or not this node reads any events in
    /// [`AudioNodeProcessor::process`]. Setting this to `false` will skip
    /// allocating an event buffer for this node.
    /// * `processor` - The processor counterpart to send to the audio thread.
    pub fn add_node(
        &mut self,
        debug_name: &'static str,
        channel_config: ChannelConfig,
        uses_events: bool,
        processor: Box<dyn AudioNodeProcessor>,
    ) -> NodeHandle {
        let id = self
            .graph
            .add_node(debug_name, channel_config, uses_events, processor);

        NodeHandle {
            id,
            event_queue_sender: self.event_queue_tx.clone(),
        }
    }

    /// Get a list of all the existing nodes in the graph.
    pub fn nodes<'a>(&'a self) -> impl Iterator<Item = &'a NodeEntry> {
        self.graph.nodes()
    }

    /// Get a list of all the existing edges in the graph.
    pub fn edges<'a>(&'a self) -> impl Iterator<Item = &'a Edge> {
        self.graph.edges()
    }

    /// Set the number of input and output channels to and from the audio graph.
    ///
    /// Returns the list of edges that were removed.
    pub fn set_graph_channel_config(
        &mut self,
        channel_config: ChannelConfig,
    ) -> Result<Vec<EdgeID>, Box<dyn Error>> {
        self.graph.set_graph_channel_config(channel_config)
    }

    /// Add connections (edges) between two nodes in the graph.
    ///
    /// * `src_node` - The ID of the source node.
    /// * `dst_node` - The ID of the destination node.
    /// * `ports_src_dst` - The port indices for each connection to make,
    /// where the first value in a tuple is the output port on `src_node`,
    /// and the second value in that tuple is the input port on `dst_node`.
    /// * `check_for_cycles` - If `true`, then this will run a check to
    /// see if adding these edges will create a cycle in the graph, and
    /// return an error if it does. Note, checking for cycles can be quite
    /// expensive, so avoid enabling this when calling this method many times
    /// in a row.
    ///
    /// If successful, then this returns a list of edge IDs in order.
    ///
    /// If this returns an error, then the audio graph has not been
    /// modified.
    pub fn connect(
        &mut self,
        src_node: NodeID,
        dst_node: NodeID,
        ports_src_dst: &[(PortIdx, PortIdx)],
        check_for_cycles: bool,
    ) -> Result<SmallVec<[EdgeID; 8]>, AddEdgeError> {
        self.graph
            .connect(src_node, dst_node, ports_src_dst, check_for_cycles)
    }

    /// Remove connections (edges) between two nodes from the graph.
    ///
    /// * `src_node` - The ID of the source node.
    /// * `dst_node` - The ID of the destination node.
    /// * `ports_src_dst` - The port indices for each connection to make,
    /// where the first value in a tuple is the output port on `src_node`,
    /// and the second value in that tuple is the input port on `dst_node`.
    ///
    /// If none of the edges existed in the graph, then `false` will be
    /// returned.
    pub fn disconnect(
        &mut self,
        src_node: NodeID,
        dst_node: NodeID,
        ports_src_dst: &[(PortIdx, PortIdx)],
    ) -> bool {
        self.graph.disconnect(src_node, dst_node, ports_src_dst)
    }

    /// Remove a connection (edge) via the edge's unique ID.
    ///
    /// If the edge did not exist in this graph, then `false` will be returned.
    pub fn disconnect_by_edge_id(&mut self, edge_id: EdgeID) -> bool {
        self.graph.disconnect_by_edge_id(edge_id)
    }

    /// Get information about the given [Edge]
    pub fn edge(&self, edge_id: EdgeID) -> Option<&Edge> {
        self.graph.edge(edge_id)
    }

    /// The current time of the clock in the number of seconds since the stream
    /// was started.
    pub fn clock_now(&self) -> ClockSeconds {
        ClockSeconds(self.clock_shared.seconds.load(Ordering::Relaxed))
    }

    /// The current time of the sample clock in the number of samples that have
    /// been processed since the beginning of the stream.
    pub fn clock_samples(&self) -> ClockSamples {
        ClockSamples(self.clock_shared.samples.load(Ordering::Relaxed))
    }

    pub fn cycle_detected(&mut self) -> bool {
        self.graph.cycle_detected()
    }

    pub fn needs_compile(&self) -> bool {
        self.graph.needs_compile()
    }

    /// Whether or not outputs are being hard clipped at 0dB.
    pub fn hard_clip_outputs(&self) -> bool {
        self.config.hard_clip_outputs
    }

    /// Set whether or not outputs should be hard clipped at 0dB to
    /// help protect the system's speakers.
    ///
    /// Note that most operating systems already hard clip the output,
    /// so this is usually not needed (TODO: Do research to see if this
    /// assumption is true.)
    pub fn set_hard_clip_outputs(&mut self, hard_clip_outputs: bool) {
        if self.config.hard_clip_outputs == hard_clip_outputs {
            return;
        }
        self.config.hard_clip_outputs = hard_clip_outputs;

        let _ = self
            .send_message_to_processor(ContextToProcessorMsg::HardClipOutputs(hard_clip_outputs));
    }

    /// Update the firewheel context.
    ///
    /// This must be called reguarly (i.e. once every frame).
    ///
    /// This should be called in an `update` method in the backend's context.
    #[must_use]
    pub fn _update(mut self) -> UpdateStatusInner {
        if self.stream_crashed {
            return UpdateStatusInner::Deactivated { error: None };
        }

        let mut dropped = false;
        while let Ok(msg) = self.from_executor_rx.pop() {
            match msg {
                ProcessorToContextMsg::ReturnCustomEvent(event) => {
                    let _ = event;
                }
                ProcessorToContextMsg::ReturnEventGroup(mut event_group) => {
                    event_group.clear();
                    self.event_group_pool.push(event_group);
                }
                ProcessorToContextMsg::ReturnSchedule(schedule_data) => {
                    let _ = schedule_data;
                }
                ProcessorToContextMsg::Dropped { .. } => {
                    dropped = true;
                }
            }
        }

        if dropped {
            return UpdateStatusInner::Deactivated { error: None };
        }

        let mut nodes_to_drop = Vec::new();

        for event in self.event_queue_rx.try_iter() {
            if let NodeEventType::_Dropped = &event.event {
                nodes_to_drop.push(event.node_id);
            } else {
                self.event_group.push(event);
            }
        }

        for node_id in nodes_to_drop.drain(..) {
            self.graph.remove_node(node_id);
        }

        if !self.event_group.is_empty() {
            let mut next_event_group = self
                .event_group_pool
                .pop()
                .unwrap_or_else(|| Vec::with_capacity(self.initial_event_group_capacity));
            std::mem::swap(&mut next_event_group, &mut self.event_group);

            if let Err(msg) =
                self.send_message_to_processor(ContextToProcessorMsg::EventGroup(next_event_group))
            {
                if let ContextToProcessorMsg::EventGroup(event_group) = msg {
                    self.event_group_pool.push(event_group);
                }
            }
        }

        if self.graph.needs_compile() {
            match self.graph.compile(self.stream_info) {
                Ok(schedule_data) => {
                    if let Err(msg) = self.send_message_to_processor(
                        ContextToProcessorMsg::NewSchedule(Box::new(schedule_data)),
                    ) {
                        if let ContextToProcessorMsg::NewSchedule(schedule_data) = msg {
                            let _ = schedule_data;
                        }
                    }
                }
                Err(e) => {
                    return UpdateStatusInner::Ok {
                        cx: self,
                        graph_compile_error: Some(e),
                    };
                }
            }
        }

        UpdateStatusInner::Ok {
            cx: self,
            graph_compile_error: None,
        }
    }

    /// Notify the context that the audio stream has stopped due to an unexpected error.
    pub fn _notify_stream_crashed(&mut self) {
        self.stream_crashed = true;
    }

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

impl Drop for FirewheelCtx {
    fn drop(&mut self) {
        if !self.stream_crashed {
            let _ = self.send_message_to_processor(ContextToProcessorMsg::Stop);
        }
    }
}

pub(crate) struct ClockValues {
    pub seconds: AtomicF64,
    pub samples: AtomicU64,
}

pub enum UpdateStatusInner {
    Ok {
        cx: FirewheelCtx,
        graph_compile_error: Option<CompileGraphError>,
    },
    /// The engine was deactivated.
    ///
    /// If this is returned, then all node handles are invalidated.
    /// The graph and all its nodes must be reconstructed.
    Deactivated { error: Option<Box<dyn Error>> },
}
