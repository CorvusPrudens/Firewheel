/// An optional hint on which channels are connected to other
/// nodes in the graph. A bit set to `1` means that channel
/// is connected to another node, and a bit set to `0` means
/// that channel is not connected to any node.
///
/// The first bit (`0x1`) is the first channel, the second bit
/// is the second channel, and so on.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectedMask(pub u64);

impl ConnectedMask {
    pub const MONO_CONNECTED: Self = Self(0b1);
    pub const STEREO_CONNECTED: Self = Self(0b11);

    /// Returns `true` if the channel is connected to another node,
    /// `false` otherwise.
    ///
    /// `i` must be less than `64`.
    pub const fn is_channel_connected(&self, i: usize) -> bool {
        self.0 & (0b1 << i) != 0
    }

    /// Returns `true` if all channels are marked as connected, `false`
    /// otherwise.
    ///
    /// `num_channels` must be less than or equal to `64`.
    pub const fn all_channels_connected(&self, num_channels: usize) -> bool {
        if num_channels >= 64 {
            self.0 == u64::MAX
        } else {
            let mask = (0b1 << num_channels) - 1;
            self.0 & mask == mask
        }
    }

    /// Mark/un-mark the given channel as connected.
    ///
    /// `i` must be less than `64`.
    pub fn set_channel(&mut self, i: usize, connected: bool) {
        if connected {
            self.0 |= 0b1 << i;
        } else {
            self.0 &= !(0b1 << i);
        }
    }
}
