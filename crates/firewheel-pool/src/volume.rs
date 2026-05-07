#[cfg(not(feature = "std"))]
use bevy_platform::prelude::{Vec, vec};

#[cfg(feature = "scheduled_events")]
use firewheel_core::clock::EventInstant;

use firewheel_core::{
    channel_config::NonZeroChannelCount,
    diff::{Diff, PathBuilder},
    node::{EmptyConfig, NodeError, NodeID},
};
use firewheel_graph::FirewheelContext;
use firewheel_nodes::volume::{VolumeNode, VolumeNodeConfig};

use crate::FxChain;

/// A default [`FxChain`] for with a single [`VolumeNode`].
///
/// This works with any number of channels.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct VolumeChain {
    pub volume_node: VolumeNode,
}

impl VolumeChain {
    /// Set the parameters of the volume pan node.
    ///
    /// * `params` - The new parameters.
    /// * `time` - The instant these new parameters should take effect. If this
    ///   is `None`, then the parameters will take effect as soon as the node receives
    ///   the event.
    pub fn set_params(
        &mut self,
        params: &VolumeNode,
        #[cfg(feature = "scheduled_events")] time: Option<EventInstant>,
        node_ids: &[NodeID],
        cx: &mut FirewheelContext,
    ) {
        let node_id = node_ids[0];

        #[cfg(not(feature = "scheduled_events"))]
        let event_queue = &mut cx.event_queue(node_id);
        #[cfg(feature = "scheduled_events")]
        let event_queue = &mut cx.event_queue_scheduled(node_id, time);

        params.diff(&self.volume_node, PathBuilder::default(), event_queue);
        self.volume_node = *params;
    }
}

impl FxChain for VolumeChain {
    type Configuration = EmptyConfig;

    fn construct_and_connect(
        &mut self,
        _configuration: &Self::Configuration,
        first_node_id: NodeID,
        first_node_num_out_channels: NonZeroChannelCount,
        dst_node_id: NodeID,
        _dst_num_channels: NonZeroChannelCount,
        cx: &mut FirewheelContext,
    ) -> Result<Vec<NodeID>, NodeError> {
        let volume_params = VolumeNode::default();
        let volume_node_id = cx.add_node(
            volume_params,
            Some(VolumeNodeConfig {
                channels: first_node_num_out_channels,
            }),
        )?;

        cx.auto_connect(first_node_id, volume_node_id, false)?;
        cx.auto_connect(volume_node_id, dst_node_id, false)?;

        Ok(vec![volume_node_id])
    }
}
