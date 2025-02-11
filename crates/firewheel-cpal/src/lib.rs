use std::{fmt::Debug, num::NonZeroU32, time::Duration, u32};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use firewheel_core::{clock::ClockSeconds, node::StreamStatus, StreamInfo};
use firewheel_graph::{
    backend::{AudioBackend, DeviceInfo},
    processor::FirewheelProcessor,
    FirewheelCtx,
};
use ringbuf::traits::{Consumer, Producer, Split};

#[cfg(feature = "input")]
pub mod input;

/// 1024 samples is a latency of about 23 milliseconds, which should
/// be good enough for most games.
const DEFAULT_MAX_BLOCK_FRAMES: u32 = 1024;
const BUILD_STREAM_TIMEOUT: Duration = Duration::from_secs(5);
const MSG_CHANNEL_CAPACITY: usize = 4;

pub type FirewheelContext = FirewheelCtx<CpalBackend>;

/// The configuration of an output audio stream in the CPAL backend.
#[derive(Debug, Clone, PartialEq)]
pub struct CpalOutputConfig {
    /// The host to use. Set to `None` to use the
    /// system's default audio host.
    pub host: Option<cpal::HostId>,

    /// The name of the output device to use. Set to `None` to use the
    /// system's default output device.
    ///
    /// By default this is set to `None`.
    pub device_name: Option<String>,

    /// The desired sample rate to use. Set to `None` to use the device's
    /// default sample rate.
    ///
    /// By default this is set to `None`.
    pub desired_sample_rate: Option<u32>,

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
}

impl Default for CpalOutputConfig {
    fn default() -> Self {
        Self {
            host: None,
            device_name: None,
            desired_sample_rate: None,
            fallback: true,
            desired_latency_frames: DEFAULT_MAX_BLOCK_FRAMES,
        }
    }
}

/// A CPAL backend for Firewheel
pub struct CpalBackend {
    from_err_rx: ringbuf::HeapCons<cpal::StreamError>,
    to_stream_tx: ringbuf::HeapProd<CtxToStreamMsg>,
    _out_stream: cpal::Stream,
}

impl AudioBackend for CpalBackend {
    type Config = CpalOutputConfig;
    type StartStreamError = StreamStartError;
    type StreamError = cpal::StreamError;

    fn available_input_devices() -> Vec<DeviceInfo> {
        let mut devices = Vec::with_capacity(8);

        // TODO: Iterate over all the available hosts?
        let host = cpal::default_host();

        let default_device_name = if let Some(default_device) = host.default_input_device() {
            match default_device.name() {
                Ok(n) => Some(n),
                Err(e) => {
                    log::warn!("Failed to get name of default audio input device: {}", e);
                    None
                }
            }
        } else {
            None
        };

        match host.input_devices() {
            Ok(input_devices) => {
                for device in input_devices {
                    let Ok(name) = device.name() else {
                        continue;
                    };

                    let is_default = if let Some(default_device_name) = &default_device_name {
                        &name == default_device_name
                    } else {
                        false
                    };

                    let default_in_config = match device.default_input_config() {
                        Ok(c) => c,
                        Err(e) => {
                            if is_default {
                                log::warn!("Failed to get default config for the default audio input device: {}", e);
                            }
                            continue;
                        }
                    };

                    devices.push(DeviceInfo {
                        name,
                        num_channels: default_in_config.channels(),
                        is_default,
                    })
                }
            }
            Err(e) => {
                log::error!("Failed to get input audio devices: {}", e);
            }
        }

        devices
    }

    fn available_output_devices() -> Vec<DeviceInfo> {
        let mut devices = Vec::with_capacity(8);

        // TODO: Iterate over all the available hosts?
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

    fn start_stream(config: Self::Config) -> Result<(Self, StreamInfo), Self::StartStreamError> {
        log::info!("Attempting to start output audio stream...");

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

        let mut out_device = None;
        if let Some(device_name) = &config.device_name {
            match host.output_devices() {
                Ok(mut output_devices) => {
                    if let Some(d) = output_devices.find(|d| {
                        if let Ok(name) = d.name() {
                            &name == device_name
                        } else {
                            false
                        }
                    }) {
                        out_device = Some(d);
                    } else if config.fallback {
                        log::warn!("Could not find requested audio output device: {}. Falling back to default device...", &device_name);
                    } else {
                        return Err(StreamStartError::DeviceNotFound(device_name.clone()));
                    }
                }
                Err(e) => {
                    if config.fallback {
                        log::error!("Failed to get output audio devices: {}. Falling back to default device...", e);
                    } else {
                        return Err(e.into());
                    }
                }
            }
        }

        if out_device.is_none() {
            let Some(default_device) = host.default_output_device() else {
                return Err(StreamStartError::DefaultDeviceNotFound);
            };
            out_device = Some(default_device);
        }
        let out_device = out_device.unwrap();

        let out_device_name = out_device.name().unwrap_or_else(|e| {
            log::warn!("Failed to get name of output audio device: {}", e);
            String::from("unknown device")
        });

        let default_config = out_device.default_output_config()?;

        let mut desired_sample_rate = config
            .desired_sample_rate
            .unwrap_or(default_config.sample_rate().0);
        let desired_latency_frames =
            if let &cpal::SupportedBufferSize::Range { min, max } = default_config.buffer_size() {
                Some(config.desired_latency_frames.clamp(min, max))
            } else {
                None
            };

        let supported_configs = out_device.supported_output_configs()?;

        let mut min_sample_rate = u32::MAX;
        let mut max_sample_rate = 0;
        for config in supported_configs.into_iter() {
            min_sample_rate = min_sample_rate.min(config.min_sample_rate().0);
            max_sample_rate = max_sample_rate.max(config.max_sample_rate().0);
        }
        desired_sample_rate = desired_sample_rate.clamp(min_sample_rate, max_sample_rate);

        let num_out_channels = default_config.channels() as usize;
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

        let max_block_frames = match stream_config.buffer_size {
            cpal::BufferSize::Default => DEFAULT_MAX_BLOCK_FRAMES as usize,
            cpal::BufferSize::Fixed(f) => f as usize,
        };

        let (to_stream_tx, from_cx_rx) =
            ringbuf::HeapRb::<CtxToStreamMsg>::new(MSG_CHANNEL_CAPACITY).split();

        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ))]
        let is_alsa = cpal::HostId::name(&host.id()) == "ALSA";

        log::info!(
            "Starting output audio stream with device \"{}\" with configuration {:?}",
            &out_device_name,
            &config
        );

        let mut data_callback = DataCallback::new(
            num_out_channels,
            from_cx_rx,
            stream_config.sample_rate.0,
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd"
            ))]
            is_alsa,
        );

        let (mut err_to_cx_tx, from_err_rx) = ringbuf::HeapRb::<cpal::StreamError>::new(4).split();

        let out_stream = out_device.build_output_stream(
            &stream_config,
            move |output: &mut [f32], info: &cpal::OutputCallbackInfo| {
                data_callback.callback(output, info);
            },
            move |err| {
                let _ = err_to_cx_tx.try_push(err);
            },
            Some(BUILD_STREAM_TIMEOUT),
        )?;

        out_stream.play()?;

        let stream_info = StreamInfo {
            sample_rate: NonZeroU32::new(stream_config.sample_rate.0).unwrap(),
            max_block_frames: NonZeroU32::new(max_block_frames as u32).unwrap(),
            num_stream_in_channels: 0,
            num_stream_out_channels: num_out_channels as u32,
            output_device_name: Some(out_device_name),
            // The engine will overwrite the other values.
            ..Default::default()
        };

        Ok((
            Self {
                from_err_rx,
                to_stream_tx,
                _out_stream: out_stream,
            },
            stream_info,
        ))
    }

    fn set_processor(&mut self, processor: FirewheelProcessor) {
        if let Err(_) = self
            .to_stream_tx
            .try_push(CtxToStreamMsg::NewProcessor(processor))
        {
            panic!("Failed to send new processor to cpal stream");
        }
    }

    fn poll_status(&mut self) -> Result<(), Self::StreamError> {
        if let Some(e) = self.from_err_rx.try_pop() {
            Err(e)
        } else {
            Ok(())
        }
    }
}

struct DataCallback {
    num_out_channels: usize,
    from_cx_rx: ringbuf::HeapCons<CtxToStreamMsg>,
    processor: Option<FirewheelProcessor>,
    sample_rate_recip: f64,
    first_internal_clock_instant: Option<cpal::StreamInstant>,
    prev_stream_instant: Option<cpal::StreamInstant>,
    first_fallback_clock_instant: Option<std::time::Instant>,
    predicted_stream_secs: Option<f64>,

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    ))]
    is_alsa: bool,
}

impl DataCallback {
    fn new(
        num_out_channels: usize,
        from_cx_rx: ringbuf::HeapCons<CtxToStreamMsg>,
        sample_rate: u32,
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ))]
        is_alsa: bool,
    ) -> Self {
        Self {
            num_out_channels,
            from_cx_rx,
            processor: None,
            sample_rate_recip: f64::from(sample_rate).recip(),
            first_internal_clock_instant: None,
            prev_stream_instant: None,
            predicted_stream_secs: None,
            first_fallback_clock_instant: None,
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd"
            ))]
            is_alsa,
        }
    }

    fn callback(&mut self, output: &mut [f32], info: &cpal::OutputCallbackInfo) {
        for msg in self.from_cx_rx.pop_iter() {
            let CtxToStreamMsg::NewProcessor(p) = msg;
            self.processor = Some(p);
        }

        let frames = output.len() / self.num_out_channels;

        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ))]
        let is_alsa = self.is_alsa;

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        )))]
        let is_alsa = false;

        let (internal_clock_secs, underflow) = if is_alsa {
            // There seems to be a bug in ALSA causing the stream timestamps to be
            // unreliable. Fall back to using the system's regular clock instead.

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
                    Some(internal_clock_secs + (frames as f64 * self.sample_rate_recip * 1.2));

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
                        // If this occurs in other APIs as well, then either CPAL is doing
                        // something wrong, or I'm doing something wrong.
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
                    Some(internal_clock_secs + (frames as f64 * self.sample_rate_recip * 1.2));

                self.prev_stream_instant = Some(info.timestamp().playback);

                (ClockSeconds(internal_clock_secs), underflow)
            } else {
                self.first_internal_clock_instant = Some(info.timestamp().playback);
                (ClockSeconds(0.0), false)
            }
        };

        if let Some(processor) = &mut self.processor {
            let mut stream_status = StreamStatus::empty();

            if underflow {
                stream_status.insert(StreamStatus::OUTPUT_UNDERFLOW);
            }

            processor.process_interleaved(
                &[],
                output,
                0,
                self.num_out_channels,
                frames,
                internal_clock_secs,
                stream_status,
            );
        } else {
            output.fill(0.0);
            return;
        }
    }
}

enum CtxToStreamMsg {
    NewProcessor(FirewheelProcessor),
}

/// An error occured while trying to start a CPAL audio stream.
#[derive(Debug, thiserror::Error)]
pub enum StreamStartError {
    #[error("The requested audio device was not found: {0}")]
    DeviceNotFound(String),
    #[error("Could not get audio devices: {0}")]
    FailedToGetDevices(#[from] cpal::DevicesError),
    #[error("Failed to get default audio device")]
    DefaultDeviceNotFound,
    #[error("Failed to get audio device configs: {0}")]
    FailedToGetConfigs(#[from] cpal::SupportedStreamConfigsError),
    #[error("Failed to get audio device config: {0}")]
    FailedToGetConfig(#[from] cpal::DefaultStreamConfigError),
    #[error("Failed to build audio stream: {0}")]
    BuildStreamError(#[from] cpal::BuildStreamError),
    #[error("Failed to play audio stream: {0}")]
    PlayStreamError(#[from] cpal::PlayStreamError),

    #[cfg(feature = "input")]
    #[error("An input stream is already active on this handle")]
    InputStreamAlreadyActive,
    #[cfg(all(feature = "input", not(feature = "resample_inputs")))]
    #[error("Not able to use a samplerate of {0} for the input audio device")]
    CouldNotMatchSampleRate(u32),
}
