use std::num::NonZeroU32;

use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    node::{AudioNodeProcessor, NodeEventIter, NodeHandle, NodeID, ProcInfo, ProcessStatus},
};

use crate::FirewheelCtx;

pub struct MixNode {
    handle: NodeHandle,
}

#[derive(Debug, thiserror::Error)]
pub enum MixNodeError {
    #[error("The number of channels times the number of input streams on a MixNode cannot be greater than 64 (channels {channels}, num_in_streams: {num_in_streams}")]
    TooManyChannels {
        channels: NonZeroU32,
        num_in_streams: NonZeroU32,
    },
}

impl MixNode {
    pub fn new(
        channels: NonZeroU32,
        num_in_streams: NonZeroU32,
        cx: &mut FirewheelCtx,
    ) -> Result<Self, MixNodeError> {
        let num_inputs = ChannelCount::new(channels.get() * num_in_streams.get()).ok_or(
            MixNodeError::TooManyChannels {
                channels,
                num_in_streams,
            },
        )?;

        let handle = cx.add_node(
            "mix",
            ChannelConfig {
                num_inputs,
                num_outputs: ChannelCount::new(channels.get()).unwrap(),
            },
            false,
            Box::new(MixNodeProcessor {
                num_in_ports: channels.get() as usize,
            }),
        );

        Ok(Self { handle })
    }

    /// The ID of this node
    pub fn id(&self) -> NodeID {
        self.handle.id
    }
}

struct MixNodeProcessor {
    num_in_ports: usize,
}

impl AudioNodeProcessor for MixNodeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _events: NodeEventIter,
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

        match self.num_in_ports {
            // Provide a few optimized loops for common number of input ports.
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
