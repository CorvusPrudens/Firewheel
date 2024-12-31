use std::{fmt::Debug, num::NonZeroU32, time::Duration, u32};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use firewheel_core::{clock::ClockSeconds, node::StreamStatus, StreamInfo};
use firewheel_graph::{
    backend::DeviceInfo,
    error::CompileGraphError,
    processor::{FirewheelProcessor, FirewheelProcessorStatus},
    FirewheelConfig, FirewheelCtx, UpdateStatusInner,
};

/// 1024 samples is a latency of about 23 milliseconds, which should
/// be good enough for most games.
const DEFAULT_MAX_BLOCK_FRAMES: u32 = 1024;
const BUILD_STREAM_TIMEOUT: Duration = Duration::from_secs(5);
const MSG_CHANNEL_CAPACITY: usize = 4;

pub fn available_output_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::with_capacity(16);

    let host = cpal::default_host();

    let default_device_name = if let Some(default_device) = host.default_output_device() {
        match default_device.name() {
            Ok(n) => Some(n),
            Err(e) => {
                log::warn!("Failed to get name of default audio output device: {}", e);
                None
            }
        }
    } else {
        None
    };

    match host.output_devices() {
        Ok(output_devices) => {
            for device in output_devices {
                let Ok(name) = device.name() else {
                    continue;
                };

                let is_default = if let Some(default_device_name) = &default_device_name {
                    &name == default_device_name
                } else {
                    false
                };

                let default_out_config = match device.default_output_config() {
                    Ok(c) => c,
                    Err(e) => {
                        if is_default {
                            log::warn!("Failed to get default config for the default audio output device: {}", e);
                        }
                        continue;
                    }
                };

                devices.push(DeviceInfo {
                    name,
                    num_channels: default_out_config.channels(),
                    is_default,
                })
            }
        }
        Err(e) => {
            log::error!("Failed to get output audio devices: {}", e);
        }
    }

    devices
}

/// A firewheel context using CPAL as the audio backend.
///
/// The generic is a custom global processing context that is available to
/// node processors.
pub struct FirewheelCpalCtx {
    pub cx: FirewheelCtx,
    from_err_rx: rtrb::Consumer<cpal::StreamError>,
    out_device_name: String,
    stream_config: cpal::StreamConfig,
    _stream: cpal::Stream,
    _to_stream_tx: rtrb::Producer<CtxToStreamMsg>,
}

impl FirewheelCpalCtx {
    /// Create a new Firewheel context using CPAL as the backend.
    pub fn new(config: FirewheelConfig, cpal_config: CpalConfig) -> Result<Self, ActivateError> {
        let host = if let Some(host_id) = cpal_config.host {
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

        let is_alsa = cpal::HostId::name(&host.id()) == "ALSA";

        let mut device = None;
        if let Some(output_device_name) = &cpal_config.output_device_name {
            match host.output_devices() {
                Ok(mut output_devices) => {
                    if let Some(d) = output_devices.find(|d| {
                        if let Ok(name) = d.name() {
                            &name == output_device_name
                        } else {
                            false
                        }
                    }) {
                        device = Some(d);
                    } else if cpal_config.fallback {
                        log::warn!("Could not find requested audio output device: {}. Falling back to default device...", &output_device_name);
                    } else {
                        return Err(ActivateError::DeviceNotFound(output_device_name.clone()));
                    }
                }
                Err(e) => {
                    if cpal_config.fallback {
                        log::error!("Failed to get output audio devices: {}. Falling back to default device...", e);
                    } else {
                        return Err(e.into());
                    }
                }
            }
        }

        if device.is_none() {
            let Some(default_device) = host.default_output_device() else {
                if cpal_config.fallback {
                    log::error!("No default audio output device found. Falling back to dummy output device...");
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err(ActivateError::DefaultDeviceNotFound);
                }
            };
            device = Some(default_device);
        }
        let device = device.unwrap();

        let default_cpal_config = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                if cpal_config.fallback {
                    log::error!(
                        "Failed to get default config for output audio device: {}. Falling back to dummy output device...",
                        e
                    );
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err(e.into());
                }
            }
        };

        let mut desired_sample_rate = cpal_config
            .desired_sample_rate
            .unwrap_or(default_cpal_config.sample_rate().0);
        let desired_latency_frames = if let &cpal::SupportedBufferSize::Range { min, max } =
            default_cpal_config.buffer_size()
        {
            Some(cpal_config.desired_latency_frames.clamp(min, max))
        } else {
            None
        };

        let supported_cpal_configs = match device.supported_output_configs() {
            Ok(c) => c,
            Err(e) => {
                if cpal_config.fallback {
                    log::error!(
                        "Failed to get configs for output audio device: {}. Falling back to dummy output device...",
                        e
                    );
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err(e.into());
                }
            }
        };

        let mut min_sample_rate = u32::MAX;
        let mut max_sample_rate = 0;
        for config in supported_cpal_configs.into_iter() {
            min_sample_rate = min_sample_rate.min(config.min_sample_rate().0);
            max_sample_rate = max_sample_rate.max(config.max_sample_rate().0);
        }
        desired_sample_rate = desired_sample_rate.clamp(min_sample_rate, max_sample_rate);

        let num_in_channels = 0;
        let num_out_channels = default_cpal_config.channels() as usize;
        assert_ne!(num_out_channels, 0);

        let desired_buffer_size = if let Some(samples) = desired_latency_frames {
            cpal::BufferSize::Fixed(samples)
        } else {
            cpal::BufferSize::Default
        };

        let stream_config = cpal::StreamConfig {
            channels: num_out_channels as u16,
            sample_rate: cpal::SampleRate(desired_sample_rate),
            buffer_size: desired_buffer_size,
        };

        let out_device_name = device.name().unwrap_or_else(|_| "unkown".into());

        log::info!(
            "Starting output audio stream with device \"{}\" with configuration {:?}",
            &out_device_name,
            &cpal_config
        );

        let max_block_frames = match stream_config.buffer_size {
            cpal::BufferSize::Default => DEFAULT_MAX_BLOCK_FRAMES as usize,
            cpal::BufferSize::Fixed(f) => f as usize,
        };

        let stream_latency_frames = if let cpal::BufferSize::Fixed(s) = stream_config.buffer_size {
            Some(s)
        } else {
            None
        };

        let (mut to_stream_tx, from_cx_rx) =
            rtrb::RingBuffer::<CtxToStreamMsg>::new(MSG_CHANNEL_CAPACITY);
        let (mut err_to_cx_tx, from_err_rx) =
            rtrb::RingBuffer::<cpal::StreamError>::new(MSG_CHANNEL_CAPACITY);

        let mut data_callback = DataCallback::new(
            num_in_channels,
            num_out_channels,
            from_cx_rx,
            stream_config.sample_rate.0,
            is_alsa,
        );

        let mut is_first_callback = true;
        let stream = match device.build_output_stream(
            &stream_config,
            move |output: &mut [f32], info: &cpal::OutputCallbackInfo| {
                if is_first_callback {
                    // Apparently there is a bug in CPAL where the callback instant in
                    // the first callback can be greater than in the second callback.
                    //
                    // Work around this by ignoring the first block of samples.
                    is_first_callback = false;
                    output.fill(0.0);
                } else {
                    data_callback.callback(output, info);
                }
            },
            move |err| {
                let _ = err_to_cx_tx.push(err);
            },
            Some(BUILD_STREAM_TIMEOUT),
        ) {
            Ok(s) => s,
            Err(e) => {
                if cpal_config.fallback {
                    log::error!("Failed to start output audio stream: {}. Falling back to dummy output device...", e);
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err(e.into());
                }
            }
        };

        stream.play()?;

        let (cx, processor) = FirewheelCtx::new(
            config,
            StreamInfo {
                sample_rate: NonZeroU32::new(stream_config.sample_rate.0).unwrap(),
                max_block_frames: NonZeroU32::new(max_block_frames as u32).unwrap(),
                num_stream_in_channels: num_in_channels as u32,
                num_stream_out_channels: num_out_channels as u32,
                stream_latency_frames,
                // The engine will overwrite the other values.
                ..Default::default()
            },
        );

        to_stream_tx
            .push(CtxToStreamMsg::NewProcessor(processor))
            .unwrap();

        Ok(Self {
            cx,
            from_err_rx,
            out_device_name,
            stream_config,
            _stream: stream,
            _to_stream_tx: to_stream_tx,
        })
    }

    /// Get the name of the audio output device.
    pub fn out_device_name(&self) -> &str {
        self.out_device_name.as_str()
    }

    /// Get information about the current audio stream.
    pub fn stream_info(&self) -> &StreamInfo {
        self.cx.stream_info()
    }

    /// Get the current configuration of the audio stream.
    pub fn stream_config(&self) -> &cpal::StreamConfig {
        &self.stream_config
    }

    /// Update the firewheel context.
    ///
    /// This must be called reguarly (i.e. once every frame).
    #[must_use]
    pub fn update(self) -> UpdateStatus {
        let FirewheelCpalCtx {
            mut cx,
            mut from_err_rx,
            out_device_name,
            stream_config,
            _stream,
            _to_stream_tx,
        } = self;

        if let Ok(e) = from_err_rx.pop() {
            cx._notify_stream_crashed();

            return UpdateStatus::Deactivated { error: Some(e) };
        }

        match cx._update() {
            UpdateStatusInner::Ok {
                cx,
                graph_compile_error,
            } => UpdateStatus::Ok {
                cx: Self {
                    cx,
                    from_err_rx,
                    out_device_name,
                    stream_config,
                    _stream,
                    _to_stream_tx,
                },
                graph_compile_error,
            },
            UpdateStatusInner::Deactivated { .. } => UpdateStatus::Deactivated { error: None },
        }
    }
}

// Implement Debug so `unwrap()` can be used.
impl Debug for FirewheelCpalCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FirewheelCpalCtx")
    }
}

pub enum UpdateStatus {
    Ok {
        cx: FirewheelCpalCtx,
        graph_compile_error: Option<CompileGraphError>,
    },
    /// The engine was deactivated.
    ///
    /// If this is returned, then all node handles are invalidated.
    /// The graph and all its nodes must be reconstructed.
    Deactivated { error: Option<cpal::StreamError> },
}

/// The configuration of an audio stream
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpalConfig {
    /// The host to use. Set to `None` to use the
    /// system's default output device.
    pub host: Option<cpal::HostId>,

    /// The name of the output device to use. Set to `None` to use the
    /// system's default output device.
    ///
    /// By default this is set to `None`.
    pub output_device_name: Option<String>,

    /// The desired sample rate to use. Set to `None` to use the device's
    /// default sample rate.
    ///
    /// By default this is set to `None`.
    pub desired_sample_rate: Option<u32>,

    /// The latency of the audio stream to use.
    ///
    /// Smaller values may give better latency, but is not supported on
    /// all platforms and may lead to performance issues.
    ///
    /// By default this is set to `1024`, which is a latency of about 23
    /// milliseconds. This should be good enough for most games. (Rhythm
    /// games may want to try a lower latency).
    pub desired_latency_frames: u32,

    /// Whether or not to fall back to the default device and then a
    /// dummy output device if a device with the given configuration
    /// could not be found.
    ///
    /// By default this is set to `true`.
    pub fallback: bool,
}

impl Default for CpalConfig {
    fn default() -> Self {
        Self {
            host: None,
            output_device_name: None,
            desired_sample_rate: None,
            desired_latency_frames: DEFAULT_MAX_BLOCK_FRAMES,
            fallback: true,
        }
    }
}

struct DataCallback {
    num_in_channels: usize,
    num_out_channels: usize,
    from_cx_rx: rtrb::Consumer<CtxToStreamMsg>,
    processor: Option<FirewheelProcessor>,
    sample_rate_recip: f64,
    first_internal_clock_instant: Option<cpal::StreamInstant>,
    prev_stream_instant: Option<cpal::StreamInstant>,
    first_fallback_clock_instant: Option<std::time::Instant>,
    predicted_stream_secs: Option<f64>,
    is_alsa: bool,
}

impl DataCallback {
    fn new(
        num_in_channels: usize,
        num_out_channels: usize,
        from_cx_rx: rtrb::Consumer<CtxToStreamMsg>,
        sample_rate: u32,
        is_alsa: bool,
    ) -> Self {
        Self {
            num_in_channels,
            num_out_channels,
            from_cx_rx,
            processor: None,
            sample_rate_recip: f64::from(sample_rate).recip(),
            first_internal_clock_instant: None,
            prev_stream_instant: None,
            predicted_stream_secs: None,
            first_fallback_clock_instant: None,
            is_alsa,
        }
    }

    fn callback(&mut self, output: &mut [f32], info: &cpal::OutputCallbackInfo) {
        while let Ok(msg) = self.from_cx_rx.pop() {
            let CtxToStreamMsg::NewProcessor(p) = msg;
            self.processor = Some(p);
        }

        let samples = output.len() / self.num_out_channels;

        let (internal_clock_secs, underflow) = if self.is_alsa {
            if let Some(instant) = self.first_fallback_clock_instant {
                let now = std::time::Instant::now();

                let internal_clock_secs = (now - instant).as_secs_f64();

                let underflow = if let Some(predicted_stream_secs) = self.predicted_stream_secs {
                    // If the stream time is significantly greater than the predicted stream
                    // time, it means an output underflow has occurred.
                    internal_clock_secs > predicted_stream_secs
                } else {
                    false
                };

                // Calculate the next predicted stream time to detect underflows.
                //
                // Add a little bit of wiggle room to account for tiny clock
                // innacuracies and rounding errors.
                self.predicted_stream_secs =
                    Some(internal_clock_secs + (samples as f64 * self.sample_rate_recip * 1.2));

                (ClockSeconds(internal_clock_secs), underflow)
            } else {
                self.first_fallback_clock_instant = Some(std::time::Instant::now());
                (ClockSeconds(0.0), false)
            }
        } else {
            if let Some(instant) = &self.first_internal_clock_instant {
                if let Some(prev_stream_instant) = &self.prev_stream_instant {
                    if info
                        .timestamp()
                        .playback
                        .duration_since(prev_stream_instant)
                        .is_none()
                    {
                        // When I tested this under ALSA, sometimes underruns caused this condition
                        // to happen, so as a workaround I'm using the clock in std::time instead
                        // when using ALSA.
                        //
                        // If this occurs in other APIs as well, then either I'm doing something
                        // wrong or CPAL is doing something wrong.
                        log::error!("CPAL and/or the system audio API returned invalid timestamp. Please notify the Firewheel developers of this bug.");
                    }
                }

                let internal_clock_secs = info
                    .timestamp()
                    .playback
                    .duration_since(instant)
                    .map(|s| s.as_secs_f64())
                    .unwrap_or_else(|| self.predicted_stream_secs.unwrap_or(0.0));

                let underflow = if let Some(predicted_stream_secs) = self.predicted_stream_secs {
                    // If the stream time is significantly greater than the predicted stream
                    // time, it means an output underflow has occurred.
                    internal_clock_secs > predicted_stream_secs
                } else {
                    false
                };

                // Calculate the next predicted stream time to detect underflows.
                //
                // Add a little bit of wiggle room to account for tiny clock
                // innacuracies and rounding errors.
                self.predicted_stream_secs =
                    Some(internal_clock_secs + (samples as f64 * self.sample_rate_recip * 1.2));

                self.prev_stream_instant = Some(info.timestamp().playback);

                (ClockSeconds(internal_clock_secs), underflow)
            } else {
                self.first_internal_clock_instant = Some(info.timestamp().playback);
                (ClockSeconds(0.0), false)
            }
        };

        let mut drop_processor = false;
        if let Some(processor) = &mut self.processor {
            let mut stream_status = StreamStatus::empty();

            if underflow {
                stream_status.insert(StreamStatus::OUTPUT_UNDERFLOW);
            }

            match processor.process_interleaved(
                &[],
                output,
                self.num_in_channels,
                self.num_out_channels,
                samples,
                internal_clock_secs,
                stream_status,
            ) {
                FirewheelProcessorStatus::Ok => {}
                FirewheelProcessorStatus::DropProcessor => drop_processor = true,
            }
        } else {
            output.fill(0.0);
            return;
        }

        if drop_processor {
            self.processor = None;
        }
    }
}

enum CtxToStreamMsg {
    NewProcessor(FirewheelProcessor),
}

/// An error occured while trying to activate an [`InactiveFwCpalcx`]
#[derive(Debug, thiserror::Error)]
pub enum ActivateError {
    #[error("The requested audio device was not found: {0}")]
    DeviceNotFound(String),
    #[error("Could not get audio devices: {0}")]
    FailedToGetDevices(#[from] cpal::DevicesError),
    #[error("Failed to get default audio output device")]
    DefaultDeviceNotFound,
    #[error("Failed to get audio device configs: {0}")]
    FailedToGetConfigs(#[from] cpal::SupportedStreamConfigsError),
    #[error("Failed to get audio device config: {0}")]
    FailedToGetConfig(#[from] cpal::DefaultStreamConfigError),
    #[error("Failed to build audio stream: {0}")]
    BuildStreamError(#[from] cpal::BuildStreamError),
    #[error("Failed to play audio stream: {0}")]
    PlayStreamError(#[from] cpal::PlayStreamError),
}
