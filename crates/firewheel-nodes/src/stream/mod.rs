use std::num::NonZeroU32;

#[cfg(feature = "stream_reader")]
pub mod reader;
#[cfg(feature = "stream_writer")]
pub mod writer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveStreamNodeInfo {
    pub stream_sample_rate: NonZeroU32,
    pub latency_frames: usize,
    pub capacity_frames: usize,
}
