use core::any::Any;

use crate::{clock::EventDelay, diff::ParamPath, node::NodeID};

/// An event sent to an [`AudioNodeProcessor`].
pub struct NodeEvent {
    /// The ID of the node that should receive the event.
    pub node_id: NodeID,
    /// The type of event.
    pub event: NodeEventType,
}

pub enum ParamData {
    F32(f32),
    F64(f64),
    I32(i32),
    U32(u32),
    U64(u64),
    Bool(bool),
    Vector2D([f32; 2]),
    Vector3D([f32; 3]),
    Any(Box<Box<dyn Any + Send>>),
}

pub trait TryConvert<T> {
    fn try_convert(&self) -> Result<T, crate::diff::PatchError>;
}

macro_rules! param_data_from {
    ($ty:ty, $variant:ident) => {
        impl From<$ty> for ParamData {
            fn from(value: $ty) -> Self {
                Self::$variant(value)
            }
        }

        impl TryConvert<$ty> for ParamData {
            fn try_convert(&self) -> Result<$ty, crate::diff::PatchError> {
                match self {
                    ParamData::$variant(value) => Ok(*value),
                    _ => Err(crate::diff::PatchError::InvalidData),
                }
            }
        }
    };
}

param_data_from!(f32, F32);
param_data_from!(f64, F64);
param_data_from!(i32, I32);
param_data_from!(u32, U32);
param_data_from!(u64, U64);
param_data_from!(bool, Bool);

#[cfg(feature = "bevy")]
impl From<bevy_math::prelude::Vec2> for ParamData {
    fn from(value: bevy_math::prelude::Vec2) -> Self {
        Self::Vector2D([value.x, value.y])
    }
}

#[cfg(feature = "bevy")]
impl From<bevy_math::prelude::Vec3> for ParamData {
    fn from(value: bevy_math::prelude::Vec3) -> Self {
        Self::Vector3D([value.x, value.y, value.z])
    }
}

/// An event type associated with an [`AudioNodeProcessor`].
pub enum NodeEventType {
    Param {
        /// Data for a specific parameter.
        data: ParamData,
        /// The path to the parameter.
        path: ParamPath,
    },
    /// A command to control the current sequence in a node.
    ///
    /// This only has an effect on certain nodes.
    SequenceCommand(SequenceCommand),
    /// Custom event type.
    Custom(Box<dyn Any + Send>),
    /// Custom event type stored on the stack as raw bytes.
    CustomBytes([u8; 16]),
}

/// A command to control the current sequence in a node.
///
/// This only has an effect on certain nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SequenceCommand {
    /// Start/restart the current sequence.
    StartOrRestart {
        /// The exact moment when the sequence should start. Set to `None`
        /// to start as soon as the event is received.
        delay: Option<EventDelay>,
    },
    /// Pause the current sequence.
    Pause,
    /// Resume the current sequence.
    Resume,
    /// Stop the current sequence.
    Stop,
}

/// A list of events for an [`AudioNodeProcessor`].
pub struct NodeEventList<'a> {
    event_buffer: &'a mut [NodeEvent],
    indices: &'a [u32],
}

impl<'a> NodeEventList<'a> {
    pub fn new(event_buffer: &'a mut [NodeEvent], indices: &'a [u32]) -> Self {
        Self {
            event_buffer,
            indices,
        }
    }

    pub fn num_events(&self) -> usize {
        self.indices.len()
    }

    pub fn get_event(&mut self, index: usize) -> Option<&mut NodeEventType> {
        self.indices
            .get(index)
            .map(|idx| &mut self.event_buffer[*idx as usize].event)
    }

    pub fn for_each(&mut self, mut f: impl FnMut(&mut NodeEventType)) {
        for &idx in self.indices {
            if let Some(event) = self.event_buffer.get_mut(idx as usize) {
                (f)(&mut event.event);
            }
        }
    }
}
