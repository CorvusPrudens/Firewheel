use std::{
    fmt::Debug,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
    u32,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use firewheel_core::{node::StreamStatus, StreamInfo};
use firewheel_graph::{
    backend::DeviceInfo,
    error::ActivateCtxError,
    graph::AudioGraph,
    processor::{FirewheelProcessor, FirewheelProcessorStatus},
    FirewheelConfig, FirewheelGraphCtx, UpdateStatus,
};

/// 1024 samples is a latency of about 23 milliseconds, which should
/// be good enough for most games.
const DEFAULT_MAX_BLOCK_FRAMES: u32 = 1024;
const BUILD_STREAM_TIMEOUT: Duration = Duration::from_secs(5);
const MSG_CHANNEL_CAPACITY: usize = 4;

struct ActiveState<C: Send + 'static> {
    _stream: cpal::Stream,
    _to_stream_tx: rtrb::Producer<CtxToStreamMsg<C>>,
    from_err_rx: rtrb::Consumer<cpal::StreamError>,
    out_device_name: String,
    cpal_config: cpal::StreamConfig,
}

/// A firewheel context using CPAL as the audio backend.
///
/// The generic is a custom global processing context that is available to
/// node processors.
pub struct FirewheelCpalCtx<C: Send + 'static> {
    cx: FirewheelGraphCtx<C>,
    active_state: Option<ActiveState<C>>,
}

impl<C: Send + 'static> FirewheelCpalCtx<C> {
    pub fn new(config: FirewheelConfig) -> Self {
        Self {
            cx: FirewheelGraphCtx::new(config),
            active_state: None,
        }
    }

    /// Get an immutable reference to the audio graph.
    pub fn graph(&self) -> &AudioGraph<C> {
        self.cx.graph()
    }

    /// Get a mutable reference to the audio graph.
    ///
    /// Returns `None` if the context is not currently activated.
    pub fn graph_mut(&mut self) -> Option<&mut AudioGraph<C>> {
        self.cx.graph_mut()
    }

    /// Returns whether or not this context is currently activated.
    pub fn is_activated(&self) -> bool {
        self.cx.is_activated()
    }

    pub fn available_output_devices(&self) -> Vec<DeviceInfo> {
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

    /// Activate the context and start the audio stream.
    ///
    /// Returns an error if the context is already active.
    ///
    /// * `config` - The configuration of the audio stream.
    /// * `user_cx` - A custom global processing context that is available to
    /// node processors.
    pub fn activate(
        &mut self,
        config: AudioStreamConfig,
        user_cx: C,
    ) -> Result<(), (ActivateError, C)> {
        if self.cx.is_activated() {
            return Err((
                ActivateError::ContextError(ActivateCtxError::AlreadyActivated),
                user_cx,
            ));
        }

        let host = cpal::default_host();

        // What a mess cpal's enumeration API is.

        let mut device = None;
        if let Some(output_device_name) = &config.output_device_name {
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
                    } else if config.fallback {
                        log::warn!("Could not find requested audio output device: {}. Falling back to default device...", &output_device_name);
                    } else {
                        return Err((
                            ActivateError::DeviceNotFound(output_device_name.clone()),
                            user_cx,
                        ));
                    }
                }
                Err(e) => {
                    if config.fallback {
                        log::error!("Failed to get output audio devices: {}. Falling back to default device...", e);
                    } else {
                        return Err((e.into(), user_cx));
                    }
                }
            }
        }

        if device.is_none() {
            let Some(default_device) = host.default_output_device() else {
                if config.fallback {
                    log::error!("No default audio output device found. Falling back to dummy output device...");
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err((ActivateError::DefaultDeviceNotFound, user_cx));
                }
            };
            device = Some(default_device);
        }
        let device = device.unwrap();

        let default_cpal_config = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                if config.fallback {
                    log::error!(
                        "Failed to get default config for output audio device: {}. Falling back to dummy output device...",
                        e
                    );
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err((e.into(), user_cx));
                }
            }
        };

        let mut desired_sample_rate = config
            .desired_sample_rate
            .unwrap_or(default_cpal_config.sample_rate().0);
        let desired_latency_samples = if let &cpal::SupportedBufferSize::Range { min, max } =
            default_cpal_config.buffer_size()
        {
            Some(config.desired_latency_samples.clamp(min, max))
        } else {
            None
        };

        let supported_cpal_configs = match device.supported_output_configs() {
            Ok(c) => c,
            Err(e) => {
                if config.fallback {
                    log::error!(
                        "Failed to get configs for output audio device: {}. Falling back to dummy output device...",
                        e
                    );
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err((e.into(), user_cx));
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

        let desired_buffer_size = if let Some(samples) = desired_latency_samples {
            cpal::BufferSize::Fixed(samples)
        } else {
            cpal::BufferSize::Default
        };

        let cpal_config = cpal::StreamConfig {
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

        let max_block_samples = match cpal_config.buffer_size {
            cpal::BufferSize::Default => DEFAULT_MAX_BLOCK_FRAMES as usize,
            cpal::BufferSize::Fixed(f) => f as usize,
        };

        let (mut to_stream_tx, from_ctx_rx) =
            rtrb::RingBuffer::<CtxToStreamMsg<C>>::new(MSG_CHANNEL_CAPACITY);
        let (mut err_to_cx_tx, from_err_rx) =
            rtrb::RingBuffer::<cpal::StreamError>::new(MSG_CHANNEL_CAPACITY);

        let mut data_callback = DataCallback::new(
            num_in_channels,
            num_out_channels,
            from_ctx_rx,
            cpal_config.sample_rate.0,
        );

        // There doesn't seem to be a way to get the stream latency from cpal before
        // the stream starts when using the `Default` buffer size, so do this as a
        // workaround.
        let (mut sl, mut sl1, sl2) = if let cpal::BufferSize::Default = cpal_config.buffer_size {
            let sl = Arc::new(AtomicU32::new(u32::MAX));
            (Some(Arc::clone(&sl)), Some(Arc::clone(&sl)), Some(sl))
        } else {
            (None, None, None)
        };

        let mut is_first_callback = true;
        let stream = match device.build_output_stream(
            &cpal_config,
            move |output: &mut [f32], info: &cpal::OutputCallbackInfo| {
                if is_first_callback {
                    // The second audio callback is usually more reliable for getting
                    // the average latency.
                    is_first_callback = false;
                } else if let Some(sl1) = sl1.take() {
                    sl1.store((output.len() / num_out_channels) as u32, Ordering::Relaxed);
                }

                data_callback.callback(output, info);
            },
            move |err| {
                // Make sure we don't deadlock if there happens to be an error right away.
                if let Some(sl2) = &sl2 {
                    sl2.store(0, Ordering::Relaxed);
                }

                let _ = err_to_cx_tx.push(err);
            },
            Some(BUILD_STREAM_TIMEOUT),
        ) {
            Ok(s) => s,
            Err(e) => {
                // Make sure we don't deadlock if there happens to be an error.
                #[allow(unused_assignments)]
                {
                    sl = None;
                }

                if config.fallback {
                    log::error!("Failed to start output audio stream: {}. Falling back to dummy output device...", e);
                    // TODO: Use dummy audio backend as fallback.
                    todo!()
                } else {
                    return Err((e.into(), user_cx));
                }
            }
        };

        if let Err(e) = stream.play() {
            return Err((e.into(), user_cx));
        }

        let stream_latency_samples = if let Some(sl) = sl.take() {
            while sl.load(Ordering::Relaxed) == u32::MAX {
                std::thread::sleep(Duration::from_millis(1));
            }

            let l = sl.load(Ordering::Relaxed);

            log::info!("Estimated audio stream latency: {}", l);

            l
        } else if let cpal::BufferSize::Fixed(samples) = cpal_config.buffer_size {
            samples
        } else {
            unreachable!()
        };

        let processor = match self.cx.activate(
            StreamInfo {
                sample_rate: cpal_config.sample_rate.0,
                max_block_samples: max_block_samples as u32,
                stream_latency_samples,
                num_stream_in_channels: num_in_channels as u32,
                num_stream_out_channels: num_out_channels as u32,
            },
            user_cx,
        ) {
            Ok(p) => p,
            Err((e, c)) => {
                return Err((e.into(), c));
            }
        };

        to_stream_tx
            .push(CtxToStreamMsg::NewProcessor(processor))
            .unwrap();

        self.active_state = Some(ActiveState {
            _stream: stream,
            _to_stream_tx: to_stream_tx,
            from_err_rx,
            out_device_name,
            cpal_config,
        });

        Ok(())
    }

    /// Get the name of the audio output device.
    ///
    /// Returns `None` if the context is not currently activated.
    pub fn out_device_name(&self) -> Option<&str> {
        self.active_state
            .as_ref()
            .map(|s| s.out_device_name.as_str())
    }

    /// Get information about the current audio stream.
    ///
    /// Returns `None` if the context is not currently activated.
    pub fn stream_info(&self) -> Option<&StreamInfo> {
        self.cx.stream_info()
    }

    /// Get the current configuration of the audio stream.
    ///
    /// Returns `None` if the context is not currently activated.
    pub fn stream_config(&self) -> Option<&cpal::StreamConfig> {
        self.active_state.as_ref().map(|s| &s.cpal_config)
    }

    /// Update the firewheel context.
    ///
    /// This must be called reguarly once the context has been activated
    /// (i.e. once every frame).
    pub fn update(&mut self) -> UpdateStatus<C> {
        if let Some(state) = &mut self.active_state {
            if let Ok(e) = state.from_err_rx.pop() {
                let user_cx = self.cx.deactivate(false);
                self.active_state = None;

                return UpdateStatus::Deactivated {
                    error: Some(Box::new(e)),
                    returned_user_cx: user_cx,
                };
            }
        }

        match self.cx.update() {
            UpdateStatus::Active { graph_error } => UpdateStatus::Active { graph_error },
            UpdateStatus::Inactive => UpdateStatus::Inactive,
            UpdateStatus::Deactivated {
                returned_user_cx,
                error,
            } => {
                if self.active_state.is_some() {
                    self.active_state = None;
                }

                UpdateStatus::Deactivated {
                    error,
                    returned_user_cx,
                }
            }
        }
    }

    /// Deactivate the firewheel context and stop the audio stream.
    ///
    /// This will block the thread until either the processor has
    /// been successfully dropped or a timeout has been reached.
    ///
    /// If the stream is still currently running, then the context
    /// will attempt to cleanly deactivate the processor. If not,
    /// then the context will wait for either the processor to be
    /// dropped or a timeout being reached.
    ///
    /// If the context is already deactivated, then this will do
    /// nothing and return `None`.
    pub fn deactivate(&mut self) -> Option<C> {
        if self.cx.is_activated() {
            let user_cx = self.cx.deactivate(self.active_state.is_some());
            self.active_state = None;
            user_cx
        } else {
            None
        }
    }
}

// Implement Debug so `unwrap()` can be used.
impl<C: Send + 'static> Debug for FirewheelCpalCtx<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FirewheelCpalCtx")
    }
}

/// The configuration of an audio stream
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamConfig {
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
    pub desired_latency_samples: u32,

    /// Whether or not to fall back to the default device and then a
    /// dummy output device if a device with the given configuration
    /// could not be found.
    ///
    /// By default this is set to `true`.
    pub fallback: bool,
}

impl Default for AudioStreamConfig {
    fn default() -> Self {
        Self {
            output_device_name: None,
            desired_sample_rate: None,
            desired_latency_samples: DEFAULT_MAX_BLOCK_FRAMES,
            fallback: true,
        }
    }
}

struct DataCallback<C: Send + 'static> {
    num_in_channels: usize,
    num_out_channels: usize,
    from_ctx_rx: rtrb::Consumer<CtxToStreamMsg<C>>,
    processor: Option<FirewheelProcessor<C>>,
    sample_rate_recip: f64,
    first_stream_instant: Option<cpal::StreamInstant>,
    predicted_stream_secs: f64,
    is_first_callback: bool,
}

impl<C: Send + 'static> DataCallback<C> {
    fn new(
        num_in_channels: usize,
        num_out_channels: usize,
        from_ctx_rx: rtrb::Consumer<CtxToStreamMsg<C>>,
        sample_rate: u32,
    ) -> Self {
        Self {
            num_in_channels,
            num_out_channels,
            from_ctx_rx,
            processor: None,
            sample_rate_recip: f64::from(sample_rate).recip(),
            first_stream_instant: None,
            predicted_stream_secs: 1.0,
            is_first_callback: true,
        }
    }

    fn callback(&mut self, output: &mut [f32], info: &cpal::OutputCallbackInfo) {
        while let Ok(msg) = self.from_ctx_rx.pop() {
            let CtxToStreamMsg::NewProcessor(p) = msg;
            self.processor = Some(p);
        }

        let samples = output.len() / self.num_out_channels;

        let (stream_time_secs, underflow) = if self.is_first_callback {
            // Apparently there is a bug in CPAL where the callback instant in
            // the first callback can be greater than in the second callback.
            //
            // Work around this by ignoring the first callback instant.
            self.is_first_callback = false;
            self.predicted_stream_secs = samples as f64 * self.sample_rate_recip;
            (0.0, false)
        } else if let Some(instant) = &self.first_stream_instant {
            let stream_time_secs = info
                .timestamp()
                .callback
                .duration_since(instant)
                .unwrap()
                .as_secs_f64();

            // If the stream time is significantly greater than the predicted stream
            // time, it means an underflow has occurred.
            let underrun = stream_time_secs > self.predicted_stream_secs;

            // Calculate the next predicted stream time to detect underflows.
            //
            // Add a little bit of wiggle room to account for tiny clock
            // innacuracies and rounding errors.
            self.predicted_stream_secs =
                stream_time_secs + (samples as f64 * self.sample_rate_recip * 1.2);

            (stream_time_secs, underrun)
        } else {
            self.first_stream_instant = Some(info.timestamp().callback);
            let stream_time_secs = self.predicted_stream_secs;
            self.predicted_stream_secs += samples as f64 * self.sample_rate_recip * 1.2;
            (stream_time_secs, false)
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
                stream_time_secs,
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

impl<C: Send + 'static> Drop for FirewheelCpalCtx<C> {
    fn drop(&mut self) {
        if self.cx.is_activated() {
            self.cx.deactivate(self.active_state.is_some());
        }
    }
}
enum CtxToStreamMsg<C: Send + 'static> {
    NewProcessor(FirewheelProcessor<C>),
}

/// An error occured while trying to activate an [`InactiveFwCpalCtx`]
#[derive(Debug, thiserror::Error)]
pub enum ActivateError {
    #[error("Firewheel context failed to activate: {0}")]
    ContextError(#[from] ActivateCtxError),
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
