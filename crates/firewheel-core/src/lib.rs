pub mod clock;
pub mod node;
pub mod param;
pub mod sample_resource;
mod silence_mask;
pub mod util;

pub use silence_mask::SilenceMask;

/// Information about a running audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamInfo {
    pub sample_rate: u32,
    pub max_block_frames: u32,
    pub stream_latency_frames: u32,
    pub num_stream_in_channels: u32,
    pub num_stream_out_channels: u32,
}
