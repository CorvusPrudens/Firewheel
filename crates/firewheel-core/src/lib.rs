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
    pub max_block_samples: u32,
    pub num_stream_in_channels: u32,
    pub num_stream_out_channels: u32,
    pub stream_latency_samples: Option<u32>,
}

impl Default for StreamInfo {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            max_block_samples: 1024,
            stream_latency_samples: None,
            num_stream_in_channels: 0,
            num_stream_out_channels: 2,
        }
    }
}

/// A supported number of channels on an audio node.
///
/// This number cannot be greater than `64`.
#[repr(transparent)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelCount(u32);

impl ChannelCount {
    pub const ZERO: Self = Self(0);
    pub const MONO: Self = Self(1);
    pub const STEREO: Self = Self(2);
    pub const MAX: Self = Self(64);

    /// Create a new [`ChannelCount`].
    ///
    /// Returns `None` if `count` is greater than `64`.
    #[inline]
    pub const fn new(count: u32) -> Option<Self> {
        if count <= 64 {
            Some(Self(count))
        } else {
            None
        }
    }

    #[inline]
    pub const fn get(&self) -> u32 {
        if self.0 <= 64 {
            self.0
        } else {
            // SAFETY:
            // The constructor ensures that the value is less than or
            // equal to `64`.
            unsafe { std::hint::unreachable_unchecked() }
        }
    }
}

impl From<usize> for ChannelCount {
    fn from(value: usize) -> Self {
        Self::new(value as u32).unwrap()
    }
}

impl Into<u32> for ChannelCount {
    #[inline]
    fn into(self) -> u32 {
        self.get()
    }
}

impl Into<usize> for ChannelCount {
    #[inline]
    fn into(self) -> usize {
        self.get() as usize
    }
}

/// A supported number of channels on an audio node.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelConfig {
    pub num_inputs: ChannelCount,
    pub num_outputs: ChannelCount,
}

impl ChannelConfig {
    pub fn new(num_inputs: impl Into<ChannelCount>, num_outputs: impl Into<ChannelCount>) -> Self {
        Self {
            num_inputs: num_inputs.into(),
            num_outputs: num_outputs.into(),
        }
    }
}

impl From<(usize, usize)> for ChannelConfig {
    fn from(value: (usize, usize)) -> Self {
        Self::new(value.0, value.1)
    }
}
