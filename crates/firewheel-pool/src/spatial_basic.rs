#[cfg(not(feature = "std"))]
use bevy_platform::prelude::{Vec, vec};

#[cfg(feature = "scheduled_events")]
use firewheel_core::clock::EventInstant;

use firewheel_core::{
    diff::Diff,
    node::{EmptyConfig, NodeID},
};
use firewheel_graph::FirewheelContext;
use firewheel_nodes::spatial_basic::SpatialBasicNode;

use crate::{FxChain, FxChainIo, ModifyNodePoolError};

/// A default [`FxChain`] for 3D game audio.
///
/// This chain contains a single [`SpatialBasicNode`]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct SpatialBasicChain {
    pub spatial_basic: SpatialBasicNode,
}

impl SpatialBasicChain {
    /// Set the parameters of the spatial basic node.
    ///
    /// * `params` - The new parameters.
    /// * `time` - The instant these new parameters should take effect. If this
    ///   is `None`, then the parameters will take effect as soon as the node receives
    ///   the event.
    pub fn set_params(
        &mut self,
        params: &SpatialBasicNode,
        #[cfg(feature = "scheduled_events")] time: Option<EventInstant>,
        node_ids: &[NodeID],
        cx: &mut FirewheelContext,
    ) {
        use firewheel_core::diff::PathBuilder;

        let node_id = node_ids[0];

        #[cfg(not(feature = "scheduled_events"))]
        let event_queue = &mut cx.event_queue(node_id);
        #[cfg(feature = "scheduled_events")]
        let event_queue = &mut cx.event_queue_scheduled(node_id, time);

        params.diff(&self.spatial_basic, PathBuilder::default(), event_queue);
        self.spatial_basic = *params;
    }
}

impl FxChain for SpatialBasicChain {
    type Configuration = EmptyConfig;

    fn construct_and_connect(
        &mut self,
        _configuration: &Self::Configuration,
        io: &FxChainIo,
        cx: &mut FirewheelContext,
    ) -> Result<Vec<NodeID>, ModifyNodePoolError> {
        let spatial_basic_params = firewheel_nodes::spatial_basic::SpatialBasicNode::default();
        let spatial_basic_node_id = cx.add_node(spatial_basic_params, None)?;

        cx.connect(
            io.first_node_id,
            spatial_basic_node_id,
            if io.first_node_out_channels.get().get() == 1 {
                &[(0, 0), (0, 1)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )?;

        cx.connect(
            spatial_basic_node_id,
            io.dst_node_id,
            if io.dst_node_in_channels.get().get() == 1 {
                &[(0, 0), (1, 0)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )?;

        Ok(vec![spatial_basic_node_id])
    }
}
