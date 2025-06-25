use firewheel_core::{
    channel_config::{ChannelConfig, NonZeroChannelCount},
    diff::{Diff, Patch},
    dsp::filter::{
        cascade::FilterCascadeUpTo,
        filter_trait::Filter,
        multi_channel_filter::{MultiChannelFilter, VecMultiChannelFilter},
        spec::{FilterSpec, DB_OCT_24},
    },
    event::NodeEventList,
    node::{
        AudioNode, AudioNodeInfo, AudioNodeProcessor, ConstructProcessorContext, ProcBuffers,
        ProcInfo, ProcessStatus,
    },
    SilenceMask,
};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct FlexibleChannelFilterNodeConfig {
    /// The number of input and output channels.
    pub channels: NonZeroChannelCount,
}

impl Default for FlexibleChannelFilterNodeConfig {
    fn default() -> Self {
        Self {
            channels: NonZeroChannelCount::STEREO,
        }
    }
}

#[derive(Default, Diff, Patch, Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
pub struct FlexibleChannelFilterNode<const MAX_ORDER: usize = DB_OCT_24> {
    /// Specifies the type of the filter. Changing this at runtime will redesign the filter accordingly.
    pub spec: FilterSpec,
}

impl<const MAX_ORDER: usize> AudioNode for FlexibleChannelFilterNode<MAX_ORDER> {
    type Configuration = FlexibleChannelFilterNodeConfig;

    fn info(&self, config: &Self::Configuration) -> AudioNodeInfo {
        let num_inputs = config.channels;
        let num_outputs = config.channels;
        AudioNodeInfo::new()
            .debug_name("volume")
            .channel_config(ChannelConfig {
                num_inputs: num_inputs.get(),
                num_outputs: num_outputs.get(),
            })
            .uses_events(true)
    }

    fn construct_processor(
        &self,
        config: &Self::Configuration,
        _cx: ConstructProcessorContext,
    ) -> impl AudioNodeProcessor {
        let mut result: FlexibleChannelFilterProcessor<MAX_ORDER> =
            FlexibleChannelFilterProcessor {
                filter: MultiChannelFilter::with_channels(config.channels),
                params: Default::default(),
                prev_block_was_silent: true,
            };
        result.design();
        result
    }
}

struct FlexibleChannelFilterProcessor<const MAX_ORDER: usize = DB_OCT_24> {
    filter: VecMultiChannelFilter<FilterCascadeUpTo<MAX_ORDER>>,
    params: FlexibleChannelFilterNode<MAX_ORDER>,
    prev_block_was_silent: bool,
}

impl<const MAX_ORDER: usize> FlexibleChannelFilterProcessor<MAX_ORDER> {
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

impl<const MAX_ORDER: usize> AudioNodeProcessor for FlexibleChannelFilterProcessor<MAX_ORDER> {
    fn process(
        &mut self,
        buffers: ProcBuffers,
        proc_info: &ProcInfo,
        mut events: NodeEventList,
    ) -> ProcessStatus {
        let mut updated = false;
        events.for_each_patch::<FlexibleChannelFilterNode<MAX_ORDER>>(|patch| {
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
