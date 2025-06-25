use firewheel_core::{
    channel_config::{ChannelConfig, ChannelCount},
    diff::{Diff, Patch},
    dsp::filter::{
        cascade::FilterCascadeUpTo,
        filter_trait::Filter,
        multi_channel_filter::ArrayMultiChannelFilter,
        spec::{FilterSpec, DB_OCT_24},
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
pub struct ConstChannelFilterNodeConfig<const NUM_CHANNELS: usize>;

#[derive(Default, Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct ConstChannelFilterNode<const NUM_CHANNELS: usize, const MAX_ORDER: usize = DB_OCT_24> {
    /// Specifies the type and parameters of the filter. Changing this at runtime will redesign the filter accordingly.
    pub spec: FilterSpec,
}

impl<const NUM_CHANNELS: usize, const MAX_ORDER: usize> AudioNode
    for ConstChannelFilterNode<NUM_CHANNELS, MAX_ORDER>
{
    type Configuration = ConstChannelFilterNodeConfig<NUM_CHANNELS>;

    fn info(&self, _config: &Self::Configuration) -> AudioNodeInfo {
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
        assert!((cx.stream_info.num_stream_in_channels as usize) <= NUM_CHANNELS);

        let mut result: ConstChannelFilterProcessor<NUM_CHANNELS, MAX_ORDER> =
            ConstChannelFilterProcessor {
                filter: Default::default(),
                params: Default::default(),
                prev_block_was_silent: true,
            };
        result.design();
        result
    }
}

struct ConstChannelFilterProcessor<const NUM_CHANNELS: usize, const MAX_ORDER: usize> {
    filter: ArrayMultiChannelFilter<NUM_CHANNELS, FilterCascadeUpTo<MAX_ORDER>>,
    params: ConstChannelFilterNode<NUM_CHANNELS, MAX_ORDER>,
    prev_block_was_silent: bool,
}

impl<const NUM_CHANNELS: usize, const MAX_ORDER: usize>
    ConstChannelFilterProcessor<NUM_CHANNELS, MAX_ORDER>
{
    fn design(&mut self) {
        match self.params.spec {
            FilterSpec::Lowpass {
                order,
                cutoff_hz,
                q,
            } => self.filter.lowpass(order, cutoff_hz, q),
            FilterSpec::Highpass {
                order,
                cutoff_hz,
                q,
            } => self.filter.highpass(order, cutoff_hz, q),
            FilterSpec::Bandpass { cutoff_hz, q } => self.filter.bandpass(cutoff_hz, q),
            FilterSpec::Allpass { cutoff_hz, q } => self.filter.allpass(cutoff_hz, q),
            FilterSpec::Bell {
                center_hz,
                q,
                gain_db,
            } => self.filter.bell(center_hz, q, gain_db),
            FilterSpec::LowShelf {
                cutoff_hz,
                q,
                gain_db,
            } => self.filter.low_shelf(cutoff_hz, q, gain_db),
            FilterSpec::HighShelf {
                cutoff_hz,
                q,
                gain_db,
            } => self.filter.high_shelf(cutoff_hz, q, gain_db),
            FilterSpec::Notch { center_hz, q } => self.filter.notch(center_hz, q),
        }
    }
}

impl<const NUM_CHANNELS: usize, const MAX_ORDER: usize> AudioNodeProcessor
    for ConstChannelFilterProcessor<NUM_CHANNELS, MAX_ORDER>
{
    fn process(
        &mut self,
        buffers: ProcBuffers,
        proc_info: &ProcInfo,
        mut events: NodeEventList,
    ) -> ProcessStatus {
        let mut updated = false;
        events.for_each_patch::<ConstChannelFilterNode<NUM_CHANNELS, MAX_ORDER>>(|patch| {
            self.params.apply(patch);
            updated = true;
        });
        if updated {
            self.design();
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
