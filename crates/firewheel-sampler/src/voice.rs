use std::{num::NonZeroUsize, ops::Range, sync::Arc};

use firewheel_core::{
    dsp::declick::{DeclickValues, Declicker},
    sample_resource::SampleResource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplerVoiceState {
    Playing,
    Paused,
    Finished,
}

pub struct SamplerVoice {
    playhead: u64,
    sample: Option<Arc<dyn SampleResource>>,
    len_samples: u64,
    num_channels: NonZeroUsize,
    state: SamplerVoiceState,
    declicker: Declicker,
}

impl SamplerVoice {
    pub fn new() -> Self {
        Self {
            sample: None,
            playhead: 0,
            len_samples: 0,
            num_channels: NonZeroUsize::MIN,
            state: SamplerVoiceState::Paused,
            declicker: Declicker::default(),
        }
    }

    pub fn len_samples(&self) -> u64 {
        if self.sample.is_some() {
            self.len_samples
        } else {
            0
        }
    }

    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    pub fn init_with_sample(&mut self, sample: &Arc<dyn SampleResource>, playhead: u64) {
        self.len_samples = sample.len_samples();
        self.num_channels = sample.num_channels();
        self.sample = Some(Arc::clone(sample));
        self.playhead = playhead;
        self.state = SamplerVoiceState::Paused;

        if playhead == 0 {
            self.declicker.reset_to_1();
        } else {
            self.declicker.reset_to_0();
        }
    }

    pub fn clear_sample(&mut self) {
        self.sample = None;
        self.len_samples = 0;
        self.state = SamplerVoiceState::Finished;
    }

    pub fn state(&self) -> SamplerVoiceState {
        self.state
    }

    pub fn playhead(&self) -> u64 {
        self.playhead
    }

    pub fn pause(&mut self, declick_values: &DeclickValues) {
        if self.state == SamplerVoiceState::Playing {
            self.state = SamplerVoiceState::Paused;
            self.declicker.fade_to_0(declick_values);
        }
    }

    pub fn resume(&mut self, declick_values: &DeclickValues) {
        if self.state == SamplerVoiceState::Paused {
            self.state = SamplerVoiceState::Playing;
            self.declicker.fade_to_1(declick_values);
        }
    }

    pub fn stop(&mut self, declick_values: &DeclickValues) {
        if self.state != SamplerVoiceState::Finished {
            self.state = SamplerVoiceState::Finished;
            self.declicker.fade_to_0(declick_values);
        }
    }

    pub fn is_processing(&self) -> bool {
        self.sample.is_some()
            && (self.state == SamplerVoiceState::Playing || !self.declicker.is_settled())
    }

    pub fn is_finished(&self) -> bool {
        self.state == SamplerVoiceState::Finished && self.declicker.is_settled()
    }

    /// Returns the number of channels that were filled.
    pub fn process(
        &mut self,
        outputs: &mut [&mut [f32]],
        block_samples: usize,
        looping: bool,
        declick_values: &DeclickValues,
    ) -> usize {
        if !self.is_processing() {
            return 0;
        }

        let first_samples = block_samples.min((self.len_samples - self.playhead) as usize);

        self.process_internal(outputs, 0..first_samples);

        let num_filled_channels = self.num_channels.get().min(outputs.len());

        if self.playhead == self.len_samples {
            if looping {
                self.playhead = 0;
            } else {
                self.state = SamplerVoiceState::Finished;
            }
        }

        if first_samples < block_samples {
            if looping {
                self.process_internal(outputs, first_samples..block_samples);
            } else {
                // Fill the rest of the samples with zeros.
                for buf in outputs[..num_filled_channels].iter_mut() {
                    buf[first_samples..block_samples].fill(0.0);
                }
            }
        }

        self.declicker.process(
            &mut outputs[..num_filled_channels],
            0..block_samples,
            declick_values,
        );

        num_filled_channels
    }

    fn process_internal(&mut self, outputs: &mut [&mut [f32]], output_range: Range<usize>) {
        // TODO: effects like doppler shifting

        let sample = self.sample.as_ref().unwrap();

        let block_samples = output_range.end - output_range.start;
        assert!(self.playhead + block_samples as u64 <= self.len_samples);

        sample.fill_buffers(outputs, 0..block_samples, self.playhead);

        self.playhead += block_samples as u64;
    }
}
