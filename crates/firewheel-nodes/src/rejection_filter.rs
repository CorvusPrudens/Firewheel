use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::filter::{
        butterworth::Butterworth,
        cascade::FilterCascadeUpTo,
        filter_trait::{Filter, FilterBank},
        spec::SimpleResponseType,
    },
    event::NodeEventList,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, ConstructProcessorContext, ProcBuffers,
        ProcInfo, ProcessStatus,
    },
    SilenceMask,
};

#[derive(Default, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct RejectionFilterNodeConfig<const NUM_CHANNELS: usize>;

#[derive(Default, Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct RejectionFilterNode<const NUM_CHANNELS: usize> {
    pub cutoff: f32,
}

impl<const NUM_CHANNELS: usize> AudioNode for RejectionFilterNode<NUM_CHANNELS> {
    type Configuration = RejectionFilterNodeConfig<NUM_CHANNELS>;

    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
        // TODO: manage channel count better, this whole file is kind of a mess just to prototype
        let num_inputs = ChannelCount::new(NUM_CHANNELS as u32).unwrap();
        let num_outputs = num_inputs;
        AudioNodeInfo::new()
            .debug_name("volume")
            .channel_config(ChannelConfig {
                num_inputs,
                num_outputs,
            })
            .uses_events(true)
    }

    fn construct_processor(
        &self,
        _config: &Self::Configuration,
        _cx: ConstructProcessorContext,
    ) -> impl AudioNodeProcessor {
        // TODO: assert num_input channels < NUM_CHANNELS
        let result: RejectionFilterProcessor<NUM_CHANNELS> = RejectionFilterProcessor {
            filter: Default::default(),
            prev_block_was_silent: true,
        };
        result
    }
}

struct RejectionFilterProcessor<const NUM_CHANNELS: usize> {
    filter: FilterBank<NUM_CHANNELS, FilterCascadeUpTo<16>>,
    prev_block_was_silent: bool,
}

impl<const NUM_CHANNELS: usize> AudioNodeProcessor for RejectionFilterProcessor<NUM_CHANNELS> {
    fn process(
        &mut self,
        buffers: ProcBuffers,
        proc_info: &ProcInfo,
        mut events: NodeEventList,
    ) -> ProcessStatus {
        events.for_each_patch::<RejectionFilterNode<NUM_CHANNELS>>(
            |RejectionFilterNodePatch::Cutoff(c)| {
                self.filter.design_butterworth(
                    SimpleResponseType::Lowpass,
                    c,
                    self.filter.sample_rate,
                    self.filter.order,
                );
            },
        );

        self.prev_block_was_silent = false;

        // TODO: think about what value the silence threshold should actually be (arbitrarily picked some low number for now)
        let silence_threshold = 0.000_000_1;
        if proc_info
            .in_silence_mask
            .all_channels_silent(buffers.inputs.len())
            && self.filter.is_silent(silence_threshold)
        {
            self.prev_block_was_silent = true;

            return ProcessStatus::ClearAllOutputs;
        }

        let mut output_silence_mask = SilenceMask::new_all_silent(buffers.inputs.len());
        for (ch_i, (out_ch, in_ch)) in buffers
            .outputs
            .iter_mut()
            .zip(buffers.inputs.iter())
            .enumerate()
        {
            if proc_info.in_silence_mask.is_channel_silent(ch_i)
                && self.filter.filters[ch_i].is_silent(silence_threshold)
            {
                if !proc_info.out_silence_mask.is_channel_silent(ch_i) {
                    out_ch.fill(0.0);
                }
            } else {
                output_silence_mask.set_channel(ch_i, false);
                for (os, &is) in out_ch.iter_mut().zip(in_ch.iter()) {
                    *os = self.filter.process(is, ch_i);
                }
            }
        }

        return ProcessStatus::OutputsModified {
            out_silence_mask: output_silence_mask,
        };
    }

    fn new_stream(&mut self, stream_info: &firewheel_core::StreamInfo) {
        // TODO: make this more ergonomic. filters should automatically redesign themselves when a relevant parameter changes
        self.filter.sample_rate = stream_info.sample_rate;
        self.filter.design_butterworth(
            SimpleResponseType::Lowpass,
            self.filter.cutoff_hz,
            self.filter.sample_rate,
            self.filter.order,
        );
    }
}
