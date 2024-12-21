use std::{
    any::Any,
    collections::VecDeque,
    ops::Range,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use arrayvec::ArrayVec;
use atomic_float::AtomicF64;
use thunderdome::Arena;

use crate::graph::{NodeHeapData, ScheduleHeapData};
use firewheel_core::{
    clock::{ClockSamples, ClockSeconds, EventDelay},
    dsp::declick::DeclickValues,
    node::{
        AudioNodeProcessor, NodeEvent, NodeEventType, NodeID, ProcInfo, ProcessStatus,
        StreamStatus, NUM_SCRATCH_BUFFERS,
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
    clock_seconds_shared: Arc<AtomicF64>,
    clock_samples_shared: Arc<AtomicU64>,

    running: bool,
    stream_info: StreamInfo,
    sample_rate: f64,
    hard_clip_outputs: bool,

    scratch_buffers: Vec<f32>,
    declick_values: DeclickValues,
}

impl FirewheelProcessor {
    pub(crate) fn new(
        from_graph_rx: rtrb::Consumer<ContextToProcessorMsg>,
        to_graph_tx: rtrb::Producer<ProcessorToContextMsg>,
        clock_seconds_shared: Arc<AtomicF64>,
        clock_samples_shared: Arc<AtomicU64>,
        node_capacity: usize,
        stream_info: StreamInfo,
        hard_clip_outputs: bool,
    ) -> Self {
        let sample_rate = f64::from(stream_info.sample_rate.get());

        let mut scratch_buffers = Vec::new();
        scratch_buffers
            .reserve_exact(NUM_SCRATCH_BUFFERS * stream_info.max_block_frames.get() as usize);
        scratch_buffers.resize(
            NUM_SCRATCH_BUFFERS * stream_info.max_block_frames.get() as usize,
            0.0,
        );

        Self {
            nodes: Arena::with_capacity(node_capacity * 2),
            schedule_data: None,
            from_graph_rx,
            to_graph_tx,
            clock_samples: ClockSamples(0),
            clock_seconds_shared,
            clock_samples_shared,
            running: true,
            stream_info,
            sample_rate,
            hard_clip_outputs,
            scratch_buffers,
            declick_values: DeclickValues::new(stream_info.declick_frames),
        }
    }

    // TODO: Add a `process_deinterleaved` method.

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
        frames: usize,
        mut clock_seconds: ClockSeconds,
        stream_status: StreamStatus,
    ) -> FirewheelProcessorStatus {
        self.poll_messages(clock_seconds);

        let mut clock_samples = self.clock_samples;
        self.clock_samples += ClockSamples(frames as u64);

        self.clock_samples_shared
            .store(self.clock_samples.0, Ordering::Relaxed);
        self.clock_seconds_shared.store(
            clock_seconds.0 + (frames as f64 * self.stream_info.sample_rate_recip),
            Ordering::Relaxed,
        );

        if !self.running {
            output.fill(0.0);
            return FirewheelProcessorStatus::DropProcessor;
        }

        if self.schedule_data.is_none() || frames == 0 {
            output.fill(0.0);
            return FirewheelProcessorStatus::Ok;
        };

        assert_eq!(input.len(), frames * num_in_channels);
        assert_eq!(output.len(), frames * num_out_channels);

        let mut frames_processed = 0;
        while frames_processed < frames {
            let block_frames =
                (frames - frames_processed).min(self.stream_info.max_block_frames.get() as usize);

            // Prepare graph input buffers.
            self.schedule_data
                .as_mut()
                .unwrap()
                .schedule
                .prepare_graph_inputs(
                    block_frames,
                    num_in_channels,
                    |channels: &mut [&mut [f32]]| -> SilenceMask {
                        firewheel_core::dsp::interleave::deinterleave(
                            channels,
                            &input[frames_processed * num_in_channels
                                ..(frames_processed + block_frames) * num_in_channels],
                            num_in_channels,
                            true,
                        )
                    },
                );

            let next_clock_seconds = clock_seconds
                + ClockSeconds(block_frames as f64 * self.stream_info.sample_rate_recip);

            self.process_block(
                block_frames,
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
                    block_frames,
                    num_out_channels,
                    |channels: &[&[f32]], silence_mask| {
                        firewheel_core::dsp::interleave::interleave(
                            channels,
                            &mut output[frames_processed * num_out_channels
                                ..(frames_processed + block_frames) * num_out_channels],
                            num_out_channels,
                            Some(silence_mask),
                        );
                    },
                );

            if !self.running {
                if frames_processed < frames {
                    output[frames_processed * num_out_channels..].fill(0.0);
                }
                break;
            }

            frames_processed += block_frames;
            clock_samples += ClockSamples(block_frames as u64);
            clock_seconds = next_clock_seconds;
        }

        if self.hard_clip_outputs {
            for s in output.iter_mut() {
                *s = s.fract();
            }
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
                        new_schedule_data.schedule.max_block_frames(),
                        self.stream_info.max_block_frames.get() as usize
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
                ContextToProcessorMsg::HardClipOutputs(hard_clip_outputs) => {
                    self.hard_clip_outputs = hard_clip_outputs;
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
        block_frames: usize,
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
                } else if samples_after_now as usize >= block_frames {
                    break;
                } else {
                    node.num_delayed_events_this_block += 1;
                    // Since we have already done the calculation for what sample
                    // this event falls on, just store it into the seconds value.
                    *event_seconds = ClockSeconds(samples_after_now);
                }
            }
        }

        // Prepare scratch buffers.
        let mut scratch_buffers: [&mut [f32]; NUM_SCRATCH_BUFFERS] = std::array::from_fn(|i| {
            // SAFETY:
            //
            // * `self.scratch_buffers` was initialized with a length of
            // `NUM_SCRATCH_BUFFERS * max_block_frames` in the constructor.
            // * The resulting slices do not overlap.
            // * `self.scratch_buffers` is never written to or read from, so
            // it is safe to write to these slices.
            unsafe {
                std::slice::from_raw_parts_mut(
                    self.scratch_buffers
                        .as_ptr()
                        .add(i * self.stream_info.max_block_frames.get() as usize)
                        as *mut f32,
                    self.stream_info.max_block_frames.get() as usize,
                )
            }
        });

        schedule_data.schedule.process(
            block_frames,
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
                    &mut scratch_buffers,
                    &self.declick_values,
                    block_frames,
                    clock_samples,
                    clock_seconds.start,
                    stream_status,
                    self.stream_info.sample_rate_recip,
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

        let mut s = Vec::new();
        std::mem::swap(&mut s, &mut self.scratch_buffers);

        let _ = self.to_graph_tx.push(ProcessorToContextMsg::Dropped {
            nodes,
            _schedule_data: self.schedule_data.take(),
            _scratch_buffers: s,
        });
    }
}

pub(crate) enum ContextToProcessorMsg {
    EventGroup(Vec<NodeEvent>),
    NewSchedule(Box<ScheduleHeapData>),
    HardClipOutputs(bool),
    Stop,
}

pub(crate) enum ProcessorToContextMsg {
    ReturnCustomEvent(Box<dyn Any + Send>),
    ReturnEventGroup(Vec<NodeEvent>),
    ReturnSchedule(Box<ScheduleHeapData>),
    Dropped {
        nodes: Arena<NodeEntry>,
        _schedule_data: Option<Box<ScheduleHeapData>>,
        _scratch_buffers: Vec<f32>,
    },
}

fn process_node(
    node: &mut NodeEntry,
    to_graph_tx: &mut rtrb::Producer<ProcessorToContextMsg>,
    in_silence_mask: SilenceMask,
    out_silence_mask: SilenceMask,
    inputs: &[&[f32]],
    outputs: &mut [&mut [f32]],
    scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    declick_values: &DeclickValues,
    block_frames: usize,
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
                frames: block_frames,
                in_silence_mask,
                out_silence_mask,
                clock_samples,
                clock_seconds,
                stream_status,
                scratch_buffers,
                declick_values,
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
            scratch_buffers,
            declick_values,
            block_frames,
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
    scratch_buffers: &mut [&mut [f32]; NUM_SCRATCH_BUFFERS],
    declick_values: &DeclickValues,
    block_frames: usize,
    clock_samples: ClockSamples,
    clock_seconds: ClockSeconds,
    stream_status: StreamStatus,
    sample_rate_recip: f64,
) -> ProcessStatus {
    let mut new_out_silence_mask = SilenceMask::new_all_silent(outputs.len());

    let mut process_sub_block = |frames_processed: usize,
                                 sub_block_frames: usize,
                                 node: &mut NodeEntry| {
        let tmp_inputs: ArrayVec<&[f32], { ChannelCount::MAX.get() as usize }> = inputs
            .iter()
            .map(|b| &b[frames_processed..frames_processed + sub_block_frames])
            .collect();
        let mut tmp_outputs: ArrayVec<&mut [f32], { ChannelCount::MAX.get() as usize }> = outputs
            .iter_mut()
            .map(|b| &mut b[frames_processed..frames_processed + sub_block_frames])
            .collect();

        let status = node.processor.process(
            tmp_inputs.as_slice(),
            tmp_outputs.as_mut_slice(),
            node.immediate_event_queue.iter_mut(),
            ProcInfo {
                frames: sub_block_frames,
                in_silence_mask,
                out_silence_mask,
                clock_samples: clock_samples + ClockSamples(frames_processed as u64),
                clock_seconds: clock_seconds
                    + ClockSeconds(frames_processed as f64 * sample_rate_recip),
                stream_status,
                scratch_buffers,
                declick_values,
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
    let sub_block_frames = node.delayed_event_queue.front().unwrap().0 .0 as usize;
    let mut frames_processed = 0;
    process_sub_block(frames_processed, sub_block_frames, node);
    frames_processed = sub_block_frames;

    while frames_processed < block_frames {
        let mut num_events: u32 = 1;
        let mut sub_block_frames = block_frames - frames_processed;
        if node.num_delayed_events_this_block > 1 {
            // Note, this value has been replaced with the sample this event
            // falls on in `FirewheelProcessor::process_block`.
            for (samples_after_now, _) in node
                .delayed_event_queue
                .range(1..node.num_delayed_events_this_block as usize)
            {
                if samples_after_now.0 as usize > frames_processed {
                    sub_block_frames = samples_after_now.0 as usize - frames_processed;
                    break;
                }
                num_events += 1;
            }
        }

        for (_, event) in node.delayed_event_queue.drain(0..num_events as usize).rev() {
            node.immediate_event_queue.push_front(event);
        }

        process_sub_block(frames_processed, sub_block_frames, node);

        node.num_delayed_events_this_block -= num_events;
        frames_processed += sub_block_frames;
    }

    ProcessStatus::OutputsModified {
        out_silence_mask: new_out_silence_mask,
    }
}
