use firewheel::{
    basic_nodes::{beep_test::BeepTestNode, StereoToMonoNode, SumNode, VolumeNode},
    clock::EventDelay,
    error::AddEdgeError,
    graph::AudioGraph,
    node::{AudioNode, NodeEvent, NodeID},
    ChannelConfig, FirewheelCpalCtx, UpdateStatus,
};

use crate::ui::GuiAudioNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    BeepTest,
    StereoToMono,
    SumMono4Ins,
    SumStereo2Ins,
    SumStereo4Ins,
    VolumeMono,
    VolumeStereo,
}

pub struct AudioSystem {
    cx: FirewheelCpalCtx,
}

impl AudioSystem {
    pub fn new() -> Self {
        let mut cx = FirewheelCpalCtx::new(Default::default());
        cx.activate(Default::default()).unwrap();

        Self { cx }
    }

    fn graph(&self) -> &AudioGraph {
        self.cx.graph()
    }

    fn graph_mut(&mut self) -> &mut AudioGraph {
        self.cx.graph_mut().unwrap()
    }

    pub fn remove_node(&mut self, node_id: NodeID) {
        if let Err(_) = self.graph_mut().remove_node(node_id) {
            log::error!("Node already removed!");
        }
    }

    pub fn add_node(&mut self, node_type: NodeType) -> GuiAudioNode {
        let (node, num_inputs, num_outputs): (Box<dyn AudioNode>, usize, usize) = match node_type {
            NodeType::BeepTest => (Box::new(BeepTestNode::new(0.4, 440.0, true)), 0, 1),
            NodeType::StereoToMono => (Box::new(StereoToMonoNode), 2, 1),
            NodeType::SumMono4Ins => (Box::new(SumNode), 4, 1),
            NodeType::SumStereo2Ins => (Box::new(SumNode), 4, 2),
            NodeType::SumStereo4Ins => (Box::new(SumNode), 8, 2),
            NodeType::VolumeMono => (Box::new(VolumeNode::new(1.0)), 1, 1),
            NodeType::VolumeStereo => (Box::new(VolumeNode::new(1.0)), 2, 2),
        };

        let id = self
            .graph_mut()
            .add_node(node, Some(ChannelConfig::new(num_inputs, num_outputs)))
            .unwrap();

        match node_type {
            NodeType::BeepTest => GuiAudioNode::BeepTest { id },
            NodeType::StereoToMono => GuiAudioNode::StereoToMono { id },
            NodeType::SumMono4Ins => GuiAudioNode::SumMono4Ins { id },
            NodeType::SumStereo2Ins => GuiAudioNode::SumStereo2Ins { id },
            NodeType::SumStereo4Ins => GuiAudioNode::SumStereo4Ins { id },
            NodeType::VolumeMono => GuiAudioNode::VolumeMono { id, percent: 100.0 },
            NodeType::VolumeStereo => GuiAudioNode::VolumeStereo { id, percent: 100.0 },
        }
    }

    pub fn connect(
        &mut self,
        src_node: NodeID,
        dst_node: NodeID,
        src_port: usize,
        dst_port: usize,
    ) -> Result<(), AddEdgeError> {
        self.graph_mut()
            .connect(src_node, src_port, dst_node, dst_port, true)?;

        Ok(())
    }

    pub fn disconnect(
        &mut self,
        src_node: NodeID,
        dst_node: NodeID,
        src_port: usize,
        dst_port: usize,
    ) {
        self.graph_mut()
            .disconnect(src_node, src_port, dst_node, dst_port);
    }

    pub fn graph_in_node(&self) -> NodeID {
        self.graph().graph_in_node()
    }

    pub fn graph_out_node(&self) -> NodeID {
        self.graph().graph_out_node()
    }

    pub fn is_activated(&self) -> bool {
        self.cx.is_activated()
    }

    pub fn update(&mut self) {
        match self.cx.update() {
            UpdateStatus::Inactive => {}
            UpdateStatus::Active { graph_error } => {
                if let Some(e) = graph_error {
                    log::error!("audio graph error: {}", e);
                }
            }
            UpdateStatus::Deactivated { error, .. } => {
                if let Some(e) = error {
                    log::error!("Stream disconnected: {}", e);
                } else {
                    log::error!("Stream disconnected");
                }
            }
        }

        self.cx.flush_events();
    }

    pub fn reset(&mut self) {
        self.graph_mut().reset();
    }

    pub fn set_volume(&mut self, node_id: NodeID, percent_volume: f32) {
        let graph = self.graph_mut();

        let event = graph
            .node_mut::<VolumeNode>(node_id)
            .unwrap()
            .set_volume(percent_volume / 100.0, false);

        graph.queue_event(NodeEvent {
            node_id,
            // Note, if you wanted to delay this event, use:
            // EventDelay::DelayUntilSeconds(graph.clock_now() + ClockSeconds(amount_of_delay))
            delay: EventDelay::Immediate,
            event,
        });
    }
}
