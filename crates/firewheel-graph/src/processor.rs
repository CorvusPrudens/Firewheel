use std::{
    num::NonZeroU32,
    ops::Range,
    sync::{atomic::Ordering, Arc},
};

use thunderdome::Arena;

use crate::{
    context::ClockValues,
    graph::{NodeHeapData, ScheduleHeapData},
};
use firewheel_core::{
    clock::{ClockSamples, ClockSeconds, MusicalTime, MusicalTransport},
    dsp::{buffer::ChannelBuffer, declick::DeclickValues},
    event::{NodeEvent, NodeEventList},
    node::{
        AudioNodeProcessor, NodeID, ProcInfo, ProcessStatus, StreamStatus, TransportInfo,
        NUM_SCRATCH_BUFFERS,
    },
    SilenceMask, StreamInfo,
};

pub struct FirewheelProcessor {
    inner: Option<FirewheelProcessorInner>,
    drop_tx: rtrb::Producer<FirewheelProcessorInner>,
}

impl Drop for FirewheelProcessor {
    fn drop(&mut self) {
        let Some(mut inner) = self.inner.take() else {
            return;
        };

        inner.stream_stopped();

        if std::thread::panicking() {
            inner.poisoned = true;
        }

        let _ = self.drop_tx.push(inner);
    }
}

impl FirewheelProcessor {
    pub(crate) fn new(
        processor: FirewheelProcessorInner,
        drop_tx: rtrb::Producer<FirewheelProcessorInner>,
    ) -> Self {
        Self {
            inner: Some(processor),
            drop_tx,
        }
    }

    pub fn process_interleaved(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        num_in_channels: usize,
        num_out_channels: usize,
        frames: usize,
        clock_seconds: ClockSeconds,
        stream_status: StreamStatus,
    ) {
        if let Some(inner) = &mut self.inner {
            inner.process_interleaved(
                input,
                output,
                num_in_channels,
                num_out_channels,
                frames,
                clock_seconds,
                stream_status,
            );
        }
    }
}

pub(crate) struct FirewheelProcessorInner {
    nodes: Arena<NodeEntry>,
    schedule_data: Option<Box<ScheduleHeapData>>,

    from_graph_rx: rtrb::Consumer<ContextToProcessorMsg>,
    to_graph_tx: rtrb::Producer<ProcessorToContextMsg>,
    event_buffer: Vec<NodeEvent>,

    sample_rate: NonZeroU32,
    sample_rate_recip: f64,
    max_block_frames: usize,

    clock_samples: ClockSamples,
    clock_shared: Arc<ClockValues>,

    last_clock_seconds: ClockSeconds,
    clock_seconds_offset: f64,
    is_new_stream: bool,

    hard_clip_outputs: bool,

    scratch_buffers: ChannelBuffer<f32, NUM_SCRATCH_BUFFERS>,
    declick_values: DeclickValues,

    transport: Option<TransportState>,

    /// If a panic occurs while processing, this flag is set to let the
    /// main thread know that it shouldn't try spawning a new audio stream
    /// with the shared `Arc<AtomicRefCell<FirewheelProcessorInner>>` object.
    pub(crate) poisoned: bool,
}

impl FirewheelProcessorInner {
    /// Note, this method gets called on the main thread, not the audio thread.
    pub(crate) fn new(
        from_graph_rx: rtrb::Consumer<ContextToProcessorMsg>,
        to_graph_tx: rtrb::Producer<ProcessorToContextMsg>,
        clock_shared: Arc<ClockValues>,
        node_capacity: usize,
        stream_info: &StreamInfo,
        hard_clip_outputs: bool,
    ) -> Self {
        Self {
            nodes: Arena::with_capacity(node_capacity * 2),
            schedule_data: None,
            from_graph_rx,
            to_graph_tx,
            event_buffer: Vec::new(),
            sample_rate: stream_info.sample_rate,
            sample_rate_recip: stream_info.sample_rate_recip,
            max_block_frames: stream_info.max_block_frames.get() as usize,
            clock_samples: ClockSamples(0),
            clock_shared,
            last_clock_seconds: ClockSeconds(0.0),
            clock_seconds_offset: 0.0,
            is_new_stream: false,
            //running: true,
            hard_clip_outputs,
            scratch_buffers: ChannelBuffer::new(stream_info.max_block_frames.get() as usize),
            declick_values: DeclickValues::new(stream_info.declick_frames),
            transport: None,
            poisoned: false,
        }
    }

    fn stream_stopped(&mut self) {
        for (_, node) in self.nodes.iter_mut() {
            node.processor.stream_stopped();
        }
    }

    /// Called when a new audio stream has been started to replace the old one.
    ///
    /// Note, this method gets called on the main thread, not the audio thread.
    pub fn new_stream(&mut self, stream_info: &StreamInfo) {
        for (_, node) in self.nodes.iter_mut() {
            node.processor.new_stream(stream_info);
        }

        if self.sample_rate != stream_info.sample_rate {
            self.sample_rate = stream_info.sample_rate;
            self.sample_rate_recip = stream_info.sample_rate_recip;

            self.declick_values = DeclickValues::new(stream_info.declick_frames);
        }

        if self.max_block_frames != stream_info.max_block_frames.get() as usize {
            self.max_block_frames = stream_info.max_block_frames.get() as usize;

            self.scratch_buffers = ChannelBuffer::new(stream_info.max_block_frames.get() as usize);
        }

        self.is_new_stream = true;
    }

    // TODO: Add a `process_deinterleaved` method.

    /// Process the given buffers of audio data.
    ///
    /// If this returns [`ProcessStatus::DropProcessor`], then this
    /// [`FirewheelProcessorInner`] must be dropped.
    pub fn process_interleaved(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        num_in_channels: usize,
        num_out_channels: usize,
        frames: usize,
        clock_seconds: ClockSeconds,
        stream_status: StreamStatus,
    ) {
        self.poll_messages();

        let mut clock_samples = self.clock_samples;
        self.clock_samples += ClockSamples(frames as i64);
        self.clock_shared
            .samples
            .store(self.clock_samples.0, Ordering::Relaxed);

        if self.is_new_stream {
            self.is_new_stream = false;

            // Apply an offset so that the clock appears to be steady for nodes.
            self.clock_seconds_offset = self.last_clock_seconds.0 - clock_seconds.0;
        }

        let mut clock_seconds = ClockSeconds(clock_seconds.0 + self.clock_seconds_offset);
        self.last_clock_seconds =
            ClockSeconds(clock_seconds.0 + (frames as f64 * self.sample_rate_recip));
        self.clock_shared
            .seconds
            .store(self.last_clock_seconds.0, Ordering::Relaxed);

        if let Some(transport) = &self.transport {
            if !transport.stopped && !transport.paused {
                self.clock_shared.musical.store(
                    transport
                        .transport
                        .sample_to_musical(
                            self.clock_samples - transport.paused_at_frame,
                            self.sample_rate.get(),
                            self.sample_rate_recip,
                        )
                        .sub_beats,
                    Ordering::Relaxed,
                );
            }
        }

        if self.schedule_data.is_none() || frames == 0 {
            output.fill(0.0);
            //return FirewheelProcessorInnerStatus::Ok;
            return;
        };

        assert_eq!(input.len(), frames * num_in_channels);
        assert_eq!(output.len(), frames * num_out_channels);

        let mut frames_processed = 0;
        while frames_processed < frames {
            let block_frames = (frames - frames_processed).min(self.max_block_frames);

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

            let next_clock_seconds =
                clock_seconds + ClockSeconds(block_frames as f64 * self.sample_rate_recip);

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

            /*
            if !self.running {
                if frames_processed < frames {
                    output[frames_processed * num_out_channels..].fill(0.0);
                }
                break;
            }
            */

            frames_processed += block_frames;
            clock_samples += ClockSamples(block_frames as i64);
            clock_seconds = next_clock_seconds;
        }

        if self.hard_clip_outputs {
            for s in output.iter_mut() {
                *s = s.fract();
            }
        }

        if self.event_buffer.capacity() > 0 {
            let mut event_group = Vec::new();
            std::mem::swap(&mut self.event_buffer, &mut event_group);

            let _ = self
                .to_graph_tx
                .push(ProcessorToContextMsg::ReturnEventGroup(event_group));
        }
    }

    fn poll_messages(&mut self) {
        while let Ok(msg) = self.from_graph_rx.pop() {
            match msg {
                ContextToProcessorMsg::EventGroup(mut event_group) => {
                    let num_existing_events = self.event_buffer.len();

                    if self.event_buffer.capacity() == 0 {
                        std::mem::swap(&mut self.event_buffer, &mut event_group);
                    } else {
                        self.event_buffer.append(&mut event_group);

                        let _ = self
                            .to_graph_tx
                            .push(ProcessorToContextMsg::ReturnEventGroup(event_group));
                    }

                    for (i, event) in self.event_buffer[num_existing_events..].iter().enumerate() {
                        if let Some(node_entry) = self.nodes.get_mut(event.node_id.0) {
                            node_entry
                                .event_indices
                                .push((i + num_existing_events) as u32);
                        }
                    }
                }
                ContextToProcessorMsg::NewSchedule(mut new_schedule_data) => {
                    assert_eq!(
                        new_schedule_data.schedule.max_block_frames(),
                        self.max_block_frames
                    );

                    if let Some(mut old_schedule_data) = self.schedule_data.take() {
                        std::mem::swap(
                            &mut old_schedule_data.removed_nodes,
                            &mut new_schedule_data.removed_nodes,
                        );

                        for node_id in new_schedule_data.nodes_to_remove.iter() {
                            if let Some(node_entry) = self.nodes.remove(node_id.0) {
                                old_schedule_data.removed_nodes.push(NodeHeapData {
                                    id: *node_id,
                                    processor: node_entry.processor,
                                    event_buffer_indices: node_entry.event_indices,
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
                                n.id.0,
                                NodeEntry {
                                    processor: n.processor,
                                    event_indices: n.event_buffer_indices,
                                }
                            )
                            .is_none());
                    }

                    self.schedule_data = Some(new_schedule_data);
                }
                ContextToProcessorMsg::HardClipOutputs(hard_clip_outputs) => {
                    self.hard_clip_outputs = hard_clip_outputs;
                }
                ContextToProcessorMsg::SetTransport(transport) => {
                    if let Some(old_transport) = &mut self.transport {
                        if let Some(new_transport) = &transport {
                            if !old_transport.stopped {
                                // Update the playhead so that the new transport resumes after
                                // where the previous left off.

                                let current_musical = old_transport.transport.sample_to_musical(
                                    self.clock_samples - old_transport.start_frame,
                                    self.sample_rate.get(),
                                    self.sample_rate_recip,
                                );

                                old_transport.start_frame = self.clock_samples
                                    - new_transport
                                        .musical_to_sample(current_musical, self.sample_rate.get());
                            }

                            old_transport.transport = *new_transport;
                        } else {
                            self.transport = None;
                            self.clock_shared.musical.store(0, Ordering::Relaxed);
                        }
                    } else {
                        self.transport = transport.map(|transport| TransportState {
                            transport,
                            start_frame: ClockSamples::default(),
                            paused_at_frame: ClockSamples::default(),
                            paused_at_musical_time: MusicalTime::default(),
                            paused: false,
                            stopped: true,
                        });

                        self.clock_shared.musical.store(0, Ordering::Relaxed);
                    }
                }
                ContextToProcessorMsg::StartOrRestartTransport => {
                    if let Some(transport) = &mut self.transport {
                        transport.stopped = false;
                        transport.paused = false;
                        transport.start_frame = self.clock_samples;
                    }

                    self.clock_shared.musical.store(0, Ordering::Relaxed);
                }
                ContextToProcessorMsg::PauseTransport => {
                    if let Some(transport) = &mut self.transport {
                        if !transport.stopped && !transport.paused {
                            transport.paused = true;
                            transport.paused_at_frame = self.clock_samples;
                            transport.paused_at_musical_time =
                                transport.transport.sample_to_musical(
                                    self.clock_samples - transport.start_frame,
                                    self.sample_rate.get(),
                                    self.sample_rate_recip,
                                );
                        }
                    }
                }
                ContextToProcessorMsg::ResumeTransport => {
                    if let Some(transport) = &mut self.transport {
                        if !transport.stopped && transport.paused {
                            transport.paused = false;
                            transport.start_frame +=
                                ClockSamples(self.clock_samples.0 - transport.paused_at_frame.0);
                        }
                    }
                }
                ContextToProcessorMsg::StopTransport => {
                    if let Some(transport) = &mut self.transport {
                        transport.stopped = true;
                    }

                    self.clock_shared.musical.store(0, Ordering::Relaxed);
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
        if self.schedule_data.is_none() {
            return;
        }
        let schedule_data = self.schedule_data.as_mut().unwrap();

        let mut scratch_buffers = self.scratch_buffers.get_mut(self.max_block_frames);

        let transport_info = if let Some(t) = &self.transport {
            if t.stopped {
                None
            } else {
                let (start_beat, end_beat) = if t.paused {
                    (t.paused_at_musical_time, t.paused_at_musical_time)
                } else {
                    (
                        t.transport.sample_to_musical(
                            clock_samples - t.start_frame,
                            self.sample_rate.get(),
                            self.sample_rate_recip,
                        ),
                        t.transport.sample_to_musical(
                            clock_samples - t.start_frame + ClockSamples(block_frames as i64),
                            self.sample_rate.get(),
                            self.sample_rate_recip,
                        ),
                    )
                };

                Some(TransportInfo {
                    musical_clock: start_beat..end_beat,
                    transport: &t.transport,
                    paused: t.paused,
                })
            }
        } else {
            None
        };

        let mut proc_info = ProcInfo {
            frames: block_frames,
            in_silence_mask: SilenceMask::default(),
            out_silence_mask: SilenceMask::default(),
            clock_samples,
            clock_seconds: clock_seconds.clone(),
            transport_info,
            stream_status,
            declick_values: &self.declick_values,
        };

        schedule_data.schedule.process(
            block_frames,
            |node_id: NodeID,
             in_silence_mask: SilenceMask,
             out_silence_mask: SilenceMask,
             inputs: &[&[f32]],
             outputs: &mut [&mut [f32]]|
             -> ProcessStatus {
                let Some(node_entry) = self.nodes.get_mut(node_id.0) else {
                    return ProcessStatus::Bypass;
                };

                let events = NodeEventList::new(&mut self.event_buffer, &node_entry.event_indices);

                proc_info.in_silence_mask = in_silence_mask;
                proc_info.out_silence_mask = out_silence_mask;

                let status = node_entry.processor.process(
                    inputs,
                    outputs,
                    events,
                    &proc_info,
                    &mut scratch_buffers,
                );

                node_entry.event_indices.clear();

                status
            },
        );
    }
}

pub(crate) struct NodeEntry {
    pub processor: Box<dyn AudioNodeProcessor>,
    pub event_indices: Vec<u32>,
}

struct TransportState {
    transport: MusicalTransport,
    start_frame: ClockSamples,
    paused_at_frame: ClockSamples,
    paused_at_musical_time: MusicalTime,
    paused: bool,
    stopped: bool,
}

pub(crate) enum ContextToProcessorMsg {
    EventGroup(Vec<NodeEvent>),
    NewSchedule(Box<ScheduleHeapData>),
    HardClipOutputs(bool),
    SetTransport(Option<MusicalTransport>),
    StartOrRestartTransport,
    PauseTransport,
    ResumeTransport,
    StopTransport,
}

pub(crate) enum ProcessorToContextMsg {
    ReturnEventGroup(Vec<NodeEvent>),
    ReturnSchedule(Box<ScheduleHeapData>),
}
