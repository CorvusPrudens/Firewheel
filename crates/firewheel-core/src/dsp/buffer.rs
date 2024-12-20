use std::num::NonZeroUsize;

use arrayvec::ArrayVec;

/// A memory-efficient buffer of samples with up to `MAX_CHANNELS` channels. Each
/// channel has a length of `frames`.
pub struct VarChannelBuffer<T: Clone + Copy + Default, const MAX_CHANNELS: usize> {
    buffer: Vec<T>,
    channels: NonZeroUsize,
    frames: NonZeroUsize,
}

impl<T: Clone + Copy + Default, const MAX_CHANNELS: usize> VarChannelBuffer<T, MAX_CHANNELS> {
    pub fn new(channels: NonZeroUsize, frames: NonZeroUsize) -> Self {
        assert!(channels.get() <= MAX_CHANNELS);

        let buffer_len = frames.get() * channels.get();

        let mut buffer = Vec::new();
        buffer.reserve_exact(buffer_len);
        buffer.resize(buffer_len, Default::default());

        Self {
            buffer,
            channels,
            frames,
        }
    }

    pub fn frames(&self) -> NonZeroUsize {
        self.frames
    }

    pub fn channels(&self) -> NonZeroUsize {
        self.channels
    }

    pub fn get(&self, channels: usize, frames: usize) -> ArrayVec<&[T], MAX_CHANNELS> {
        let frames = frames.min(self.frames.get());
        let channels = channels.min(self.channels.get());

        let mut res = ArrayVec::new();

        // SAFETY:
        //
        // * The constructor has set the size of the buffer to`self.frames * self.channels`,
        // and we have constrained `channels` and `frames` above, so this is always
        // within range.
        // * The constructor has ensured that `self.channels <= MAX_CHANNELS`.
        unsafe {
            for ch_i in 0..channels {
                res.push_unchecked(std::slice::from_raw_parts(
                    self.buffer.as_ptr().add(ch_i * self.frames.get()),
                    frames,
                ));
            }
        }

        res
    }

    pub fn get_mut(&mut self, channels: usize, frames: usize) -> ArrayVec<&mut [T], MAX_CHANNELS> {
        let frames = frames.min(self.frames.get());
        let channels = channels.min(self.channels.get());

        let mut res = ArrayVec::new();

        // SAFETY:
        //
        // * The constructor has set the size of the buffer to`self.frames * self.channels`,
        // and we have constrained `channels` and `frames` above, so this is always
        // within range.
        // * The constructor has ensured that `self.channels <= MAX_CHANNELS`.
        // * None of these slices overlap, and `self` is borrowed mutably in this method,
        // so all mutability rules are being upheld.
        unsafe {
            for ch_i in 0..channels {
                res.push_unchecked(std::slice::from_raw_parts_mut(
                    self.buffer.as_mut_ptr().add(ch_i * self.frames.get()),
                    frames,
                ));
            }
        }

        res
    }
}

/// A memory-efficient buffer of samples with variable number of instances each with up to
/// `MAX_CHANNELS` channels. Each channel has a length of `frames`.
pub struct InstanceBuffer<T: Clone + Copy + Default, const MAX_CHANNELS: usize> {
    buffer: Vec<T>,
    num_instances: usize,
    channels: NonZeroUsize,
    frames: NonZeroUsize,
}

impl<T: Clone + Copy + Default, const MAX_CHANNELS: usize> InstanceBuffer<T, MAX_CHANNELS> {
    pub fn new(num_instances: usize, channels: NonZeroUsize, frames: NonZeroUsize) -> Self {
        assert!(channels.get() <= MAX_CHANNELS);

        let buffer_len = frames.get() * channels.get() * num_instances;

        let mut buffer = Vec::new();
        buffer.reserve_exact(buffer_len);
        buffer.resize(buffer_len, Default::default());

        Self {
            buffer,
            num_instances,
            channels,
            frames,
        }
    }

    pub fn frames(&self) -> NonZeroUsize {
        self.frames
    }

    pub fn channels(&self) -> NonZeroUsize {
        self.channels
    }

    pub fn num_instances(&self) -> usize {
        self.num_instances
    }

    pub fn get(
        &self,
        instance_index: usize,
        channels: usize,
        frames: usize,
    ) -> Option<ArrayVec<&[T], MAX_CHANNELS>> {
        if instance_index >= self.num_instances {
            return None;
        }

        let frames = frames.min(self.frames.get());
        let channels = channels.min(self.channels.get());

        let start_frame = instance_index * self.frames.get() * self.channels.get();

        let mut res = ArrayVec::new();

        // SAFETY:
        //
        // * The constructor has set the size of the buffer to
        // `self.frames * self.channels * self.num_instances`, and we have constrained
        // `instance_index`, `channels` and `frames` above, so this is always within range.
        // * The constructor has ensured that `self.channels <= MAX_CHANNELS`.
        unsafe {
            for ch_i in 0..channels {
                res.push_unchecked(std::slice::from_raw_parts(
                    self.buffer
                        .as_ptr()
                        .add(start_frame + (ch_i * self.frames.get())),
                    frames,
                ));
            }
        }

        Some(res)
    }

    pub fn get_mut(
        &mut self,
        instance_index: usize,
        channels: usize,
        frames: usize,
    ) -> Option<ArrayVec<&mut [T], MAX_CHANNELS>> {
        if instance_index >= self.num_instances {
            return None;
        }

        let frames = frames.min(self.frames.get());
        let channels = channels.min(self.channels.get());

        let start_frame = instance_index * self.frames.get() * self.channels.get();

        let mut res = ArrayVec::new();

        // SAFETY:
        //
        // * The constructor has set the size of the buffer to
        // `self.frames * self.channels * self.num_instances`, and we have constrained
        // `instance_index`, `channels` and `frames` above, so this is always within range.
        // * The constructor has ensured that `self.channels <= MAX_CHANNELS`.
        // * None of these slices overlap, and `self` is borrowed mutably in this method,
        // so all mutability rules are being upheld.
        unsafe {
            for ch_i in 0..channels {
                res.push_unchecked(std::slice::from_raw_parts_mut(
                    self.buffer
                        .as_mut_ptr()
                        .add(start_frame + (ch_i * self.frames.get())),
                    frames,
                ));
            }
        }

        Some(res)
    }
}
