use firewheel_core::{
    node::{AudioNode, AudioNodeInfo, AudioNodeProcessor, ProcInfo, ProcessStatus},
    ChannelConfig, ChannelCount, StreamInfo,
};

pub struct SumNode;

impl<C> AudioNode<C> for SumNode {
    fn debug_name(&self) -> &'static str {
        "sum"
    }

    fn info(&self) -> AudioNodeInfo {
        AudioNodeInfo {
            num_min_supported_inputs: ChannelCount::MONO,
            num_max_supported_inputs: ChannelCount::MAX,
            num_min_supported_outputs: ChannelCount::MONO,
            num_max_supported_outputs: ChannelCount::MAX,
            default_channel_config: ChannelConfig {
                num_inputs: ChannelCount::new(4).unwrap(),
                num_outputs: ChannelCount::STEREO,
            },
            equal_num_ins_and_outs: false,
            updates: false,
        }
    }

    fn channel_config_supported(
        &self,
        channel_config: ChannelConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if channel_config.num_inputs.get() % channel_config.num_outputs.get() != 0 {
            Err(format!("The number of inputs on a SumNode must be a multiple of the number of outputs. Got config: {:?}", channel_config).into())
        } else {
            Ok(())
        }
    }

    fn activate(
        &mut self,
        _stream_info: &StreamInfo,
        channel_config: ChannelConfig,
    ) -> Result<Box<dyn AudioNodeProcessor<C>>, Box<dyn std::error::Error>> {
        assert!(channel_config.num_inputs.get() % channel_config.num_outputs.get() == 0);

        Ok(Box::new(SumNodeProcessor {
            num_in_ports: (channel_config.num_inputs.get() / channel_config.num_outputs.get())
                as usize,
        }))
    }
}

struct SumNodeProcessor {
    num_in_ports: usize,
}

impl<C> AudioNodeProcessor<C> for SumNodeProcessor {
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        proc_info: ProcInfo,
        _cx: &mut C,
    ) -> ProcessStatus {
        let num_inputs = inputs.len();
        let num_outputs = outputs.len();
        let samples = proc_info.samples;

        if proc_info.in_silence_mask.all_channels_silent(inputs.len()) {
            // All inputs are silent.
            return ProcessStatus::NoOutputsModified;
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

        ProcessStatus::all_outputs_filled()
    }
}

impl<C> Into<Box<dyn AudioNode<C>>> for SumNode {
    fn into(self) -> Box<dyn AudioNode<C>> {
        Box::new(self)
    }
}
