use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::{
        declick::{Declicker, FadeType},
        filter::{
            single_pole_iir::{
                OnePoleIirHPFCoeff, OnePoleIirHPFCoeffSimd, OnePoleIirHPFSimd, OnePoleIirLPFCoeff,
                OnePoleIirLPFCoeffSimd, OnePoleIirLPFSimd,
            },
            smoothing_filter::DEFAULT_SMOOTH_SECONDS,
        },
    },
    event::ProcEvents,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, ConstructProcessorContext, EmptyConfig,
        ProcBuffers, ProcExtra, ProcInfo, ProcessStatus,
    },
    param::smoother::SmoothedParam,
    SilenceMask, StreamInfo,
};

use super::{MAX_HZ, MIN_HZ};

const CALC_FILTER_COEFF_INTERVAL: usize = 16;

pub type FastBandpassMonoNode = FastBandpassNode<1>;
pub type FastBandpassStereoNode = FastBandpassNode<2>;

/// A simple single-pole IIR bandpass filter.
///
/// It is computationally efficient, but it doesn't do that great of
/// a job at attenuating low frequencies.
#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
pub struct FastBandpassNode<const CHANNELS: usize> {
    /// The cutoff frequency in hertz in the range `[20.0, 20_000.0]`.
    pub cutoff_hz: f32,
    /// Whether or not this node is enabled.
    pub enabled: bool,

    /// The time in seconds of the internal smoothing filter.
    ///
    /// By default this is set to `0.015` (15ms).
    pub smooth_seconds: f32,
}

impl<const CHANNELS: usize> Default for FastBandpassNode<CHANNELS> {
    fn default() -> Self {
        Self {
            cutoff_hz: 1_000.0,
            enabled: true,
            smooth_seconds: DEFAULT_SMOOTH_SECONDS,
        }
    }
}

// Implement the AudioNode type for your node.
impl<const CHANNELS: usize> AudioNode for FastBandpassNode<CHANNELS> {
    type Configuration = EmptyConfig;

    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
        AudioNodeInfo::new()
            .debug_name("fast_lowpass")
            .channel_config(ChannelConfig {
                num_inputs: ChannelCount::new(CHANNELS as u32).unwrap(),
                num_outputs: ChannelCount::new(CHANNELS as u32).unwrap(),
            })
    }

    // Construct the realtime processor counterpart using the given information
    // about the audio stream.
    //
    // This method is called before the node processor is sent to the realtime
    // thread, so it is safe to do non-realtime things here like allocating.
    fn construct_processor(
        &self,
        _config: &Self::Configuration,
        cx: ConstructProcessorContext,
    ) -> impl AudioNodeProcessor {
        // The reciprocal of the sample rate.
        let sample_rate_recip = cx.stream_info.sample_rate_recip as f32;

        let cutoff_hz = self.cutoff_hz.clamp(MIN_HZ, MAX_HZ);

        Processor {
            lpf: OnePoleIirLPFSimd::default(),
            lpf_coeff: OnePoleIirLPFCoeffSimd::<CHANNELS>::splat(OnePoleIirLPFCoeff::new(
                cutoff_hz,
                sample_rate_recip,
            )),
            hpf: OnePoleIirHPFSimd::default(),
            hpf_coeff: OnePoleIirHPFCoeffSimd::<CHANNELS>::splat(OnePoleIirHPFCoeff::new(
                cutoff_hz,
                sample_rate_recip,
            )),
            cutoff_hz: SmoothedParam::new(
                cutoff_hz,
                Default::default(),
                cx.stream_info.sample_rate,
            ),
            enable_declicker: Declicker::from_enabled(self.enabled),
            sample_rate_recip,
        }
    }
}

// The realtime processor counterpart to your node.
struct Processor<const CHANNELS: usize> {
    lpf: OnePoleIirLPFSimd<CHANNELS>,
    hpf: OnePoleIirHPFSimd<CHANNELS>,
    lpf_coeff: OnePoleIirLPFCoeffSimd<CHANNELS>,
    hpf_coeff: OnePoleIirHPFCoeffSimd<CHANNELS>,

    cutoff_hz: SmoothedParam,
    enable_declicker: Declicker,
    sample_rate_recip: f32,
}

impl<const CHANNELS: usize> AudioNodeProcessor for Processor<CHANNELS> {
    // The realtime process method.
    fn process(
        &mut self,
        // Information about the process block.
        info: &ProcInfo,
        // The buffers of data to process.
        buffers: ProcBuffers,
        // The list of events for our node to process.
        events: &mut ProcEvents,
        // Extra buffers and utilities.
        extra: &mut ProcExtra,
    ) -> ProcessStatus {
        let mut cutoff_changed = false;

        // Process the events.
        //
        // We don't need to keep around a `FilterNode` instance,
        // so we can just match on each event directly.
        for patch in events.drain_patches::<FastBandpassNode<CHANNELS>>() {
            match patch {
                FastBandpassNodePatch::CutoffHz(cutoff) => {
                    cutoff_changed = true;
                    self.cutoff_hz.set_value(cutoff.clamp(MIN_HZ, MAX_HZ));
                }
                FastBandpassNodePatch::Enabled(enabled) => {
                    // Tell the declicker to crossfade.
                    self.enable_declicker
                        .fade_to_enabled(enabled, &extra.declick_values);
                }
                FastBandpassNodePatch::SmoothSeconds(seconds) => {
                    self.cutoff_hz.set_smooth_seconds(seconds, info.sample_rate);
                }
            }
        }

        if self.enable_declicker.disabled() {
            // Disabled. Bypass this node.
            return ProcessStatus::Bypass;
        }

        if info.in_silence_mask.all_channels_silent(CHANNELS) && self.enable_declicker.is_settled()
        {
            // Outputs will be silent, so no need to process.

            // Reset the smoothers and filters since they don't need to smooth any
            // output.
            self.cutoff_hz.reset();
            self.lpf.reset();
            self.hpf.reset();
            self.enable_declicker.reset_to_target();

            return ProcessStatus::ClearAllOutputs;
        }

        assert!(buffers.inputs.len() == CHANNELS);
        assert!(buffers.outputs.len() == CHANNELS);
        for ch in buffers.inputs.iter() {
            assert!(ch.len() >= info.frames);
        }
        for ch in buffers.outputs.iter() {
            assert!(ch.len() >= info.frames);
        }

        if self.cutoff_hz.is_smoothing() {
            for i in 0..info.frames {
                let cutoff_hz = self.cutoff_hz.next_smoothed();

                // Because recalculating filter coefficients is expensive, a trick like
                // this can be use to only recalculate them every CALC_FILTER_COEFF_INTERVAL
                // frames.
                //
                // TODO: use core::hint::cold_path() once that stabilizes
                //
                // TODO: Alternatively, this could be optimized using a lookup table
                if i & (CALC_FILTER_COEFF_INTERVAL - 1) == 0 {
                    self.lpf_coeff = OnePoleIirLPFCoeffSimd::splat(OnePoleIirLPFCoeff::new(
                        cutoff_hz,
                        info.sample_rate_recip as f32,
                    ));
                    self.hpf_coeff = OnePoleIirHPFCoeffSimd::splat(OnePoleIirHPFCoeff::new(
                        cutoff_hz,
                        info.sample_rate_recip as f32,
                    ));
                }

                let s: [f32; CHANNELS] = core::array::from_fn(|ch_i| {
                    // Safety: These bounds have been checked above.
                    unsafe { *buffers.inputs.get_unchecked(ch_i).get_unchecked(i) }
                });

                let out = self.lpf.process(s, &self.lpf_coeff);
                let out = self.hpf.process(out, &self.hpf_coeff);

                for ch_i in 0..CHANNELS {
                    // Safety: These bounds have been checked above.
                    unsafe {
                        *buffers.outputs.get_unchecked_mut(ch_i).get_unchecked_mut(i) = out[ch_i];
                    }
                }
            }

            if self.cutoff_hz.settle() {
                self.lpf_coeff = OnePoleIirLPFCoeffSimd::splat(OnePoleIirLPFCoeff::new(
                    self.cutoff_hz.target_value(),
                    info.sample_rate_recip as f32,
                ));
                self.hpf_coeff = OnePoleIirHPFCoeffSimd::splat(OnePoleIirHPFCoeff::new(
                    self.cutoff_hz.target_value(),
                    info.sample_rate_recip as f32,
                ));
            }
        } else {
            // The cutoff parameter is not currently smoothing, so we can optimize by
            // only updating the filter coefficients once.
            if cutoff_changed {
                self.lpf_coeff = OnePoleIirLPFCoeffSimd::splat(OnePoleIirLPFCoeff::new(
                    self.cutoff_hz.target_value(),
                    info.sample_rate_recip as f32,
                ));
                self.hpf_coeff = OnePoleIirHPFCoeffSimd::splat(OnePoleIirHPFCoeff::new(
                    self.cutoff_hz.target_value(),
                    info.sample_rate_recip as f32,
                ));
            }

            for i in 0..info.frames {
                let s: [f32; CHANNELS] = core::array::from_fn(|ch_i| {
                    // Safety: These bounds have been checked above.
                    unsafe { *buffers.inputs.get_unchecked(ch_i).get_unchecked(i) }
                });

                let out = self.lpf.process(s, &self.lpf_coeff);
                let out = self.hpf.process(out, &self.hpf_coeff);

                for ch_i in 0..CHANNELS {
                    // Safety: These bounds have been checked above.
                    unsafe {
                        *buffers.outputs.get_unchecked_mut(ch_i).get_unchecked_mut(i) = out[ch_i];
                    }
                }
            }
        }

        // Crossfade between the wet and dry signals to declick enabling/disabling.
        self.enable_declicker.process_crossfade(
            buffers.inputs,
            buffers.outputs,
            info.frames,
            &extra.declick_values,
            FadeType::Linear,
        );

        ProcessStatus::OutputsModified {
            out_silence_mask: SilenceMask::NONE_SILENT,
        }
    }

    fn new_stream(&mut self, stream_info: &StreamInfo) {
        self.sample_rate_recip = stream_info.sample_rate_recip as f32;

        self.cutoff_hz.update_sample_rate(stream_info.sample_rate);
        self.lpf_coeff = OnePoleIirLPFCoeffSimd::splat(OnePoleIirLPFCoeff::new(
            self.cutoff_hz.target_value(),
            stream_info.sample_rate_recip as f32,
        ));
        self.hpf_coeff = OnePoleIirHPFCoeffSimd::splat(OnePoleIirHPFCoeff::new(
            self.cutoff_hz.target_value(),
            stream_info.sample_rate_recip as f32,
        ));
    }
}
