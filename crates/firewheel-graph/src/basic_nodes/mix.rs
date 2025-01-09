use std::num::NonZeroU32;

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    event::NodeEventList,
    node::{AudioNodeConstructor, AudioNodeProcessor, ProcInfo, ProcessStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixNodeConfig {
    channels: NonZeroU32,
    num_in_streams: NonZeroU32,
}

impl MixNodeConfig {
    pub fn new(channels: NonZeroU32, num_in_streams: NonZeroU32) -> Result<Self, MixConfigError> {
        if channels.get() * num_in_streams.get() > 64 {
            return Err(MixConfigError::TooManyChannels {
                channels,
                num_in_streams,
            });
        }

        Ok(Self {
            channels,
            num_in_streams,
        })
    }

    pub fn channels(&self) -> NonZeroU32 {
        self.channels
    }

    pub fn num_in_streams(&self) -> NonZeroU32 {
        self.num_in_streams
    }
}

impl Default for MixNodeConfig {
    fn default() -> Self {
        Self {
            channels: NonZeroU32::new(2).unwrap(),
            num_in_streams: NonZeroU32::new(4).unwrap(),
        }
    }
}

impl AudioNodeConstructor for MixNodeConfig {
    fn debug_name(&self) -> &'static str {
        "mix"
    }

    fn channel_config(&self) -> ChannelConfig {
        let num_inputs =
            ChannelCount::new(self.channels.get() * self.num_in_streams.get()).unwrap();

        ChannelConfig {
            num_inputs,
            num_outputs: ChannelCount::new(self.channels.get()).unwrap(),
        }
    }

    fn uses_events(&self) -> bool {
        false
    }

    fn processor(&self, _stream_info: &firewheel_core::StreamInfo) -> Box<dyn AudioNodeProcessor> {
        Box::new(MixConfigProcessor {
            num_in_streams: self.channels.get() as usize,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MixConfigError {
    #[error("The number of channels times the number of input streams on a MixConfig cannot be greater than 64 (channels {channels}, num_in_streams: {num_in_streams}")]
    TooManyChannels {
        channels: NonZeroU32,
        num_in_streams: NonZeroU32,
    },
}

struct MixConfigProcessor {
    num_in_streams: usize,
}

impl AudioNodeProcessor for MixConfigProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _events: NodeEventList,
        proc_info: ProcInfo,
    ) -> ProcessStatus {
        let num_inputs = inputs.len();
        let num_outputs = outputs.len();
        let samples = proc_info.frames;

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All inputs are silent.
            return ProcessStatus::ClearAllOutputs;
        }

        if num_inputs == num_outputs {
            // No need to sum, just copy.
            for (out, input) in outputs.iter_mut().zip(inputs.iter()) {
                out[..samples].copy_from_slice(&input[..samples]);
            }

            return ProcessStatus::outputs_modified(proc_info.in_silence_mask);
        }

        match self.num_in_streams {
            // Provide a few optimized loops for common number of input streams.
            2 => {
                assert!(num_inputs >= (num_outputs * 2));

                for (ch_i, out) in outputs.iter_mut().enumerate() {
                    let in1 = &inputs[ch_i][..samples];
                    let in2 = &inputs[(num_outputs * 1) + ch_i][..samples];
                    let out = &mut out[0..samples];

                    for i in 0..samples {
                        out[i] = in1[i] + in2[i];
                    }
                }
            }
            3 => {
                assert!(num_inputs >= (num_outputs * 3));

                for (ch_i, out) in outputs.iter_mut().enumerate() {
                    let in1 = &inputs[ch_i][..samples];
                    let in2 = &inputs[(num_outputs * 1) + ch_i][..samples];
                    let in3 = &inputs[(num_outputs * 2) + ch_i][..samples];
                    let out = &mut out[0..samples];

                    for i in 0..samples {
                        out[i] = in1[i] + in2[i] + in3[i];
                    }
                }
            }
            4 => {
                assert!(num_inputs >= (num_outputs * 4));

                for (ch_i, out) in outputs.iter_mut().enumerate() {
                    let in1 = &inputs[ch_i][..samples];
                    let in2 = &inputs[(num_outputs * 1) + ch_i][..samples];
                    let in3 = &inputs[(num_outputs * 2) + ch_i][..samples];
                    let in4 = &inputs[(num_outputs * 3) + ch_i][..samples];
                    let out = &mut out[0..samples];

                    for i in 0..samples {
                        out[i] = in1[i] + in2[i] + in3[i] + in4[i];
                    }
                }
            }
            n => {
                assert!(num_inputs >= (num_outputs * n));

                for (ch_i, out) in outputs.iter_mut().enumerate() {
                    let out = &mut out[0..samples];

                    out.copy_from_slice(&inputs[ch_i][..samples]);

                    for in_port_i in 1..n {
                        let in_ch_i = (num_outputs * in_port_i) + ch_i;

                        if proc_info.in_silence_mask.is_channel_silent(in_ch_i) {
                            continue;
                        }

                        let input = &inputs[in_ch_i][..samples];

                        for i in 0..samples {
                            out[i] += input[i];
                        }
                    }
                }
            }
        }

        ProcessStatus::outputs_not_silent()
    }
}
