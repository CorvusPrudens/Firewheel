use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount, NonZeroChannelCount},
    event::{NodeEventList, NodeEventType},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, EmptyConfig, ProcInfo,
        ProcessStatus, NUM_SCRATCH_BUFFERS,
    },
    sync_wrapper::SyncWrapper,
    SilenceMask, StreamInfo,
};
use fixed_resample::ReadStatus;
use ringbuf::traits::{Consumer, Producer, Split};

pub use fixed_resample::ResamplingChannelConfig;

#[cfg(feature = "resample_inputs")]
pub use fixed_resample::ResampleQuality;

use crate::{BUILD_STREAM_TIMEOUT, DEFAULT_MAX_BLOCK_FRAMES};

use super::StreamStartError;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct CpalInputNodeConfig {
    /// The configuration of the input to output channel.
    pub channel_config: ResamplingChannelConfig,

    /// If the input stream is running faster than the output stream by this
    /// amount in seconds, then discard samples to reduce the percieved
    /// glitchiness due to excessive overflows.
    ///
    /// This can happen if there are a lot of underruns occuring in the
    /// output audio thread.
    ///
    /// By default this is set to `0.15` (150ms).
    pub discard_jitter_threshold_seconds: f64,

    /// Whether or not to check for silence in the input stream. Highly
    /// recommened to set this to `true` to improve audio graph performance
    /// when there is no input on the microphone.
    ///
    /// By default this is set to `true`.
    pub check_for_silence: bool,
}

impl Default for CpalInputNodeConfig {
    fn default() -> Self {
        Self {
            channel_config: ResamplingChannelConfig::default(),
            discard_jitter_threshold_seconds: 0.15,
            check_for_silence: true,
        }
    }
}

/// The configuration of an input audio stream in the CPAL backend.
#[derive(Debug, Clone, PartialEq)]
pub struct CpalInputConfig {
    /// The host to use. Set to `None` to use the
    /// system's default audio host.
    pub host: Option<cpal::HostId>,

    /// The name of the input device to use. Set to `None` to use the
    /// system's default input device.
    ///
    /// By default this is set to `None`.
    pub device_name: Option<String>,

    /// The latency/block size of the audio stream to use.
    ///
    /// Smaller values may give better latency, but is not supported on
    /// all platforms and may lead to performance issues.
    ///
    /// By default this is set to `1024`, which is a latency of about 23
    /// milliseconds.
    pub desired_latency_frames: u32,

    /// Whether or not to fall back to the default device  if a device
    /// with the given configuration could not be found.
    ///
    /// By default this is set to `true`.
    pub fallback: bool,

    #[cfg(feature = "resample_inputs")]
    /// The desired sample rate to use. Set to `None` to use the device's
    /// default sample rate.
    ///
    /// By default this is set to `None`.
    pub desired_sample_rate: Option<u32>,
}

impl Default for CpalInputConfig {
    fn default() -> Self {
        Self {
            host: None,
            device_name: None,
            desired_latency_frames: DEFAULT_MAX_BLOCK_FRAMES,
            fallback: true,
            #[cfg(feature = "resample_inputs")]
            desired_sample_rate: None,
        }
    }
}

pub struct CpalInputNodeHandle {
    config: CpalInputNodeConfig,
    channels: NonZeroChannelCount,
    active_state: Option<ActiveHandleState>,
    shared_state: Arc<SharedState>,
}

impl CpalInputNodeHandle {
    pub fn new(config: CpalInputNodeConfig, channels: NonZeroChannelCount) -> Self {
        Self {
            config,
            channels,
            active_state: None,
            shared_state: Arc::new(SharedState::new()),
        }
    }

    pub fn constructor(&self) -> Constructor {
        Constructor {
            shared_state: Arc::clone(&self.shared_state),
            config: self.config,
            channels: self.channels,
        }
    }

    pub fn config(&self) -> &CpalInputNodeConfig {
        &self.config
    }

    pub fn channels(&self) -> NonZeroChannelCount {
        self.channels
    }

    pub fn is_active(&self) -> bool {
        self.active_state.is_some() && self.shared_state.stream_active.load(Ordering::Relaxed)
    }

    /// Returns `true` if an underflow occured (due to the output stream
    /// running faster than the input stream).
    ///
    /// If this happens in Release mode, you may want to consider increasing
    /// `[CpalInputNodeConfig::channel_config.latency_seconds`].
    ///
    /// (Calling this will also reset the flag indicating whether an
    /// underflow occurred.)
    pub fn underflow_occurred(&self) -> bool {
        self.shared_state
            .underflow_occurred
            .swap(false, Ordering::Relaxed)
    }

    /// Returns `true` if an overflow occured (due to the input stream
    /// running faster than the output stream).
    ///
    /// If this happens in Release mode, you may want to consider increasing
    /// `[CpalInputNodeConfig::channel_config.capacity_seconds`].
    ///
    /// (Calling this will also reset the flag indicating whether an
    /// overflow occurred.)
    pub fn overflow_occurred(&self) -> bool {
        self.shared_state
            .overflow_occurred
            .swap(false, Ordering::Relaxed)
    }

    /// Start a new input audio stream on this node.
    ///
    /// The returned event must be sent to the node's processor for this to take effect.
    ///
    /// * `config` - The configuration of the input stream.
    /// * `output_stream_sample_rate` - The sample rate of the active output audio stream.
    pub fn start_stream(
        &mut self,
        config: CpalInputConfig,
        output_stream_sample_rate: NonZeroU32,
    ) -> Result<(StreamInfo, NewInputStreamEvent), StreamStartError> {
        log::info!("Attempting to start input audio stream...");

        if self.is_active() {
            return Err(StreamStartError::InputStreamAlreadyActive);
        }

        self.shared_state.reset();

        let host = if let Some(host_id) = config.host {
            match cpal::host_from_id(host_id) {
                Ok(host) => host,
                Err(e) => {
                    log::warn!("Requested audio host {:?} is not available: {}. Falling back to default host...", &host_id, e);
                    cpal::default_host()
                }
            }
        } else {
            cpal::default_host()
        };

        let mut in_device = None;
        if let Some(device_name) = &config.device_name {
            match host.input_devices() {
                Ok(mut input_devices) => {
                    if let Some(d) = input_devices.find(|d| {
                        if let Ok(name) = d.name() {
                            &name == device_name
                        } else {
                            false
                        }
                    }) {
                        in_device = Some(d);
                    } else if config.fallback {
                        log::warn!("Could not find requested audio input device: {}. Falling back to default device...", &device_name);
                    } else {
                        return Err(StreamStartError::DeviceNotFound(device_name.clone()));
                    }
                }
                Err(e) => {
                    if config.fallback {
                        log::error!("Failed to get input audio devices: {}. Falling back to default device...", e);
                    } else {
                        return Err(e.into());
                    }
                }
            }
        }

        if in_device.is_none() {
            let Some(default_device) = host.default_input_device() else {
                return Err(StreamStartError::DefaultDeviceNotFound);
            };
            in_device = Some(default_device);
        }
        let in_device = in_device.unwrap();

        let in_device_name = in_device.name().unwrap_or_else(|e| {
            log::warn!("Failed to get name of input audio device: {}", e);
            String::from("unknown device")
        });

        let default_config = in_device.default_input_config()?;

        #[cfg(feature = "resample_inputs")]
        let mut desired_sample_rate = config
            .desired_sample_rate
            .unwrap_or(output_stream_sample_rate.get());
        #[cfg(not(feature = "resample_inputs"))]
        let mut desired_sample_rate = output_stream_sample_rate.get();

        let desired_latency_frames =
            if let &cpal::SupportedBufferSize::Range { min, max } = default_config.buffer_size() {
                Some(config.desired_latency_frames.clamp(min, max))
            } else {
                None
            };

        let supported_configs = in_device.supported_input_configs()?;

        let mut min_sample_rate = u32::MAX;
        let mut max_sample_rate = 0;
        for config in supported_configs.into_iter() {
            min_sample_rate = min_sample_rate.min(config.min_sample_rate().0);
            max_sample_rate = max_sample_rate.max(config.max_sample_rate().0);
        }
        desired_sample_rate = desired_sample_rate.clamp(min_sample_rate, max_sample_rate);

        #[cfg(not(feature = "resample_inputs"))]
        if desired_sample_rate != output_stream_sample_rate.get() {
            return Err(StreamStartError::CouldNotMatchSampleRate(
                output_stream_sample_rate.get(),
            ));
        }

        let num_in_channels = default_config.channels() as usize;
        assert_ne!(num_in_channels, 0);

        let desired_buffer_size = if let Some(samples) = desired_latency_frames {
            cpal::BufferSize::Fixed(samples)
        } else {
            cpal::BufferSize::Default
        };

        let stream_config = cpal::StreamConfig {
            channels: num_in_channels as u16,
            sample_rate: cpal::SampleRate(desired_sample_rate),
            buffer_size: desired_buffer_size,
        };

        let stream_latency_frames = if let cpal::BufferSize::Fixed(s) = stream_config.buffer_size {
            Some(s)
        } else {
            None
        };

        log::info!(
            "Starting input audio stream with device \"{}\" with configuration {:?}",
            &in_device_name,
            &config
        );

        let mut tmp_intl_buf = if num_in_channels != self.channels.get().get() as usize {
            (0..self.channels.get().get() as usize)
                .map(|_| {
                    let mut v = Vec::new();
                    v.reserve_exact(1024);
                    v.resize(1024, 0.0);
                    v
                })
                .collect()
        } else {
            Vec::new()
        };

        let (mut channel_tx, channel_rx) = fixed_resample::resampling_channel::<f32>(
            desired_sample_rate,
            output_stream_sample_rate.get(),
            self.channels.get().get() as usize,
            self.config.channel_config,
        );

        let (mut err_to_cx_tx, from_err_rx) = ringbuf::HeapRb::<cpal::StreamError>::new(4).split();

        let shared_state = Arc::clone(&self.shared_state);
        let shared_state_2 = Arc::clone(&self.shared_state);

        let stream_handle = in_device.build_input_stream(
            &stream_config,
            move |input: &[f32], _info: &cpal::InputCallbackInfo| {
                // Wait until the output stream has recieved the producer before
                // pushing more samples into the channel.
                if !shared_state.channel_started.load(Ordering::Relaxed) {
                    return;
                }

                let total_frames = input.len() / num_in_channels;

                if tmp_intl_buf.is_empty() {
                    let pushed_frames = channel_tx.push_interleaved(input);
                    if pushed_frames < total_frames {
                        shared_state
                            .overflow_occurred
                            .store(true, Ordering::Relaxed);
                    }
                } else {
                    let mut frames_processed = 0;
                    while frames_processed < total_frames {
                        let frames = (total_frames - frames_processed).min(1024);

                        fixed_resample::interleave::deinterleave(
                            input,
                            &mut tmp_intl_buf,
                            NonZeroUsize::new(num_in_channels).unwrap(),
                            0..frames,
                        );

                        let pushed_frames = channel_tx.push(&tmp_intl_buf, 0..frames);
                        if pushed_frames < frames {
                            shared_state
                                .overflow_occurred
                                .store(true, Ordering::Relaxed);
                        }

                        frames_processed += frames;
                    }
                }
            },
            move |err| {
                let _ = err_to_cx_tx.try_push(err);
                shared_state_2.stream_active.store(false, Ordering::Relaxed);
            },
            Some(BUILD_STREAM_TIMEOUT),
        )?;

        stream_handle.play()?;

        self.active_state = Some(ActiveHandleState {
            _stream_handle: stream_handle,
            from_err_rx,
        });
        self.shared_state
            .stream_active
            .store(true, Ordering::Relaxed);

        Ok((
            StreamInfo {
                sample_rate: NonZeroU32::new(desired_sample_rate).unwrap(),
                sample_rate_recip: (desired_sample_rate as f64).recip(),
                max_block_frames: NonZeroU32::new(
                    stream_latency_frames.unwrap_or(DEFAULT_MAX_BLOCK_FRAMES),
                )
                .unwrap(),
                num_stream_in_channels: num_in_channels as u32,
                num_stream_out_channels: 0,
                declick_frames: NonZeroU32::MIN,
                input_device_name: Some(in_device_name),
                output_device_name: None,
            },
            NewInputStreamEvent {
                channel_rx: Some(channel_rx),
            },
        ))
    }

    // Stop any active audio input streams.
    pub fn stop_stream(&mut self) {
        self.active_state = None;
        self.shared_state.reset();
    }

    // Poll the status of the active input stream. If an error is returned, then
    // it means that the input stream has been stopped.
    pub fn poll_status(&mut self) -> Result<(), cpal::StreamError> {
        if let Some(state) = &mut self.active_state {
            if let Some(e) = state.from_err_rx.try_pop() {
                self.active_state = None;

                return Err(e);
            }
        }

        Ok(())
    }
}

impl Drop for CpalInputNodeHandle {
    fn drop(&mut self) {
        self.stop_stream();
    }
}

#[derive(Clone)]
pub struct Constructor {
    shared_state: Arc<SharedState>,
    config: CpalInputNodeConfig,
    channels: NonZeroChannelCount,
}

impl AudioNodeConstructor for Constructor {
    type Configuration = EmptyConfig;

    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "cpal_input",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::ZERO,
                num_outputs: self.channels.get(),
            },
            uses_events: true,
        }
    }

    fn processor(
        &self,
        _config: &Self::Configuration,
        _stream_info: &StreamInfo,
    ) -> impl AudioNodeProcessor {
        Processor {
            channel_rx: None,
            shared_state: Arc::clone(&self.shared_state),
            discard_jitter_threshold_seconds: self.config.discard_jitter_threshold_seconds,
            check_for_silence: self.config.check_for_silence,
        }
    }
}

struct ActiveHandleState {
    _stream_handle: cpal::Stream,
    from_err_rx: ringbuf::HeapCons<cpal::StreamError>,
}

struct Processor {
    channel_rx: Option<fixed_resample::ResamplingCons<f32>>,
    shared_state: Arc<SharedState>,
    discard_jitter_threshold_seconds: f64,
    check_for_silence: bool,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        events.for_each(|event| {
            if let NodeEventType::Custom(event) = event {
                if let Some(in_stream_event) = event
                    .downcast_mut::<SyncWrapper<NewInputStreamEvent>>()
                    .and_then(SyncWrapper::get_mut)
                {
                    // Swap the memory so that the old channel will be properly
                    // dropped outside of the audio thread.
                    std::mem::swap(&mut self.channel_rx, &mut in_stream_event.channel_rx);
                }
            }
        });

        if !self.shared_state.stream_active.load(Ordering::Relaxed) {
            return ProcessStatus::ClearAllOutputs;
        }

        let Some(channel_rx) = &mut self.channel_rx else {
            return ProcessStatus::ClearAllOutputs;
        };

        // Notify the input stream that the output stream has begun
        // reading data.
        self.shared_state
            .channel_started
            .store(true, Ordering::Relaxed);

        let num_discarded_samples =
            channel_rx.discard_jitter(self.discard_jitter_threshold_seconds);
        if num_discarded_samples > 0 {
            self.shared_state
                .overflow_occurred
                .store(true, Ordering::Relaxed);
        }

        match channel_rx.read(outputs, 0..proc_info.frames) {
            ReadStatus::Ok => {}
            ReadStatus::Underflow => {
                self.shared_state
                    .underflow_occurred
                    .store(true, Ordering::Relaxed);
            }
            ReadStatus::WaitingForFrames => {
                return ProcessStatus::outputs_modified(SilenceMask::new_all_silent(outputs.len()));
            }
        }

        let mut silence_mask = SilenceMask::NONE_SILENT;
        if self.check_for_silence {
            let resampler_channels = channel_rx.num_channels().get();

            for (ch_i, ch) in outputs.iter().enumerate() {
                if ch_i >= resampler_channels {
                    // `channel_rx.read()` clears any extra channels
                    silence_mask.set_channel(ch_i, true);
                } else {
                    let mut all_silent = true;
                    for &s in ch[..proc_info.frames].iter() {
                        if s != 0.0 {
                            all_silent = false;
                            break;
                        }
                    }

                    if all_silent {
                        silence_mask.set_channel(ch_i, true);
                    }
                }
            }
        }

        ProcessStatus::outputs_modified(silence_mask)
    }

    fn stream_stopped(&mut self) {
        self.shared_state
            .stream_active
            .store(false, Ordering::Relaxed);
        self.channel_rx = None;
    }
}

pub struct NewInputStreamEvent {
    channel_rx: Option<fixed_resample::ResamplingCons<f32>>,
}

impl From<NewInputStreamEvent> for NodeEventType {
    fn from(value: NewInputStreamEvent) -> Self {
        NodeEventType::Custom(Box::new(SyncWrapper::new(value)))
    }
}

struct SharedState {
    stream_active: AtomicBool,
    channel_started: AtomicBool,
    underflow_occurred: AtomicBool,
    overflow_occurred: AtomicBool,
}

impl SharedState {
    fn new() -> Self {
        Self {
            stream_active: AtomicBool::new(false),
            channel_started: AtomicBool::new(false),
            underflow_occurred: AtomicBool::new(false),
            overflow_occurred: AtomicBool::new(false),
        }
    }

    fn reset(&self) {
        self.stream_active.store(false, Ordering::Relaxed);
        self.channel_started.store(false, Ordering::Relaxed);
        self.underflow_occurred.store(false, Ordering::Relaxed);
        self.overflow_occurred.store(false, Ordering::Relaxed);
    }
}
