#[cfg(not(feature = "std"))]
use bevy_platform::prelude::{Vec, vec};

#[cfg(feature = "scheduled_events")]
use firewheel_core::clock::EventInstant;

use firewheel_core::{
    channel_config::NonZeroChannelCount,
    diff::{Diff, PathBuilder},
    node::{EmptyConfig, NodeID},
};
use firewheel_graph::FirewheelContext;
use firewheel_nodes::{volume::VolumeNodeConfig, volume_pan::VolumePanNode};

use crate::{FxChain, FxChainIo, ModifyNodePoolError};

/// A default [`FxChain`] for 2D game audio.
///
/// This chain contains a single [`VolumePanNode`].
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct VolumePanChain {
    pub volume_pan: VolumePanNode,
}

impl VolumePanChain {
    /// Set the parameters of the volume pan node.
    ///
    /// * `params` - The new parameters.
    /// * `time` - The instant these new parameters should take effect. If this
    ///   is `None`, then the parameters will take effect as soon as the node receives
    ///   the event.
    pub fn set_params(
        &mut self,
        params: &firewheel_nodes::volume_pan::VolumePanNode,
        #[cfg(feature = "scheduled_events")] time: Option<EventInstant>,
        node_ids: &[NodeID],
        cx: &mut FirewheelContext,
    ) {
        let node_id = node_ids[0];

        #[cfg(not(feature = "scheduled_events"))]
        let event_queue = &mut cx.event_queue(node_id);
        #[cfg(feature = "scheduled_events")]
        let event_queue = &mut cx.event_queue_scheduled(node_id, time);

        params.diff(&self.volume_pan, PathBuilder::default(), event_queue);
        self.volume_pan = *params;
    }
}

impl FxChain for VolumePanChain {
    type Configuration = EmptyConfig;

    fn construct_and_connect(
        &mut self,
        _configuration: &Self::Configuration,
        io: &FxChainIo,
        cx: &mut FirewheelContext,
    ) -> Result<Vec<NodeID>, ModifyNodePoolError> {
        let volume_pan_params = VolumePanNode::default();
        let volume_pan_node_id = cx.add_node(
            volume_pan_params,
            Some(VolumeNodeConfig {
                channels: NonZeroChannelCount::STEREO,
            }),
        )?;

        cx.connect(
            io.first_node_id,
            volume_pan_node_id,
            if io.first_node_out_channels.get().get() == 1 {
                &[(0, 0), (0, 1)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )?;

        cx.connect(
            volume_pan_node_id,
            io.dst_node_id,
            if io.dst_node_in_channels.get().get() == 1 {
                &[(0, 0), (1, 0)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )?;

        Ok(vec![volume_pan_node_id])
    }
}
