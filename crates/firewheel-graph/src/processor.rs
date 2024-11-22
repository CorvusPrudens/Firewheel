use std::{any::Any, collections::VecDeque, ops::Range, time::Instant};

use arrayvec::ArrayVec;
use thunderdome::Arena;

use crate::graph::{NodeHeapData, ScheduleHeapData};
use firewheel_core::{
    clock::{ClockSamples, ClockSeconds, EventDelay},
    node::{
        AudioNodeProcessor, NodeEvent, NodeEventType, NodeID, ProcInfo, ProcessStatus, StreamStatus,
    },
    ChannelCount, SilenceMask, StreamInfo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewheelProcessorStatus {
    Ok,
    /// If this is returned, then the [`FirewheelProcessor`] must be dropped.
    DropProcessor,
}

pub(crate) struct NodeEntry {
    pub processor: Box<dyn AudioNodeProcessor>,
    immediate_event_queue: VecDeque<NodeEventType>,
    delayed_event_queue: VecDeque<(ClockSeconds, NodeEventType)>,
    num_delayed_events_this_block: u32,
    num_delayed_events_this_block_first_sample: u32,
    paused_at_seconds: Option<ClockSeconds>,
}

pub struct FirewheelProcessor {
    nodes: Arena<NodeEntry>,
    schedule_data: Option<Box<ScheduleHeapData>>,

    from_graph_rx: rtrb::Consumer<ContextToProcessorMsg>,
    to_graph_tx: rtrb::Producer<ProcessorToContextMsg>,

    clock_samples: ClockSamples,
    main_thread_clock_start_instant: Instant,
    main_to_internal_clock_offset: Option<ClockSeconds>,

    running: bool,
    stream_info: StreamInfo,
    sample_rate: f64,
    sample_rate_recip: f64,
}

impl FirewheelProcessor {
    pub(crate) fn new(
        from_graph_rx: rtrb::Consumer<ContextToProcessorMsg>,
        to_graph_tx: rtrb::Producer<ProcessorToContextMsg>,
        main_thread_clock_start_instant: Instant,
        node_capacity: usize,
        stream_info: StreamInfo,
    ) -> Self {
        let sample_rate = f64::from(stream_info.sample_rate);
        let sample_rate_recip = sample_rate.recip();

        Self {
            nodes: Arena::with_capacity(node_capacity * 2),
            schedule_data: None,
            from_graph_rx,
            to_graph_tx,
            clock_samples: ClockSamples(0),
            main_thread_clock_start_instant,
            main_to_internal_clock_offset: None,
            running: true,
            stream_info,
            sample_rate,
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
        internal_clock_seconds: ClockSeconds,
        stream_status: StreamStatus,
    ) -> FirewheelProcessorStatus {
        let mut clock_samples = self.clock_samples;
        self.clock_samples += ClockSamples(samples as u64);

        // If this is the first block, calculate the offset between the the main thread's
        // clock and the internal realtime clock.
        //
        // main_thread_clock_seconds - internal_clock_seconds =
        //    (first_block_instant - main_thread_clock_start_instant)
        //    - first_block_internal_clock_seconds
        let main_to_internal_clock_offset =
            *self.main_to_internal_clock_offset.get_or_insert_with(|| {
                ClockSeconds(
                    (Instant::now() - self.main_thread_clock_start_instant).as_secs_f64()
                        - internal_clock_seconds.0,
                )
            });
        // Offset the internal clock so it matches the main thread clock.
        let mut clock_seconds = internal_clock_seconds + main_to_internal_clock_offset;

        self.poll_messages(clock_seconds);

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

            let next_clock_seconds =
                clock_seconds + ClockSeconds(block_samples as f64 * self.sample_rate_recip);

            self.process_block(
                block_samples,
                clock_samples,
                clock_seconds..next_clock_seconds,
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
            clock_samples += ClockSamples(block_samples as u64);
            clock_seconds = next_clock_seconds;
        }

        if self.running {
            FirewheelProcessorStatus::Ok
        } else {
            FirewheelProcessorStatus::DropProcessor
        }
    }

    fn poll_messages(&mut self, clock_seconds: ClockSeconds) {
        while let Ok(msg) = self.from_graph_rx.pop() {
            match msg {
                ContextToProcessorMsg::EventGroup(mut events) => {
                    for event in events.drain(..) {
                        self.queue_new_event(event, clock_seconds);
                    }

                    let _ = self
                        .to_graph_tx
                        .push(ProcessorToContextMsg::ReturnEventGroup(events));
                }
                ContextToProcessorMsg::NewSchedule(mut new_schedule_data) => {
                    assert_eq!(
                        new_schedule_data.schedule.max_block_samples(),
                        self.stream_info.max_block_samples as usize
                    );

                    if let Some(mut old_schedule_data) = self.schedule_data.take() {
                        std::mem::swap(
                            &mut old_schedule_data.removed_nodes,
                            &mut new_schedule_data.removed_nodes,
                        );

                        for node_id in new_schedule_data.nodes_to_remove.iter() {
                            if let Some(node_entry) = self.nodes.remove(node_id.idx) {
                                old_schedule_data.removed_nodes.push(NodeHeapData {
                                    id: *node_id,
                                    processor: node_entry.processor,
                                    immediate_event_queue: node_entry.immediate_event_queue,
                                    delayed_event_queue: node_entry.delayed_event_queue,
                                });
                            }
                        }

                        self.to_graph_tx
                            .push(ProcessorToContextMsg::ReturnSchedule(old_schedule_data))
                            .unwrap();
                    }

                    for n in new_schedule_data.new_node_processors.drain(..) {
                        assert!(self
                            .nodes
                            .insert_at(
                                n.id.idx,
                                NodeEntry {
                                    processor: n.processor,
                                    immediate_event_queue: n.immediate_event_queue,
                                    delayed_event_queue: n.delayed_event_queue,
                                    num_delayed_events_this_block: 0,
                                    num_delayed_events_this_block_first_sample: 0,
                                    paused_at_seconds: None,
                                }
                            )
                            .is_none());
                    }

                    self.schedule_data = Some(new_schedule_data);
                }
                ContextToProcessorMsg::Stop => {
                    self.running = false;
                }
            }
        }
    }

    fn queue_new_event(&mut self, event: NodeEvent, clock_seconds: ClockSeconds) {
        let Some(node_entry) = self.nodes.get_mut(event.node_id.idx) else {
            if let NodeEventType::Custom(event) = event.event {
                let _ = self
                    .to_graph_tx
                    .push(ProcessorToContextMsg::ReturnCustomEvent(event));
            }
            return;
        };

        match &event.event {
            NodeEventType::Pause => {
                if node_entry.paused_at_seconds.is_none() {
                    node_entry.paused_at_seconds = Some(clock_seconds);
                }

                node_entry.immediate_event_queue.push_back(event.event);

                return;
            }
            NodeEventType::Resume => {
                if let Some(paused_seconds) = node_entry.paused_at_seconds.take() {
                    let offset = clock_seconds - paused_seconds;

                    for (event_clock_seconds, _) in node_entry.delayed_event_queue.iter_mut() {
                        *event_clock_seconds += offset;
                    }
                }

                node_entry.immediate_event_queue.push_back(event.event);

                return;
            }
            NodeEventType::Stop => {
                for (_, event) in node_entry.delayed_event_queue.drain(..) {
                    if let NodeEventType::Custom(event) = event {
                        let _ = self
                            .to_graph_tx
                            .push(ProcessorToContextMsg::ReturnCustomEvent(event));
                    }
                }

                node_entry.immediate_event_queue.push_back(event.event);

                return;
            }
            _ => {}
        }

        match event.delay {
            EventDelay::Immediate => {
                node_entry.immediate_event_queue.push_back(event.event);
            }
            EventDelay::DelayUntilSeconds(seconds) => {
                if seconds <= clock_seconds {
                    // Assume that delayed events should happen before immediate
                    // events.
                    node_entry.immediate_event_queue.push_front(event.event);
                } else if node_entry.delayed_event_queue.is_empty() {
                    node_entry
                        .delayed_event_queue
                        .push_back((seconds, event.event));
                } else if seconds >= node_entry.delayed_event_queue.back().unwrap().0 {
                    node_entry
                        .delayed_event_queue
                        .push_back((seconds, event.event));
                } else if seconds <= node_entry.delayed_event_queue.front().unwrap().0 {
                    node_entry
                        .delayed_event_queue
                        .push_front((seconds, event.event));
                } else {
                    let i = node_entry
                        .delayed_event_queue
                        .iter()
                        .enumerate()
                        .find_map(|(i, (s, _))| if seconds <= *s { Some(i) } else { None })
                        .unwrap();

                    node_entry
                        .delayed_event_queue
                        .insert(i, (seconds, event.event));
                }
            }
        }
    }

    fn process_block(
        &mut self,
        block_samples: usize,
        clock_samples: ClockSamples,
        clock_seconds: Range<ClockSeconds>,
        stream_status: StreamStatus,
    ) {
        if !self.running {
            return;
        }

        let Some(schedule_data) = &mut self.schedule_data else {
            return;
        };

        // Find delayed events which have elapsed.
        for (_, node) in self.nodes.iter_mut() {
            node.num_delayed_events_this_block = 0;
            node.num_delayed_events_this_block_first_sample = 0;

            if node.paused_at_seconds.is_some() {
                continue;
            }

            for (event_seconds, _) in node.delayed_event_queue.iter_mut() {
                if *event_seconds >= clock_seconds.end {
                    break;
                }

                let seconds_after_now = event_seconds.0 - clock_seconds.start.0;
                let samples_after_now = (seconds_after_now * self.sample_rate).round();

                if samples_after_now <= 0.0 {
                    node.num_delayed_events_this_block_first_sample += 1;
                } else if samples_after_now as usize >= block_samples {
                    break;
                } else {
                    node.num_delayed_events_this_block += 1;
                    // Since we have already done the calculation for what sample
                    // this event falls on, just store it into the seconds value.
                    *event_seconds = ClockSeconds(samples_after_now);
                }
            }
        }

        schedule_data.schedule.process(
            block_samples,
            |node_id: NodeID,
             in_silence_mask: SilenceMask,
             out_silence_mask: SilenceMask,
             inputs: &[&[f32]],
             outputs: &mut [&mut [f32]]|
             -> ProcessStatus {
                process_node(
                    &mut self.nodes[node_id.idx],
                    &mut self.to_graph_tx,
                    in_silence_mask,
                    out_silence_mask,
                    inputs,
                    outputs,
                    block_samples,
                    clock_samples,
                    clock_seconds.start,
                    stream_status,
                    self.sample_rate_recip,
                )
            },
        );
    }
}

impl Drop for FirewheelProcessor {
    fn drop(&mut self) {
        // Make sure the nodes are not deallocated in the audio thread.
        let mut nodes = Arena::new();
        std::mem::swap(&mut nodes, &mut self.nodes);

        let _ = self.to_graph_tx.push(ProcessorToContextMsg::Dropped {
            nodes,
            _schedule_data: self.schedule_data.take(),
        });
    }
}

pub(crate) enum ContextToProcessorMsg {
    EventGroup(Vec<NodeEvent>),
    NewSchedule(Box<ScheduleHeapData>),
    Stop,
}

pub(crate) enum ProcessorToContextMsg {
    ReturnCustomEvent(Box<dyn Any + Send>),
    ReturnEventGroup(Vec<NodeEvent>),
    ReturnSchedule(Box<ScheduleHeapData>),
    Dropped {
        nodes: Arena<NodeEntry>,
        _schedule_data: Option<Box<ScheduleHeapData>>,
    },
}

fn process_node(
    node: &mut NodeEntry,
    to_graph_tx: &mut rtrb::Producer<ProcessorToContextMsg>,
    in_silence_mask: SilenceMask,
    out_silence_mask: SilenceMask,
    inputs: &[&[f32]],
    outputs: &mut [&mut [f32]],
    block_samples: usize,
    clock_samples: ClockSamples,
    clock_seconds: ClockSeconds,
    stream_status: StreamStatus,
    sample_rate_recip: f64,
) -> ProcessStatus {
    // Queue events which should happen at the first sample in this block.
    if node.num_delayed_events_this_block_first_sample > 0 {
        for (_, event) in node
            .delayed_event_queue
            .drain(0..node.num_delayed_events_this_block_first_sample as usize)
            .rev()
        {
            node.immediate_event_queue.push_front(event);
        }
    }

    let status = if node.num_delayed_events_this_block == 0 {
        let status = node.processor.process(
            inputs,
            outputs,
            node.immediate_event_queue.iter_mut(),
            ProcInfo {
                samples: block_samples,
                in_silence_mask,
                out_silence_mask,
                clock_samples,
                clock_seconds,
                stream_status,
            },
        );

        // Cleanup events
        for event in node.immediate_event_queue.drain(..) {
            if let NodeEventType::Custom(event) = event {
                let _ = to_graph_tx.push(ProcessorToContextMsg::ReturnCustomEvent(event));
            }
        }

        status
    } else {
        // Process in sub-blocks
        process_node_sub_blocks(
            node,
            to_graph_tx,
            in_silence_mask,
            out_silence_mask,
            inputs,
            outputs,
            block_samples,
            clock_samples,
            clock_seconds,
            stream_status,
            sample_rate_recip,
        )
    };

    status
}

fn process_node_sub_blocks(
    node: &mut NodeEntry,
    to_graph_tx: &mut rtrb::Producer<ProcessorToContextMsg>,
    in_silence_mask: SilenceMask,
    out_silence_mask: SilenceMask,
    inputs: &[&[f32]],
    outputs: &mut [&mut [f32]],
    block_samples: usize,
    clock_samples: ClockSamples,
    clock_seconds: ClockSeconds,
    stream_status: StreamStatus,
    sample_rate_recip: f64,
) -> ProcessStatus {
    let mut new_out_silence_mask = SilenceMask::new_all_silent(outputs.len());

    let mut process_sub_block = |samples_processed: usize,
                                 sub_block_samples: usize,
                                 node: &mut NodeEntry| {
        let tmp_inputs: ArrayVec<&[f32], { ChannelCount::MAX.get() as usize }> = inputs
            .iter()
            .map(|b| &b[samples_processed..samples_processed + sub_block_samples])
            .collect();
        let mut tmp_outputs: ArrayVec<&mut [f32], { ChannelCount::MAX.get() as usize }> = outputs
            .iter_mut()
            .map(|b| &mut b[samples_processed..samples_processed + sub_block_samples])
            .collect();

        let status = node.processor.process(
            tmp_inputs.as_slice(),
            tmp_outputs.as_mut_slice(),
            node.immediate_event_queue.iter_mut(),
            ProcInfo {
                samples: sub_block_samples,
                in_silence_mask,
                out_silence_mask,
                clock_samples: clock_samples + ClockSamples(samples_processed as u64),
                clock_seconds: clock_seconds
                    + ClockSeconds(samples_processed as f64 * sample_rate_recip),
                stream_status,
            },
        );

        match status {
            ProcessStatus::ClearAllOutputs => {
                // Clear output buffers which need cleared.
                for (i, b) in tmp_outputs.iter_mut().enumerate() {
                    if !out_silence_mask.is_channel_silent(i) {
                        b.fill(0.0);
                    }
                }
            }
            ProcessStatus::Bypass => {
                for (i, (in_buf, out_buf)) in
                    tmp_inputs.iter().zip(tmp_outputs.iter_mut()).enumerate()
                {
                    let in_s = in_silence_mask.is_channel_silent(i);
                    let out_s = out_silence_mask.is_channel_silent(i);

                    if in_s {
                        if !out_s {
                            out_buf.fill(0.0);
                        }
                    } else {
                        out_buf.copy_from_slice(in_buf);
                        new_out_silence_mask.set_channel(i, false);
                    }
                }

                if tmp_outputs.len() > tmp_inputs.len() {
                    for (i, out_buf) in tmp_outputs.iter_mut().enumerate().skip(tmp_inputs.len()) {
                        if !out_silence_mask.is_channel_silent(i) {
                            out_buf.fill(0.0);
                        }
                    }
                }
            }
            ProcessStatus::OutputsModified { out_silence_mask } => {
                new_out_silence_mask.0 &= out_silence_mask.0;
            }
        }

        // Cleanup events
        for event in node.immediate_event_queue.drain(..) {
            if let NodeEventType::Custom(event) = event {
                let _ = to_graph_tx.push(ProcessorToContextMsg::ReturnCustomEvent(event));
            }
        }
    };

    // Process the first sub-block.
    // Note, this value has been replaced with the sample this event
    // falls on in `FirewheelProcessor::process_block`.
    let sub_block_samples = node.delayed_event_queue.front().unwrap().0 .0 as usize;
    let mut samples_processed = 0;
    process_sub_block(samples_processed, sub_block_samples, node);
    samples_processed = sub_block_samples;

    while samples_processed < block_samples {
        let mut num_events: u32 = 1;
        let mut sub_block_samples = block_samples - samples_processed;
        if node.num_delayed_events_this_block > 1 {
            // Note, this value has been replaced with the sample this event
            // falls on in `FirewheelProcessor::process_block`.
            for (samples_after_now, _) in node
                .delayed_event_queue
                .range(1..node.num_delayed_events_this_block as usize)
            {
                if samples_after_now.0 as usize > samples_processed {
                    sub_block_samples = samples_after_now.0 as usize - samples_processed;
                    break;
                }
                num_events += 1;
            }
        }

        for (_, event) in node.delayed_event_queue.drain(0..num_events as usize).rev() {
            node.immediate_event_queue.push_front(event);
        }

        process_sub_block(samples_processed, sub_block_samples, node);

        node.num_delayed_events_this_block -= num_events;
        samples_processed += sub_block_samples;
    }

    ProcessStatus::OutputsModified {
        out_silence_mask: new_out_silence_mask,
    }
}
