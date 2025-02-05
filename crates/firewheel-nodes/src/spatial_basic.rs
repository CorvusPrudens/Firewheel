//! A 3D spatial positioning node using a basic (and naive) algorithm. It does
//! not make use of any fancy binaural algorithms, rather it just applies basic
//! panning and filtering.

use std::f32::consts::PI;

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch, PatchParams},
    dsp::{decibel::normalized_volume_to_raw_gain, pan_law::PanLaw},
    event::{NodeEventList, TryConvert},
    node::{
        AudioNodeConstructor, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus,
        NUM_SCRATCH_BUFFERS,
    },
    param::smoother::{SmoothedParam, SmootherConfig},
    SilenceMask,
};

const DAMPING_CUTOFF_HZ_MIN: f32 = 20.0;
const DAMPING_CUTOFF_HZ_MAX: f32 = 21_500.0;
const CALC_FILTER_COEFF_INTERVAL: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialBasicConfig {
    /// The time in seconds of the internal smoothing filter.
    ///
    /// By default this is set to `0.01` (10ms).
    pub smooth_secs: f32,
}

impl Default for SpatialBasicConfig {
    fn default() -> Self {
        Self {
            smooth_secs: 10.0 / 1_000.0,
        }
    }
}

/// The parameters for a 3D spatial positioning node using a basic (and naive) algorithm.
/// It does not make use of any fancy binaural algorithms, rather it just applies basic
/// panning and filtering.
#[derive(Diff, Debug, Clone, Copy, PartialEq)]
pub struct SpatialBasicParams {
    /// The normalized volume where `0.0` is mute and `1.0` is unity gain. This is
    /// applied before the spatialization algorithm.
    ///
    /// By default this is set to `1.0`.
    pub normalized_volume: f32,

    /// A 3D vector representing the offset between the listener and the
    /// sound source.
    ///
    /// The coordinates are `[x, y, z]`.
    ///
    /// * `-x` is to the left of the listener, and `+x` is the the right of the listener
    /// * `-y` is below the listener, and `+y` is above the listener.
    /// * `-z` is in front of the listener, and `+z` is behind the listener
    ///
    /// The origin `[0.0, 0.0, 0.0]` will have a volume equal to the original signal
    /// (with the `normalized_volume` paramter applied). A  distance  of `10.0`
    /// from the origin will have a volume equal to `-6dB`, a distance of `20.0` will
    /// have a volume equal to `-12dB`, a distance of `40.0` will have a volume equal
    /// to `-24dB`, and so on (every doubling of distance is a 6dB reduction in
    /// volume).
    ///
    /// 1 unit is roughly equal to 1 meter (if I did my math right), but you may wish
    /// to scale this unit as you see fit.
    ///
    /// By default this is set to `[0.0, 0.0, 0.0]`
    pub offset: [f32; 3],

    /// The amount of damping (lowpass) applied to the signal per unit distance.
    ///
    /// A value of `0.0` is no damping, and a value of `1.0` fully dampens the signal
    /// at a distance of 150 units (-90dB).
    ///
    /// Increasing this value to a larger number can be used to give the effect of a
    /// sound playing behind a wall.
    ///
    /// By default this is set to `0.9`.
    pub damping_factor: f32,

    /// The threshold for the maximum amount of panning that can occur, in the range
    /// `[0.0, 1.0]`, where `0.0` is no panning and `1.0` is full panning (where one
    /// of the channels is fully silent when panned hard left or right).
    ///
    /// Setting this to a value less than `1.0` can help remove some of the
    /// jarringness of having a sound playing in only one ear.
    ///
    /// By default this is set to `0.58`.
    pub panning_threshold: f32,
}

impl Patch for SpatialBasicParams {
    fn patch(
        &mut self,
        data: &firewheel_core::event::ParamData,
        path: &[u32],
    ) -> Result<(), firewheel_core::diff::PatchError> {
        match path.first() {
            Some(0) => {
                let value: f32 = data.try_convert()?;
                self.normalized_volume = value.max(0.0);

                if self.normalized_volume < 0.00001 {
                    self.normalized_volume = 0.0;
                }
            }
            Some(1) => {
                self.offset.patch(data, &path[1..])?;

                for x in self.offset.iter_mut() {
                    if !x.is_normal() {
                        *x = 0.0;
                    }
                }
            }
            Some(2) => {
                let value: f32 = data.try_convert()?;
                self.damping_factor = value.max(0.0);
            }
            Some(3) => {
                let value: f32 = data.try_convert()?;
                self.panning_threshold = value.clamp(0.0, 1.0);
            }
            _ => return Err(firewheel_core::diff::PatchError::InvalidPath),
        }

        Ok(())
    }
}

impl Default for SpatialBasicParams {
    fn default() -> Self {
        Self {
            normalized_volume: 1.0,
            offset: [0.0, 0.0, 0.0],
            damping_factor: 0.9,
            panning_threshold: 0.58,
        }
    }
}

impl SpatialBasicParams {
    /// Create a volume pan node constructor using these parameters.
    pub fn constructor(&self, config: SpatialBasicConfig) -> Constructor {
        Constructor {
            params: *self,
            config,
        }
    }

    pub fn compute_values(&self) -> ComputedValues {
        let x2_z2 = (self.offset[0] * self.offset[0]) + (self.offset[2] * self.offset[2]);
        let xyz_distance = (x2_z2 + (self.offset[1] * self.offset[1])).sqrt();
        let xz_distance = x2_z2.sqrt();

        let distance_gain = 10.0f32.powf(-0.03 * xyz_distance);

        let pan = if xz_distance > 0.0 {
            (self.offset[0] / xz_distance) * self.panning_threshold.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (pan_gain_l, pan_gain_r) = PanLaw::EqualPower3dB.compute_gains(pan);

        let volume_gain = normalized_volume_to_raw_gain(self.normalized_volume);

        let damping_cutoff_hz = if self.damping_factor < 0.00001 {
            None
        } else {
            // A distance of 150.0 is a dB value of -90.0.
            let damping_normal =
                ((150.0 - xyz_distance.min(150.0)) / 150.0).powf(self.damping_factor);

            Some(
                (DAMPING_CUTOFF_HZ_MIN
                    + ((DAMPING_CUTOFF_HZ_MAX - DAMPING_CUTOFF_HZ_MIN) * damping_normal))
                    .clamp(DAMPING_CUTOFF_HZ_MIN, DAMPING_CUTOFF_HZ_MAX),
            )
        };

        ComputedValues {
            gain_l: pan_gain_l * distance_gain * volume_gain,
            gain_r: pan_gain_r * distance_gain * volume_gain,
            damping_cutoff_hz,
        }
    }
}

pub struct ComputedValues {
    pub gain_l: f32,
    pub gain_r: f32,
    pub damping_cutoff_hz: Option<f32>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Constructor {
    pub params: SpatialBasicParams,
    pub config: SpatialBasicConfig,
}

impl AudioNodeConstructor for Constructor {
    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            debug_name: "spatial_basic",
            channel_config: ChannelConfig {
                num_inputs: ChannelCount::STEREO,
                num_outputs: ChannelCount::STEREO,
            },
            uses_events: true,
        }
    }

    fn processor(
        &mut self,
        stream_info: &firewheel_core::StreamInfo,
    ) -> Box<dyn AudioNodeProcessor> {
        let computed_values = self.params.compute_values();

        dbg!(stream_info.sample_rate);

        Box::new(Processor {
            gain_l: SmoothedParam::new(
                computed_values.gain_l,
                SmootherConfig {
                    smooth_secs: self.config.smooth_secs,
                    ..Default::default()
                },
                stream_info.sample_rate,
            ),
            gain_r: SmoothedParam::new(
                computed_values.gain_r,
                SmootherConfig {
                    smooth_secs: self.config.smooth_secs,
                    ..Default::default()
                },
                stream_info.sample_rate,
            ),
            damping_cutoff_hz: SmoothedParam::new(
                computed_values
                    .damping_cutoff_hz
                    .unwrap_or(DAMPING_CUTOFF_HZ_MAX),
                SmootherConfig {
                    smooth_secs: self.config.smooth_secs,
                    ..Default::default()
                },
                stream_info.sample_rate,
            ),
            damping_disabled: computed_values.damping_cutoff_hz.is_none(),
            filter_l: OnePoleLPBiquad::default(),
            filter_r: OnePoleLPBiquad::default(),
            params: self.params,
            prev_block_was_silent: true,
            sample_rate_recip: stream_info.sample_rate_recip as f32,
        })
    }
}

struct Processor {
    gain_l: SmoothedParam,
    gain_r: SmoothedParam,
    damping_cutoff_hz: SmoothedParam,
    damping_disabled: bool,

    filter_l: OnePoleLPBiquad,
    filter_r: OnePoleLPBiquad,

    params: SpatialBasicParams,

    prev_block_was_silent: bool,
    sample_rate_recip: f32,
}

impl AudioNodeProcessor for Processor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        mut events: NodeEventList,
        proc_info: &ProcInfo,
        _scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    ) -> ProcessStatus {
        let mut params_changed = false;

        events.for_each(|event| {
            self.params.patch_params(event);
            params_changed = true;
        });

        if params_changed {
            let computed_values = self.params.compute_values();

            self.gain_l.set_value(computed_values.gain_l);
            self.gain_r.set_value(computed_values.gain_r);

            if let Some(cutoff_hz) = computed_values.damping_cutoff_hz {
                self.damping_cutoff_hz.set_value(cutoff_hz);
                self.damping_disabled = false;
            } else {
                self.damping_cutoff_hz.set_value(DAMPING_CUTOFF_HZ_MAX);
                self.damping_disabled = true;
            }

            if self.prev_block_was_silent {
                // Previous block was silent, so no need to smooth.
                self.gain_l.reset();
                self.gain_r.reset();
                self.damping_cutoff_hz.reset();
                self.filter_l.reset();
                self.filter_r.reset();
            }
        }

        self.prev_block_was_silent = false;

        if proc_info.in_silence_mask.all_channels_silent(2) {
            self.gain_l.reset();
            self.gain_r.reset();
            self.damping_cutoff_hz.reset();
            self.filter_l.reset();
            self.filter_r.reset();

            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        let in1 = &inputs[0][..proc_info.frames];
        let in2 = &inputs[1][..proc_info.frames];
        let (out1, out2) = outputs.split_first_mut().unwrap();
        let out1 = &mut out1[..proc_info.frames];
        let out2 = &mut out2[0][..proc_info.frames];

        if !self.gain_l.is_smoothing()
            && !self.gain_r.is_smoothing()
            && !self.damping_cutoff_hz.is_smoothing()
        {
            if self.gain_l.target_value() == 0.0 && self.gain_r.target_value() == 0.0 {
                self.gain_l.reset();
                self.gain_r.reset();
                self.damping_cutoff_hz.reset();
                self.filter_l.reset();
                self.filter_r.reset();

                self.prev_block_was_silent = true;

                return ProcessStatus::ClearAllOutputs;
            } else if self.damping_disabled {
                for i in 0..proc_info.frames {
                    out1[i] = in1[i] * self.gain_l.target_value();
                    out2[i] = in2[i] * self.gain_r.target_value();
                }
            } else {
                // The cutoff parameter is not currently smoothing, so we can optimize by
                // only updating the filter coefficients once.
                self.filter_l.set_cutoff(
                    self.damping_cutoff_hz.target_value(),
                    self.sample_rate_recip,
                );
                self.filter_r.copy_cutoff_from(&self.filter_l);

                for i in 0..proc_info.frames {
                    out1[i] = in1[i] * self.gain_l.target_value();
                    out2[i] = in2[i] * self.gain_r.target_value();

                    out1[i] = self.filter_l.process(out1[i]);
                    out2[i] = self.filter_r.process(out2[i]);
                }
            }

            ProcessStatus::outputs_modified(proc_info.in_silence_mask);
        } else {
            if self.damping_disabled && !self.damping_cutoff_hz.is_smoothing() {
                for i in 0..proc_info.frames {
                    let gain_l = self.gain_l.next_smoothed();
                    let gain_r = self.gain_r.next_smoothed();

                    out1[i] = in1[i] * gain_l;
                    out2[i] = in2[i] * gain_r;
                }
            } else {
                for i in 0..proc_info.frames {
                    let cutoff_hz = self.damping_cutoff_hz.next_smoothed();
                    let gain_l = self.gain_l.next_smoothed();
                    let gain_r = self.gain_r.next_smoothed();

                    out1[i] = in1[i] * gain_l;
                    out2[i] = in2[i] * gain_r;

                    // Because recalculating filter coefficients is expensive, a trick like
                    // this can be use to only recalculate them every CALC_FILTER_COEFF_INTERVAL
                    // frames.
                    if i & (CALC_FILTER_COEFF_INTERVAL - 1) == 0 {
                        self.filter_l.set_cutoff(cutoff_hz, self.sample_rate_recip);
                        self.filter_r.copy_cutoff_from(&self.filter_l);
                    }

                    out1[i] = self.filter_l.process(out1[i]);
                    out2[i] = self.filter_r.process(out2[i]);
                }
            }
        }

        return ProcessStatus::outputs_modified(SilenceMask::NONE_SILENT);
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        self.sample_rate_recip = stream_info.sample_rate_recip as f32;

        self.gain_l.update_sample_rate(stream_info.sample_rate);
        self.gain_r.update_sample_rate(stream_info.sample_rate);
        self.damping_cutoff_hz
            .update_sample_rate(stream_info.sample_rate);

        self.filter_l.set_cutoff(
            self.damping_cutoff_hz.target_value(),
            self.sample_rate_recip,
        );
        self.filter_r.copy_cutoff_from(&self.filter_l);
    }
}

// A simple one pole lowpass biquad filter.
#[derive(Default)]
struct OnePoleLPBiquad {
    a0: f32,
    b1: f32,
    z1: f32,
}

impl OnePoleLPBiquad {
    pub fn reset(&mut self) {
        self.z1 = 0.0;
    }

    #[inline]
    pub fn set_cutoff(&mut self, cutoff_hz: f32, sample_rate_recip: f32) {
        self.b1 = (-2.0 * PI * cutoff_hz * sample_rate_recip).exp();
        self.a0 = 1.0 - self.b1;
    }

    #[inline]
    pub fn copy_cutoff_from(&mut self, other: &Self) {
        self.a0 = other.a0;
        self.b1 = other.b1;
    }

    #[inline]
    pub fn process(&mut self, s: f32) -> f32 {
        self.z1 = (self.a0 * s) + (self.b1 * self.z1);
        self.z1
    }
}
