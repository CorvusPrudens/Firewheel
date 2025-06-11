use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::filter::{
        cascade::FilterCascadeUpTo, filter_trait::Filter, multi_channel_filter::MultiChannelFilter,
    },
    event::NodeEventList,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, ConstructProcessorContext, ProcBuffers,
        ProcInfo, ProcessStatus,
    },
    SilenceMask,
};

#[derive(Default, Diff, Patch, Debug, Clone, Copy, PartialEq)]
pub enum RejectionFilterType {
    #[default]
    Lowpass,
    Rejection,
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct RejectionFilterNodeConfig<const NUM_CHANNELS: usize>;

#[derive(Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct RejectionFilterNode<const NUM_CHANNELS: usize> {
    pub filter_type: RejectionFilterType,
    pub order: u32,
    pub cutoff_hz: f32,
    pub q: f32,
}

impl<const NUM_CHANNELS: usize> Default for RejectionFilterNode<NUM_CHANNELS> {
    fn default() -> Self {
        Self {
            filter_type: Default::default(),
            order: 2,
            cutoff_hz: 1.,
            q: 1.,
        }
    }
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
        cx: ConstructProcessorContext,
    ) -> impl AudioNodeProcessor {
        assert!((cx.stream_info.num_stream_in_channels as usize) < NUM_CHANNELS);

        let result: RejectionFilterProcessor<NUM_CHANNELS> = RejectionFilterProcessor {
            filter: Default::default(),
            params: Default::default(),
            prev_block_was_silent: true,
        };
        result
    }
}

struct RejectionFilterProcessor<const NUM_CHANNELS: usize> {
    filter: MultiChannelFilter<NUM_CHANNELS, FilterCascadeUpTo<16>>,
    params: RejectionFilterNode<NUM_CHANNELS>,
    prev_block_was_silent: bool,
}

impl<const NUM_CHANNELS: usize> AudioNodeProcessor for RejectionFilterProcessor<NUM_CHANNELS> {
    fn process(
        &mut self,
        buffers: ProcBuffers,
        proc_info: &ProcInfo,
        mut events: NodeEventList,
    ) -> ProcessStatus {
        let mut updated = false;
        events.for_each_patch::<RejectionFilterNode<NUM_CHANNELS>>(|patch| {
            self.params.apply(patch);
            updated = true;
        });
        if updated {
            match self.params.filter_type {
                RejectionFilterType::Lowpass => self.filter.lowpass(
                    self.params.order as usize,
                    self.params.cutoff_hz,
                    self.params.q,
                ),
                RejectionFilterType::Rejection => self.filter.highpass(
                    self.params.order as usize,
                    self.params.cutoff_hz,
                    self.params.q,
                ),
            }
        }

        self.prev_block_was_silent = false;

        if proc_info
            .in_silence_mask
            .all_channels_silent(buffers.inputs.len())
            && self.filter.is_silent()
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
                && self.filter.filters[ch_i].is_silent()
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
        self.filter.sample_rate_recip = stream_info.sample_rate_recip as f32;
    }
}
