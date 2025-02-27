use std::sync::atomic::{AtomicBool, Ordering};

use atomic_float::AtomicF32;
use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    collector::ArcGc,
    dsp::decibel::{gain_to_db_clamped_neg_100_db, DbMeterNormalizer},
    event::NodeEventList,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, EmptyConfig, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    StreamInfo,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeakMeterSmootherConfig {
    /// The rate of decay in seconds.
    ///
    /// By default this is set to `0.3` (300ms).
    pub decay_rate: f32,
    /// The rate at which this meter will refresh. This will typically
    /// match the display's frame rate.
    ///
    /// By default this is set to `60.0`.
    pub refresh_rate: f32,
    /// The number of frames that the values in `has_clipped` will
    /// hold their values before resetting to `false`.
    ///
    /// By default this is set to `60`.
    pub clip_hold_frames: usize,
}

impl Default for PeakMeterSmootherConfig {
    fn default() -> Self {
        Self {
            decay_rate: 0.3,
            refresh_rate: 60.0,
            clip_hold_frames: 60,
        }
    }
}

/// A helper struct to smooth out the output of [`PeakMeterNode`]. This
/// can be used to drive the animation of a peak meter in a GUI.
#[derive(Debug, Clone, Copy)]
pub struct PeakMeterSmoother<const NUM_CHANNELS: usize> {
    /// The current smoothed peak value of each channel, in decibels.
    smoothed_peaks: [f32; NUM_CHANNELS],
    clipped_frames_left: [usize; NUM_CHANNELS],
    level_decay: f32,
    frame_interval: f32,
    frame_counter: f32,
    clip_hold_frames: usize,
}

impl<const NUM_CHANNELS: usize> PeakMeterSmoother<NUM_CHANNELS> {
    pub fn new(config: PeakMeterSmootherConfig) -> Self {
        assert!(config.decay_rate > 0.0);
        assert!(config.refresh_rate > 0.0);
        assert!(config.clip_hold_frames > 0);

        Self {
            smoothed_peaks: [-100.0; NUM_CHANNELS],
            clipped_frames_left: [0; NUM_CHANNELS],
            level_decay: 1.0 - (-1.0 / (config.refresh_rate * config.decay_rate)).exp(),
            frame_interval: config.refresh_rate.recip(),
            frame_counter: 0.0,
            clip_hold_frames: config.clip_hold_frames,
        }
    }

    pub fn reset(&mut self) {
        self.smoothed_peaks = [-100.0; NUM_CHANNELS];
        self.clipped_frames_left = [0; NUM_CHANNELS];
    }

    pub fn update(&mut self, peaks_db: [f32; NUM_CHANNELS], delta_seconds: f32) {
        for ((smoothed_peak, &in_peak), clipped_frames_left) in self
            .smoothed_peaks
            .iter_mut()
            .zip(peaks_db.iter())
            .zip(self.clipped_frames_left.iter_mut())
        {
            if in_peak >= *smoothed_peak {
                *smoothed_peak = in_peak;

                if in_peak > 0.0 {
                    *clipped_frames_left = self.clip_hold_frames;
                }
            }
        }

        self.frame_counter += delta_seconds;

        // Correct for cumulative errors.
        if (self.frame_counter - self.frame_interval).abs() < 0.0001 {
            self.frame_counter = self.frame_interval;
        }

        while self.frame_counter >= self.frame_interval {
            self.frame_counter -= self.frame_interval;

            // Correct for cumulative errors.
            if (self.frame_counter - self.frame_interval).abs() < 0.0001 {
                self.frame_counter = self.frame_interval;
            }

            for ((smoothed_peak, &in_peak), clipped_frames_left) in self
                .smoothed_peaks
                .iter_mut()
                .zip(peaks_db.iter())
                .zip(self.clipped_frames_left.iter_mut())
            {
                if in_peak + 0.001 < *smoothed_peak {
                    *smoothed_peak += ((in_peak - *smoothed_peak) * self.level_decay).max(-100.0);
                }

                if *smoothed_peak > 0.0 {
                    *clipped_frames_left = self.clip_hold_frames;
                } else if *clipped_frames_left > 0 {
                    *clipped_frames_left -= 1;
                }
            }
        }
    }

    pub fn has_clipped(&self) -> [bool; NUM_CHANNELS] {
        std::array::from_fn(|i| self.clipped_frames_left[i] > 0)
    }

    pub fn smoothed_peaks_db(&self) -> &[f32; NUM_CHANNELS] {
        &self.smoothed_peaks
    }

    pub fn smoothed_peak_db_mono(&self) -> f32 {
        let mut max_value: f32 = -100.0;
        for ch in self.smoothed_peaks {
            max_value = max_value.max(ch);
        }

        max_value
    }

    /// Get the peak values as a normalized value in the range `[0.0, 1.0]`.
    pub fn smoothed_peaks_normalized(&self, normalizer: &DbMeterNormalizer) -> [f32; NUM_CHANNELS] {
        std::array::from_fn(|i| normalizer.normalize(self.smoothed_peaks[i]))
    }

    pub fn smoothed_peaks_normalized_mono(&self, normalizer: &DbMeterNormalizer) -> f32 {
        normalizer.normalize(self.smoothed_peak_db_mono())
    }
}

#[derive(Clone)]
pub struct PeakMeterNode<const NUM_CHANNELS: usize> {
    shared_state: ArcGc<SharedState<NUM_CHANNELS>>,
}

impl<const NUM_CHANNELS: usize> PeakMeterNode<NUM_CHANNELS> {
    /// Create a new [`PeakMeterNode`].
    ///
    /// # Panics
    ///
    /// Panics if `NUM_CHANNELS == 0` or `NUM_CHANNELS > 64`.
    pub fn new(enabled: bool) -> Self {
        assert_ne!(NUM_CHANNELS, 0);
        assert!(NUM_CHANNELS <= 64);

        Self {
            shared_state: ArcGc::new(SharedState {
                peak_gains: std::array::from_fn(|_| AtomicF32::new(0.0)),
                enabled: AtomicBool::new(enabled),
            }),
        }
    }

    /// Get the latest peak values for each channel in decibels.
    ///
    /// If the node is currently disabled, then this will return a value
    /// of -100.0 dB (silence) for all channels.
    pub fn peak_gain_db(&self) -> [f32; NUM_CHANNELS] {
        std::array::from_fn(|i| {
            gain_to_db_clamped_neg_100_db(self.shared_state.peak_gains[i].load(Ordering::Relaxed))
        })
    }

    /// Whether or not the node is currently enabled.
    pub fn enabled(&self) -> bool {
        self.shared_state.enabled.load(Ordering::Relaxed)
    }

    /// Enable/disable this node.
    ///
    /// It is a good idea to disable this node when not in use to save
    /// on CPU.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.shared_state.enabled.store(enabled, Ordering::Relaxed);
    }
}

impl<const NUM_CHANNELS: usize> AudioNode for PeakMeterNode<NUM_CHANNELS> {
    type Configuration = EmptyConfig;

    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo::new()
            .debug_name("peak_meter")
            .channel_config(ChannelConfig {
                num_inputs: ChannelCount::new(NUM_CHANNELS as u32).unwrap(),
                num_outputs: ChannelCount::new(NUM_CHANNELS as u32).unwrap(),
            })
            .uses_events(false)
    }

    fn processor(
        &self,
        _config: &Self::Configuration,
        _stream_info: &StreamInfo,
    ) -> impl AudioNodeProcessor {
        Processor {
            shared_state: ArcGc::clone(&self.shared_state),
            enabled: self.shared_state.enabled.load(Ordering::Relaxed),
        }
    }
}

struct SharedState<const NUM_CHANNELS: usize> {
    peak_gains: [AtomicF32; NUM_CHANNELS],
    enabled: AtomicBool,
}

struct Processor<const NUM_CHANNELS: usize> {
    shared_state: ArcGc<SharedState<NUM_CHANNELS>>,
    enabled: bool,
}

impl<const NUM_CHANNELS: usize> AudioNodeProcessor for Processor<NUM_CHANNELS> {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        let enabled = self.shared_state.enabled.load(Ordering::Relaxed);

        if self.enabled && !enabled {
            for ch in self.shared_state.peak_gains.iter() {
                ch.store(0.0, Ordering::Relaxed);
            }
        }
        self.enabled = enabled;

        if !self.enabled {
            return ProcessStatus::Bypass;
        }

        for (i, (in_ch, peak_shared)) in inputs
            .iter()
            .zip(self.shared_state.peak_gains.iter())
            .enumerate()
        {
            if proc_info.in_silence_mask.is_channel_silent(i) {
                peak_shared.store(0.0, Ordering::Relaxed);
            } else {
                let mut max_peak: f32 = 0.0;
                for &s in in_ch.iter() {
                    max_peak = max_peak.max(s.abs());
                }

                peak_shared.store(max_peak, Ordering::Relaxed);
            }
        }

        ProcessStatus::Bypass
    }
}
