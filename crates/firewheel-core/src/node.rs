use downcast_rs::Downcast;
use std::{any::Any, error::Error};

use crate::{SilenceMask, StreamInfo};

pub trait AudioNode: 'static + Downcast {
    fn debug_name(&self) -> &'static str;

    fn info(&self) -> AudioNodeInfo;

    /// Activate the audio node for processing.
    fn activate(
        &mut self,
        stream_info: StreamInfo,
        num_inputs: usize,
        num_outputs: usize,
    ) -> Result<Box<dyn AudioNodeProcessor>, Box<dyn Error>>;

    /// Called when the processor counterpart has been deactivated
    /// and dropped.
    ///
    /// If the audio graph counterpart has gracefully shut down, then
    /// the processor counterpart is returned.
    #[allow(unused)]
    fn deactivate(&mut self, processor: Option<Box<dyn AudioNodeProcessor>>) {}

    /// A method that gets called periodically (i.e. once every frame).
    ///
    /// This method will only be called if [`AudioNodeInfo::updates`]
    /// was set to `true`.
    fn update(&mut self) {}
}

downcast_rs::impl_downcast!(AudioNode);

/// The status of processing buffers in an audio node.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// No output buffers were modified. If this is returned, then
    /// the engine will automatically clear all output buffers
    /// efficiently.
    #[default]
    NoOutputsModified,
    /// Output buffers were modified
    OutputsModified { out_silence_mask: SilenceMask },
}

impl ProcessStatus {
    /// All output buffers were filled with non-silence
    pub const fn all_outputs_filled() -> Self {
        Self::OutputsModified {
            out_silence_mask: SilenceMask::NONE_SILENT,
        }
    }

    /// Output buffers were modified
    pub const fn outputs_modified(out_silence_mask: SilenceMask) -> Self {
        Self::OutputsModified { out_silence_mask }
    }
}

pub trait AudioNodeProcessor: 'static + Send {
    /// Process the given block of audio. Only process data in the
    /// buffers up to `frames`.
    ///
    /// The node *MUST* either return `ProcessStatus::NoOutputsModified`
    /// or fill all output buffers with data.
    ///
    /// If any output buffers contain all zeros up to `frames` (silent),
    /// then mark that buffer as silent in [`ProcInfo::out_silence_mask`].
    fn process(
        &mut self,
        frames: usize,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo,
    ) -> ProcessStatus;
}

/// Additional information about an [`AudioNode`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioNodeInfo {
    /// The minimum number of input buffers this node supports
    pub num_min_supported_inputs: u32,
    /// The maximum number of input buffers this node supports
    ///
    /// This value must be less than `64`.
    pub num_max_supported_inputs: u32,

    /// The minimum number of output buffers this node supports
    pub num_min_supported_outputs: u32,
    /// The maximum number of output buffers this node supports
    ///
    /// This value must be less than `64`.
    pub num_max_supported_outputs: u32,

    /// Whether or not to call the `update` method on this node.
    ///
    /// If you do not need this, set this to `false` to save
    /// some performance overhead.
    ///
    /// By default this is set to `false`.
    pub updates: bool,
}

impl Default for AudioNodeInfo {
    fn default() -> Self {
        Self {
            num_min_supported_inputs: 0,
            num_max_supported_inputs: 0,
            num_min_supported_outputs: 0,
            num_max_supported_outputs: 0,
            updates: false,
        }
    }
}

/// Additional information for processing audio
pub struct ProcInfo<'a> {
    /// An optional optimization hint on which input channels contain
    /// all zeros (silence). The first bit (`0b1`) is the first channel,
    /// the second bit is the second channel, and so on.
    pub in_silence_mask: SilenceMask,

    /// An optional optimization hint on which output channels contain
    /// all zeros (silence). The first bit (`0b1`) is the first channel,
    /// the second bit is the second channel, and so on.
    pub out_silence_mask: SilenceMask,

    /// The number of seconds that have elapsed from when the stream was
    /// started to the fist sample in this process cycle. This uses the
    /// clock from the OS's audio API so it should be very accurate.
    pub stream_time_secs: f64,

    /// Flags indicating the current status of the audio stream
    pub stream_status: StreamStatus,

    /// A global user-defined context
    pub cx: &'a mut Box<dyn Any + Send>,
}

bitflags::bitflags! {
    /// Flags indicating the current status of the audio stream
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct StreamStatus: u32 {
        /// Some input data was discarded because of an overflow condition
        /// at the audio driver.
        const INPUT_OVERFLOW = 0b01;

        /// The output buffer ran low, likely producing a break in the
        /// output sound.
        const OUTPUT_UNDERFLOW = 0b10;
    }
}
