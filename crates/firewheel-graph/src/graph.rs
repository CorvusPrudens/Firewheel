mod compiler;

use std::error::Error;
use std::fmt::Debug;
use std::hash::Hash;
use std::time::Instant;

use ahash::{AHashMap, AHashSet};
use firewheel_core::clock::ClockSeconds;
use firewheel_core::{ChannelConfig, ChannelCount, StreamInfo};
use smallvec::SmallVec;
use thunderdome::Arena;

use crate::basic_nodes::dummy::DummyAudioNode;
use crate::context::FirewheelConfig;
use crate::error::{AddEdgeError, CompileGraphError, NodeError};
use firewheel_core::node::{AudioNode, NodeEvent, NodeID};

pub(crate) use self::compiler::{CompiledSchedule, NodeHeapData, ScheduleHeapData};

pub use self::compiler::{Edge, EdgeID, NodeEntry, PortIdx};

pub struct NodeWeight {
    pub node: Box<dyn AudioNode>,
    pub activated: bool,
    pub updates: bool,
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
struct EdgeHash {
    pub src_node: NodeID,
    pub dst_node: NodeID,
    pub src_port: PortIdx,
    pub dst_port: PortIdx,
}

struct ActiveState {
    stream_info: StreamInfo,
    main_thread_clock_start_instant: Instant,
}

/// An audio graph implementation.
///
/// The generic is a custom global processing context that is available to
/// node processors.
pub struct AudioGraph {
    nodes: Arena<NodeEntry<NodeWeight>>,
    edges: Arena<Edge>,
    connected_input_ports: AHashSet<(NodeID, PortIdx)>,
    existing_edges: AHashMap<EdgeHash, EdgeID>,

    graph_in_id: NodeID,
    graph_out_id: NodeID,
    needs_compile: bool,

    active_state: Option<ActiveState>,

    nodes_to_remove_from_schedule: Vec<NodeID>,
    active_nodes_to_remove: AHashMap<NodeID, NodeEntry<NodeWeight>>,
    new_node_processors: Vec<NodeHeapData>,
    event_queue_capacity: usize,

    // Re-uses the allocations for groups of events.
    event_group_pool: Vec<Vec<NodeEvent>>,
    event_group: Vec<NodeEvent>,
    initial_event_group_capacity: usize,
}

impl AudioGraph {
    pub(crate) fn new(config: &FirewheelConfig) -> Self {
        let mut nodes = Arena::with_capacity(config.initial_node_capacity as usize);

        let graph_in_id = NodeID {
            idx: nodes.insert(NodeEntry::new(
                ChannelConfig {
                    num_inputs: ChannelCount::ZERO,
                    num_outputs: config.num_graph_inputs,
                },
                NodeWeight {
                    node: Box::new(DummyAudioNode),
                    activated: false,
                    updates: false,
                },
            )),
            debug_name: "graph_in",
        };
        nodes[graph_in_id.idx].id = graph_in_id;

        let graph_out_id = NodeID {
            idx: nodes.insert(NodeEntry::new(
                ChannelConfig {
                    num_inputs: config.num_graph_outputs,
                    num_outputs: ChannelCount::ZERO,
                },
                NodeWeight {
                    node: Box::new(DummyAudioNode),
                    activated: false,
                    updates: false,
                },
            )),
            debug_name: "graph_out",
        };
        nodes[graph_out_id.idx].id = graph_out_id;

        let initial_event_group_capacity = config.initial_event_group_capacity as usize;
        let mut event_group_pool = Vec::with_capacity(16);
        for _ in 0..3 {
            event_group_pool.push(Vec::with_capacity(initial_event_group_capacity));
        }

        Self {
            nodes,
            edges: Arena::with_capacity(config.initial_edge_capacity as usize),
            connected_input_ports: AHashSet::with_capacity(config.initial_edge_capacity as usize),
            existing_edges: AHashMap::with_capacity(config.initial_edge_capacity as usize),
            graph_in_id,
            graph_out_id,
            needs_compile: true,
            active_state: None,
            nodes_to_remove_from_schedule: Vec::with_capacity(
                config.initial_node_capacity as usize,
            ),
            active_nodes_to_remove: AHashMap::with_capacity(config.initial_edge_capacity as usize),
            new_node_processors: Vec::with_capacity(config.initial_node_capacity as usize),
            event_queue_capacity: config.event_queue_capacity as usize,
            event_group_pool,
            event_group: Vec::with_capacity(initial_event_group_capacity),
            initial_event_group_capacity,
        }
    }

    /// Queue an event to be sent to a node's processor.
    ///
    /// Note, events in the queue will not be sent until `FirewheelGraphCtx::flush_events()`
    /// is called.
    ///
    /// If a node with the given ID does not exist in the graph, then the event will be
    /// ignored.
    pub fn queue_event(&mut self, event: NodeEvent) {
        if !self.nodes.contains(event.node_id.idx) {
            return;
        }

        self.event_group.push(event);
    }

    /// Remove all existing nodes from the graph.
    pub fn reset(&mut self) {
        let nodes_to_remove = self
            .nodes
            .iter()
            .map(|(_, node_entry)| node_entry.id)
            .filter(|&id| id != self.graph_in_id && id != self.graph_out_id)
            .collect::<Vec<_>>();

        for node_id in nodes_to_remove {
            self.remove_node(node_id).unwrap();
        }
    }

    pub fn current_node_capacity(&self) -> usize {
        self.nodes.capacity()
    }

    /// The ID of the graph input node
    pub fn graph_in_node(&self) -> NodeID {
        self.graph_in_id
    }

    /// The ID of the graph output node
    pub fn graph_out_node(&self) -> NodeID {
        self.graph_out_id
    }

    /// Add a new [`AudioNode`] the the audio graph.
    ///
    /// This will return the globally unique ID assigned to this node.
    ///
    /// * `custom_channel_config` - A custom channel configuration to use for
    /// this node. Set this to `None` to use the default configuration.
    pub fn add_node(
        &mut self,
        mut node: Box<dyn AudioNode>,
        custom_channel_config: Option<ChannelConfig>,
    ) -> Result<NodeID, NodeError> {
        let stream_info = &self.active_state.as_ref().unwrap().stream_info;

        let debug_name = node.debug_name();

        let info = node.info();

        assert!(info.num_min_supported_inputs <= info.num_max_supported_inputs);
        assert!(info.num_min_supported_outputs <= info.num_max_supported_outputs);

        let channel_config = custom_channel_config.unwrap_or(info.default_channel_config);

        if channel_config.num_inputs < info.num_min_supported_inputs
            || channel_config.num_inputs > info.num_max_supported_inputs
            || channel_config.num_outputs < info.num_min_supported_outputs
            || channel_config.num_outputs > info.num_max_supported_outputs
        {
            return Err(NodeError::InvalidChannelConfig {
                channel_config,
                node_info: info,
                msg: None,
            });
        }

        if info.equal_num_ins_and_outs {
            if channel_config.num_inputs != channel_config.num_outputs {
                return Err(NodeError::InvalidChannelConfig {
                    channel_config,
                    node_info: info,
                    msg: None,
                });
            }
        }

        if let Err(e) = node.channel_config_supported(channel_config) {
            return Err(NodeError::InvalidChannelConfig {
                channel_config,
                node_info: info,
                msg: Some(e),
            });
        }

        let processor = node.activate(&stream_info, channel_config).map_err(|e| {
            NodeError::ActivationFailed {
                node_id: None,
                error: e,
            }
        })?;

        let new_id = NodeID {
            idx: self.nodes.insert(NodeEntry::new(
                channel_config,
                NodeWeight {
                    node,
                    activated: false,
                    updates: info.updates,
                },
            )),
            debug_name,
        };
        self.nodes[new_id.idx].id = new_id;

        self.new_node_processors.push(NodeHeapData::new(
            new_id,
            processor,
            self.event_queue_capacity,
            info.uses_events,
        ));

        self.needs_compile = true;

        Ok(new_id)
    }

    /// Get an immutable reference to a node.
    ///
    /// This will return `None` if a node with the given ID does not
    /// exist in the graph, or if the node doesn't match the given
    /// type.
    pub fn node<N: AudioNode>(&self, node_id: NodeID) -> Option<&N> {
        self.nodes
            .get(node_id.idx)
            .and_then(|n| n.weight.node.downcast_ref::<N>())
    }

    /// Get a mutable reference to the node.
    ///
    /// This will return `None` if a node with the given ID does not
    /// exist in the graph, or if the node doesn't match the given
    /// type.
    pub fn node_mut<N: AudioNode>(&mut self, node_id: NodeID) -> Option<&mut N> {
        self.nodes
            .get_mut(node_id.idx)
            .and_then(|n| n.weight.node.downcast_mut::<N>())
    }

    /// Get info about a node.
    ///
    /// This will return `None` if a node with the given ID does not
    /// exist in the graph.
    pub fn node_info(&self, node_id: NodeID) -> Option<&NodeEntry<NodeWeight>> {
        self.nodes.get(node_id.idx)
    }

    /// Remove the given node from the graph.
    ///
    /// This will automatically remove all edges from the graph that
    /// were connected to this node.
    ///
    /// On success, this returns a list of all edges that were removed
    /// from the graph as a result of removing this node.
    ///
    /// This will return an error if a node with the given ID does not
    /// exist in the graph, or if the ID is of the graph input or graph
    /// output node.
    pub fn remove_node(&mut self, node_id: NodeID) -> Result<Vec<EdgeID>, ()> {
        if node_id == self.graph_in_id || node_id == self.graph_out_id {
            return Err(());
        }

        let node_entry = self.nodes.remove(node_id.idx).ok_or(())?;

        let mut removed_edges: Vec<EdgeID> = Vec::new();

        for port_idx in 0..node_entry.channel_config.num_inputs.get() {
            removed_edges.append(&mut self.remove_edges_with_input_port(node_id, port_idx));
        }
        for port_idx in 0..node_entry.channel_config.num_outputs.get() {
            removed_edges.append(&mut self.remove_edges_with_output_port(node_id, port_idx));
        }

        for port_idx in 0..node_entry.channel_config.num_inputs.get() {
            self.connected_input_ports.remove(&(node_id, port_idx));
        }

        self.nodes_to_remove_from_schedule.push(node_id);

        if node_entry.weight.activated {
            self.active_nodes_to_remove.insert(node_id, node_entry);
        }

        self.needs_compile = true;
        Ok(removed_edges)
    }

    /// Get a list of all the existing nodes in the graph.
    pub fn nodes<'a>(&'a self) -> impl Iterator<Item = &'a NodeEntry<NodeWeight>> {
        self.nodes.iter().map(|(_, n)| n)
    }

    /// Get a list of all the existing edges in the graph.
    pub fn edges<'a>(&'a self) -> impl Iterator<Item = &'a Edge> {
        self.edges.iter().map(|(_, e)| e)
    }

    /// Set the number of input and output channels to and from the audio graph.
    ///
    /// Returns the list of edges that were removed.
    pub fn set_graph_channel_config(
        &mut self,
        channel_config: ChannelConfig,
    ) -> Result<Vec<EdgeID>, Box<dyn Error>> {
        let mut removed_edges = Vec::new();

        let graph_in_node = self.nodes.get_mut(self.graph_in_id.idx).unwrap();
        if channel_config.num_inputs != graph_in_node.channel_config.num_outputs {
            let old_num_inputs = graph_in_node.channel_config.num_outputs;
            graph_in_node.channel_config.num_outputs = channel_config.num_inputs;

            if channel_config.num_inputs < old_num_inputs {
                for port_idx in channel_config.num_inputs.get()..old_num_inputs.get() {
                    removed_edges.append(
                        &mut self.remove_edges_with_output_port(self.graph_in_id, port_idx),
                    );
                }
            }

            self.needs_compile = true;
        }

        let graph_out_node = self.nodes.get_mut(self.graph_in_id.idx).unwrap();

        if channel_config.num_outputs != graph_out_node.channel_config.num_inputs {
            let old_num_outputs = graph_out_node.channel_config.num_inputs;
            graph_out_node.channel_config.num_inputs = channel_config.num_outputs;

            if channel_config.num_outputs < old_num_outputs {
                for port_idx in channel_config.num_outputs.get()..old_num_outputs.get() {
                    removed_edges.append(
                        &mut self.remove_edges_with_input_port(self.graph_out_id, port_idx),
                    );
                    self.connected_input_ports
                        .remove(&(self.graph_out_id, port_idx));
                }
            }

            self.needs_compile = true;
        }

        Ok(removed_edges)
    }

    /// Add connections (edges) between two nodes to the graph.
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
        let src_node_entry = self
            .nodes
            .get(src_node.idx)
            .ok_or(AddEdgeError::SrcNodeNotFound(src_node))?;
        let dst_node_entry = self
            .nodes
            .get(dst_node.idx)
            .ok_or(AddEdgeError::DstNodeNotFound(dst_node))?;

        if src_node.idx == dst_node.idx {
            return Err(AddEdgeError::CycleDetected);
        }

        for (src_port, dst_port) in ports_src_dst.iter().copied() {
            if src_port >= src_node_entry.channel_config.num_outputs.get() {
                return Err(AddEdgeError::OutPortOutOfRange {
                    node: src_node,
                    port_idx: src_port,
                    num_out_ports: src_node_entry.channel_config.num_outputs,
                });
            }
            if dst_port >= dst_node_entry.channel_config.num_inputs.get() {
                return Err(AddEdgeError::InPortOutOfRange {
                    node: dst_node,
                    port_idx: dst_port,
                    num_in_ports: dst_node_entry.channel_config.num_inputs,
                });
            }

            if self.existing_edges.contains_key(&EdgeHash {
                src_node,
                src_port,
                dst_node,
                dst_port,
            }) {
                return Err(AddEdgeError::EdgeAlreadyExists);
            }

            if self.connected_input_ports.contains(&(dst_node, dst_port)) {
                return Err(AddEdgeError::InputPortAlreadyConnected(dst_node, dst_port));
            }
        }

        let mut edge_ids = SmallVec::new();

        for (src_port, dst_port) in ports_src_dst.iter().copied() {
            if self.existing_edges.contains_key(&EdgeHash {
                src_node,
                src_port,
                dst_node,
                dst_port,
            }) {
                // The caller gave us more than one of the same edge.
                continue;
            }

            self.connected_input_ports.insert((dst_node, dst_port));

            let new_edge_id = EdgeID(self.edges.insert(Edge {
                id: EdgeID(thunderdome::Index::DANGLING),
                src_node,
                src_port,
                dst_node,
                dst_port,
            }));
            self.edges[new_edge_id.0].id = new_edge_id;
            self.existing_edges.insert(
                EdgeHash {
                    src_node,
                    src_port,
                    dst_node,
                    dst_port,
                },
                new_edge_id,
            );

            edge_ids.push(new_edge_id);
        }

        if check_for_cycles {
            if self.cycle_detected() {
                self.disconnect(src_node, dst_node, ports_src_dst);

                return Err(AddEdgeError::CycleDetected);
            }
        }

        self.needs_compile = true;

        Ok(edge_ids)
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
        let mut any_removed = false;

        for (src_port, dst_port) in ports_src_dst.iter().copied() {
            if let Some(edge_id) = self.existing_edges.remove(&EdgeHash {
                src_node,
                src_port: src_port.into(),
                dst_node,
                dst_port: dst_port.into(),
            }) {
                self.disconnect_by_edge_id(edge_id);
                any_removed = true;
            }
        }

        any_removed
    }

    /// Remove a connection (edge) via the edge's unique ID.
    ///
    /// If the edge did not exist in this graph, then `false` will be returned.
    pub fn disconnect_by_edge_id(&mut self, edge_id: EdgeID) -> bool {
        if let Some(edge) = self.edges.remove(edge_id.0) {
            self.existing_edges.remove(&EdgeHash {
                src_node: edge.src_node,
                src_port: edge.src_port,
                dst_node: edge.dst_node,
                dst_port: edge.dst_port,
            });
            self.connected_input_ports
                .remove(&(edge.dst_node, edge.dst_port));

            self.needs_compile = true;

            true
        } else {
            false
        }
    }

    /// Get information about the given [Edge]
    pub fn edge(&self, edge_id: EdgeID) -> Option<&Edge> {
        self.edges.get(edge_id.0)
    }

    fn remove_edges_with_input_port(&mut self, node_id: NodeID, port_idx: PortIdx) -> Vec<EdgeID> {
        let mut edges_to_remove: Vec<EdgeID> = Vec::new();

        // Remove all existing edges which have this port.
        for (edge_id, edge) in self.edges.iter() {
            if edge.dst_node == node_id && edge.dst_port == port_idx {
                edges_to_remove.push(EdgeID(edge_id));
            }
        }

        for edge_id in edges_to_remove.iter() {
            self.disconnect_by_edge_id(*edge_id);
        }

        edges_to_remove
    }

    fn remove_edges_with_output_port(&mut self, node_id: NodeID, port_idx: PortIdx) -> Vec<EdgeID> {
        let mut edges_to_remove: Vec<EdgeID> = Vec::new();

        // Remove all existing edges which have this port.
        for (edge_id, edge) in self.edges.iter() {
            if edge.src_node == node_id && edge.src_port == port_idx {
                edges_to_remove.push(EdgeID(edge_id));
            }
        }

        for edge_id in edges_to_remove.iter() {
            self.disconnect_by_edge_id(*edge_id);
        }

        edges_to_remove
    }

    /// The current time of the clock in the number of seconds since the stream
    /// was started.
    pub fn clock_now(&self) -> ClockSeconds {
        ClockSeconds(
            (Instant::now()
                - self
                    .active_state
                    .as_ref()
                    .unwrap()
                    .main_thread_clock_start_instant)
                .as_secs_f64(),
        )
    }

    pub fn cycle_detected(&mut self) -> bool {
        compiler::cycle_detected::<NodeWeight>(
            &mut self.nodes,
            &mut self.edges,
            self.graph_in_id,
            self.graph_out_id,
        )
    }

    pub fn needs_compile(&self) -> bool {
        self.needs_compile
    }

    pub(crate) fn compile(
        &mut self,
        stream_info: StreamInfo,
    ) -> Result<ScheduleHeapData, CompileGraphError> {
        let schedule = self.compile_internal(stream_info.max_block_samples as usize)?;

        let new_node_processors = self.new_node_processors.drain(..).collect::<Vec<_>>();

        let schedule_data = ScheduleHeapData::new(
            schedule,
            self.nodes_to_remove_from_schedule.clone(),
            new_node_processors,
        );

        self.needs_compile = false;
        self.nodes_to_remove_from_schedule.clear();

        log::debug!("compiled new audio graph: {:?}", &schedule_data);

        Ok(schedule_data)
    }

    fn compile_internal(
        &mut self,
        max_block_samples: usize,
    ) -> Result<CompiledSchedule, CompileGraphError> {
        assert!(max_block_samples > 0);

        compiler::compile(
            &mut self.nodes,
            &mut self.edges,
            self.graph_in_id,
            self.graph_out_id,
            max_block_samples,
        )
    }

    pub(crate) fn flush_events(&mut self) -> Option<Vec<NodeEvent>> {
        if self.event_group.is_empty() {
            return None;
        }

        let mut next_event_group = self
            .event_group_pool
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(self.initial_event_group_capacity));
        std::mem::swap(&mut next_event_group, &mut self.event_group);
        Some(next_event_group)
    }

    pub(crate) fn return_event_group(&mut self, mut event_group: Vec<NodeEvent>) {
        event_group.clear();
        self.event_group_pool.push(event_group);
    }

    pub(crate) fn on_schedule_returned(&mut self, mut schedule_data: Box<ScheduleHeapData>) {
        for node_heap_data in schedule_data.removed_nodes.drain(..) {
            if let Some(mut node_entry) = self.active_nodes_to_remove.remove(&node_heap_data.id) {
                node_entry
                    .weight
                    .node
                    .deactivate(Some(node_heap_data.processor));
                node_entry.weight.activated = false;
            }
        }
    }

    pub(crate) fn on_processor_dropped(&mut self, mut nodes: Arena<crate::processor::NodeEntry>) {
        for (node_id, proc_node_entry) in nodes.drain() {
            if let Some(node_entry) = self.nodes.get_mut(node_id) {
                if node_entry.weight.activated {
                    node_entry
                        .weight
                        .node
                        .deactivate(Some(proc_node_entry.processor));
                    node_entry.weight.activated = false;
                }
            }
        }
    }

    pub(crate) fn activate(
        &mut self,
        stream_info: StreamInfo,
        main_thread_clock_start_instant: Instant,
    ) -> Result<(), NodeError> {
        let mut error = None;

        for (_, node_entry) in self.nodes.iter_mut() {
            assert!(!node_entry.weight.activated);

            match node_entry
                .weight
                .node
                .activate(&stream_info, node_entry.channel_config)
            {
                Ok(processor) => {
                    self.new_node_processors.push(NodeHeapData::new(
                        node_entry.id,
                        processor,
                        self.event_queue_capacity,
                        node_entry.weight.node.info().uses_events,
                    ));

                    node_entry.weight.activated = true;
                }
                Err(e) => {
                    error = Some(NodeError::ActivationFailed {
                        node_id: Some(node_entry.id),
                        error: e,
                    });
                    break;
                }
            }
        }

        if let Some(e) = error {
            self.deactivate();
            Err(e)
        } else {
            self.active_state = Some(ActiveState {
                stream_info,
                main_thread_clock_start_instant,
            });
            self.needs_compile = true;
            Ok(())
        }
    }

    pub(crate) fn deactivate(&mut self) {
        for (_, node_entry) in self.nodes.iter_mut() {
            if node_entry.weight.activated {
                let processor = self
                    .new_node_processors
                    .iter()
                    .enumerate()
                    .find_map(|(i, n)| if n.id == node_entry.id { Some(i) } else { None })
                    .map(|i| self.new_node_processors.remove(i).processor);

                node_entry.weight.node.deactivate(processor);
                node_entry.weight.activated = false;
            }
        }

        self.active_nodes_to_remove.clear();
        self.nodes_to_remove_from_schedule.clear();
        self.new_node_processors.clear();
        self.active_state = None;
    }

    pub(crate) fn update(&mut self) {
        for (_, node_entry) in self.nodes.iter_mut() {
            if node_entry.weight.updates {
                node_entry.weight.node.update();
            }
        }
    }
}
