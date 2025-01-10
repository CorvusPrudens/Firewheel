use std::any::Any;

use crate::{clock::EventDelay, node::NodeID};

/// An event sent to an [`AudioNodeProcessor`].
pub struct NodeEvent {
    /// The ID of the node that should receive the event.
    pub node_id: NodeID,
    /// The type of event.
    pub event: NodeEventType,
}

/// An event type associated with an [`AudioNodeProcessor`].
pub enum NodeEventType {
    /// Set the value of an `f32` parameter.
    F32Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: f32,
    },
    /// Set the value of an `f64` parameter.
    F64Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: f64,
    },
    /// Set the value of an `i32` parameter.
    I32Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: i32,
    },
    /// Set the value of an `u32` parameter.
    U32Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: u32,
    },
    /// Set the value of an `u64` parameter.
    U64Param {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: u64,
    },
    /// Set the value of a `bool` parameter.
    BoolParam {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: bool,
    },
    /// Set the value of a parameter containing three
    /// `f32` elements.
    Vector3DParam {
        /// The unique ID of the paramater.
        id: u32,
        /// The parameter value.
        value: [f32; 3],
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
        /// The exact moment when the sequence should start.
        delay: EventDelay,
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
