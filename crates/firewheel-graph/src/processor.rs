use std::{ops::Range, sync::Arc};

use thunderdome::Arena;

use crate::graph::{NodeID, ScheduleHeapData};
use firewheel_core::{
    clock::{SampleTime, SampleTimeShared, SecondsShared},
    node::{AudioNodeProcessor, ProcInfo, ProcessStatus, StreamStatus},
    SilenceMask, StreamInfo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewheelProcessorStatus {
    Ok,
    /// If this is returned, then the [`FirewheelProcessor`] must be dropped.
    DropProcessor,
}

pub struct FirewheelProcessor<C: Send + 'static> {
    nodes: Arena<Box<dyn AudioNodeProcessor<C>>>,
    schedule_data: Option<Box<ScheduleHeapData<C>>>,
    user_cx: Option<C>,

    // TODO: Do research on whether `rtrb` is compatible with
    // webassembly. If not, use conditional compilation to
    // use a different channel type when targeting webassembly.
    from_graph_rx: rtrb::Consumer<ContextToProcessorMsg<C>>,
    to_graph_tx: rtrb::Producer<ProcessorToContextMsg<C>>,

    total_samples_processed_shared: Arc<SampleTimeShared>,
    event_time_secs_shared: Arc<SecondsShared>,

    running: bool,
    stream_info: StreamInfo,
    sample_rate_recip: f64,
}

impl<C: Send + 'static> FirewheelProcessor<C> {
    pub(crate) fn new(
        from_graph_rx: rtrb::Consumer<ContextToProcessorMsg<C>>,
        to_graph_tx: rtrb::Producer<ProcessorToContextMsg<C>>,
        total_samples_processed_shared: Arc<SampleTimeShared>,
        event_time_secs_shared: Arc<SecondsShared>,
        node_capacity: usize,
        stream_info: StreamInfo,
        user_cx: C,
    ) -> Self {
        let sample_rate_recip = f64::from(stream_info.sample_rate).recip();

        Self {
            nodes: Arena::with_capacity(node_capacity * 2),
            schedule_data: None,
            user_cx: Some(user_cx),
            from_graph_rx,
            to_graph_tx,
            total_samples_processed_shared,
            event_time_secs_shared,
            running: true,
            stream_info,
            sample_rate_recip,
        }
    }

    /// Process the given buffers of audio data.
    ///
    /// If this returns [`ProcessStatus::DropProcessor`], then this
    /// [`FirewheelProcessor`] must be dropped.
    pub fn process_interleaved(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        num_in_channels: usize,
        num_out_channels: usize,
        samples: usize,
        mut realtime_seconds: f64,
        stream_status: StreamStatus,
    ) -> FirewheelProcessorStatus {
        let mut total_samples_processed = self.total_samples_processed_shared.load();
        self.total_samples_processed_shared
            .store(total_samples_processed + SampleTime::new(samples as u64));
        self.event_time_secs_shared.store(realtime_seconds);

        self.poll_messages();

        if !self.running {
            output.fill(0.0);
            return FirewheelProcessorStatus::DropProcessor;
        }

        if self.schedule_data.is_none() || samples == 0 {
            output.fill(0.0);
            return FirewheelProcessorStatus::Ok;
        };

        assert_eq!(input.len(), samples * num_in_channels);
        assert_eq!(output.len(), samples * num_out_channels);

        let mut samples_processed = 0;
        while samples_processed < samples {
            let block_samples =
                (samples - samples_processed).min(self.stream_info.max_block_samples as usize);

            // Prepare graph input buffers.
            self.schedule_data
                .as_mut()
                .unwrap()
                .schedule
                .prepare_graph_inputs(
                    block_samples,
                    num_in_channels,
                    |channels: &mut [&mut [f32]]| -> SilenceMask {
                        firewheel_core::util::deinterleave(
                            channels,
                            &input[samples_processed * num_in_channels
                                ..(samples_processed + block_samples) * num_in_channels],
                            num_in_channels,
                            true,
                        )
                    },
                );

            let next_realtime_seconds =
                realtime_seconds + (block_samples as f64 * self.sample_rate_recip);

            self.process_block(
                block_samples,
                total_samples_processed,
                realtime_seconds..next_realtime_seconds,
                stream_status,
            );

            // Copy the output of the graph to the output buffer.
            self.schedule_data
                .as_mut()
                .unwrap()
                .schedule
                .read_graph_outputs(
                    block_samples,
                    num_out_channels,
                    |channels: &[&[f32]], silence_mask| {
                        firewheel_core::util::interleave(
                            channels,
                            &mut output[samples_processed * num_out_channels
                                ..(samples_processed + block_samples) * num_out_channels],
                            num_out_channels,
                            Some(silence_mask),
                        );
                    },
                );

            if !self.running {
                if samples_processed < samples {
                    output[samples_processed * num_out_channels..].fill(0.0);
                }
                break;
            }

            samples_processed += block_samples;
            total_samples_processed += SampleTime::new(block_samples as u64);
            realtime_seconds = next_realtime_seconds;
        }

        if self.running {
            FirewheelProcessorStatus::Ok
        } else {
            FirewheelProcessorStatus::DropProcessor
        }
    }

    fn poll_messages(&mut self) {
        while let Ok(msg) = self.from_graph_rx.pop() {
            match msg {
                ContextToProcessorMsg::NewSchedule(mut new_schedule_data) => {
                    assert_eq!(
                        new_schedule_data.schedule.max_block_samples(),
                        self.stream_info.max_block_samples as usize
                    );

                    if let Some(mut old_schedule_data) = self.schedule_data.take() {
                        std::mem::swap(
                            &mut old_schedule_data.removed_node_processors,
                            &mut new_schedule_data.removed_node_processors,
                        );

                        for node_id in new_schedule_data.nodes_to_remove.iter() {
                            if let Some(processor) = self.nodes.remove(node_id.idx) {
                                old_schedule_data
                                    .removed_node_processors
                                    .push((*node_id, processor));
                            }
                        }

                        self.to_graph_tx
                            .push(ProcessorToContextMsg::ReturnSchedule(old_schedule_data))
                            .unwrap();
                    }

                    for (node_id, processor) in new_schedule_data.new_node_processors.drain(..) {
                        assert!(self.nodes.insert_at(node_id.idx, processor).is_none());
                    }

                    self.schedule_data = Some(new_schedule_data);
                }
                ContextToProcessorMsg::Stop => {
                    self.running = false;
                }
            }
        }
    }

    fn process_block(
        &mut self,
        block_samples: usize,
        total_samples_processed: SampleTime,
        realtime_seconds: Range<f64>,
        stream_status: StreamStatus,
    ) {
        self.poll_messages();

        if !self.running {
            return;
        }

        let Some(schedule_data) = &mut self.schedule_data else {
            return;
        };

        let user_cx = self.user_cx.as_mut().unwrap();

        schedule_data.schedule.process(
            block_samples,
            |node_id: NodeID,
             in_silence_mask: SilenceMask,
             out_silence_mask: SilenceMask,
             inputs: &[&[f32]],
             outputs: &mut [&mut [f32]]|
             -> ProcessStatus {
                self.nodes[node_id.idx].process(
                    inputs,
                    outputs,
                    ProcInfo {
                        samples: block_samples,
                        in_silence_mask,
                        out_silence_mask,
                        total_samples_processed,
                        realtime_seconds: realtime_seconds.clone(),
                        stream_status,
                    },
                    user_cx,
                )
            },
        );
    }
}

impl<C: Send + 'static> Drop for FirewheelProcessor<C> {
    fn drop(&mut self) {
        // Make sure the nodes are not deallocated in the audio thread.
        let mut nodes = Arena::new();
        std::mem::swap(&mut nodes, &mut self.nodes);

        let _ = self.to_graph_tx.push(ProcessorToContextMsg::Dropped {
            nodes,
            _schedule_data: self.schedule_data.take(),
            user_cx: self.user_cx.take(),
        });
    }
}

pub(crate) enum ContextToProcessorMsg<C: Send + 'static> {
    NewSchedule(Box<ScheduleHeapData<C>>),
    Stop,
}

pub(crate) enum ProcessorToContextMsg<C: Send + 'static> {
    ReturnSchedule(Box<ScheduleHeapData<C>>),
    Dropped {
        nodes: Arena<Box<dyn AudioNodeProcessor<C>>>,
        _schedule_data: Option<Box<ScheduleHeapData<C>>>,
        user_cx: Option<C>,
    },
}
