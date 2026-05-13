use bevy_platform::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use triple_buf_64::{Input64, Output64, triple_buffer_64};

pub(super) fn shared_channel(
    shared_state: Arc<SharedState>,
) -> (SharedChannelMainThread, SharedChannelAudioThread) {
    let (sample_playhead_frames_tx, sample_playhead_frames_rx) = triple_buffer_64(0);
    let (finished_tx, finished_rx) = triple_buffer_64(0);

    shared_state.finished_cleared.store(true, Ordering::Relaxed);
    shared_state.set_playback_state(SharedPlaybackState::Stopped);

    (
        SharedChannelMainThread {
            sample_playhead_frames: sample_playhead_frames_rx,
            finished: finished_rx,
            shared_state: Arc::clone(&shared_state),
        },
        SharedChannelAudioThread {
            sample_playhead_frames: sample_playhead_frames_tx,
            finished: finished_tx,
            shared_state,
        },
    )
}

pub(super) struct SharedChannelMainThread {
    pub shared_state: Arc<SharedState>,
    sample_playhead_frames: Output64<u64>,
    finished: Output64<u64>,
}

impl SharedChannelMainThread {
    pub fn sample_playhead_frames(&mut self) -> u64 {
        self.sample_playhead_frames.read()
    }

    pub fn finished(&mut self) -> Option<u64> {
        if self.shared_state.finished_cleared.load(Ordering::Relaxed) {
            None
        } else {
            Some(self.finished.read())
        }
    }
}

pub(super) struct SharedChannelAudioThread {
    pub shared_state: Arc<SharedState>,
    sample_playhead_frames: Input64<u64>,
    finished: Input64<u64>,
}

impl SharedChannelAudioThread {
    pub fn set_sample_playhead_frames(&mut self, frames: u64) {
        self.sample_playhead_frames.write(frames);
    }

    pub fn set_finished(&mut self, finished: Option<u64>) {
        let (f, cleared) = finished.map(|f| (f, false)).unwrap_or((0, true));

        self.finished.write(f);
        self.shared_state
            .finished_cleared
            .store(cleared, Ordering::Relaxed);
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SharedPlaybackState {
    Stopped = 0,
    Paused,
    Playing,
}

impl SharedPlaybackState {
    pub fn from_u32(val: u32) -> Self {
        match val {
            1 => Self::Paused,
            2 => Self::Playing,
            _ => Self::Stopped,
        }
    }
}

pub(super) struct SharedState {
    has_sample_resource: AtomicBool,
    playback_state: AtomicU32,
    finished_cleared: AtomicBool,
}

impl SharedState {
    pub fn has_sample_resource(&self) -> bool {
        self.has_sample_resource.load(Ordering::Relaxed)
    }

    pub fn set_has_sample_resource(&self, val: bool) {
        self.has_sample_resource.store(val, Ordering::Relaxed)
    }

    pub fn playback_state(&self) -> SharedPlaybackState {
        SharedPlaybackState::from_u32(self.playback_state.load(Ordering::Relaxed))
    }

    pub fn set_playback_state(&self, playback_state: SharedPlaybackState) {
        self.playback_state
            .store(playback_state as u32, Ordering::Relaxed);
    }

    pub fn clear_finished(&self) {
        self.finished_cleared.store(true, Ordering::Relaxed);
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            has_sample_resource: AtomicBool::new(false),
            playback_state: AtomicU32::new(SharedPlaybackState::Stopped as u32),
            finished_cleared: AtomicBool::new(true),
        }
    }
}
