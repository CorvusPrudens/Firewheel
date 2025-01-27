use firewheel::{
    channel_config::NonZeroChannelCount,
    error::{AddEdgeError, UpdateError},
    event::{NodeEvent, NodeEventType},
    node::NodeID,
    nodes::{
        beep_test::BeepTestParams, volume::VolumeParams, volume_pan::VolumePanParams,
        StereoToMonoNode,
    },
    FirewheelContext,
};

use crate::ui::GuiAudioNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    BeepTest,
    StereoToMono,
    VolumeMono,
    VolumeStereo,
    VolumePan,
}

pub struct AudioSystem {
    cx: FirewheelContext,
}

impl AudioSystem {
    pub fn new() -> Self {
        let mut cx = FirewheelContext::new(Default::default());
        cx.start_stream(Default::default()).unwrap();

        Self { cx }
    }

    pub fn remove_node(&mut self, node_id: NodeID) {
        if let Err(_) = self.cx.remove_node(node_id) {
            log::error!("Node already removed!");
        }
    }

    pub fn add_node(&mut self, node_type: NodeType) -> GuiAudioNode {
        let id = match node_type {
            NodeType::BeepTest => self.cx.add_node(BeepTestParams::default().constructor()),
            NodeType::StereoToMono => self.cx.add_node(StereoToMonoNode),
            NodeType::VolumeMono => self.cx.add_node(
                VolumeParams::default().constructor(NonZeroChannelCount::MONO, Default::default()),
            ),
            NodeType::VolumeStereo => self.cx.add_node(
                VolumeParams::default()
                    .constructor(NonZeroChannelCount::STEREO, Default::default()),
            ),
            NodeType::VolumePan => self
                .cx
                .add_node(VolumePanParams::default().constructor(Default::default())),
        };

        match node_type {
            NodeType::BeepTest => GuiAudioNode::BeepTest {
                id,
                params: BeepTestParams::default(),
            },
            NodeType::StereoToMono => GuiAudioNode::StereoToMono { id },
            NodeType::VolumeMono => GuiAudioNode::VolumeMono {
                id,
                params: VolumeParams::default(),
            },
            NodeType::VolumeStereo => GuiAudioNode::VolumeStereo {
                id,
                params: VolumeParams::default(),
            },
            NodeType::VolumePan => GuiAudioNode::VolumePan {
                id,
                params: VolumePanParams::default(),
            },
        }
    }

    pub fn connect(
        &mut self,
        src_node: NodeID,
        dst_node: NodeID,
        src_port: u32,
        dst_port: u32,
    ) -> Result<(), AddEdgeError> {
        self.cx
            .connect(src_node, dst_node, &[(src_port, dst_port)], true)?;

        Ok(())
    }

    pub fn disconnect(&mut self, src_node: NodeID, dst_node: NodeID, src_port: u32, dst_port: u32) {
        self.cx
            .disconnect(src_node, dst_node, &[(src_port, dst_port)]);
    }

    pub fn graph_in_node(&self) -> NodeID {
        self.cx.graph_in_node()
    }

    pub fn graph_out_node(&self) -> NodeID {
        self.cx.graph_out_node()
    }

    pub fn is_activated(&self) -> bool {
        self.cx.is_audio_stream_running()
    }

    pub fn update(&mut self) {
        if let Err(e) = self.cx.update() {
            log::error!("{:?}", &e);

            if let UpdateError::StreamStoppedUnexpectedly(_) = e {
                // The stream has stopped unexpectedly (i.e the user has
                // unplugged their headphones.)
                //
                // Typically you should start a new stream as soon as
                // possible to resume processing (event if it's a dummy
                // output device).
                //
                // In this example we just quit the application.
                panic!("Stream stopped unexpectedly.");
            }
        }
    }

    pub fn reset(&mut self) {
        let nodes: Vec<NodeID> = self.cx.nodes().map(|n| n.id).collect();
        for node_id in nodes {
            let _ = self.cx.remove_node(node_id);
        }
    }

    pub fn queue_event(&mut self, node_id: NodeID, event: NodeEventType) {
        self.cx.queue_event(NodeEvent { node_id, event });
    }
}
