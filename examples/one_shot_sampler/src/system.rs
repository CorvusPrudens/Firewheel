use std::sync::Arc;

use firewheel::{
    basic_nodes::MixNode,
    clock::EventDelay,
    graph::AudioGraph,
    node::{NodeEvent, NodeEventType, NodeID},
    sample_resource::SampleResource,
    sampler::one_shot::OneShotSamplerNode,
    ChannelConfig, FirewheelCpalCtx, UpdateStatus,
};
use symphonium::SymphoniumLoader;

pub const SAMPLE_PATHS: [&'static str; 3] = [
    "assets/test_files/sword_swing.flac",
    "assets/test_files/bird_cherp.wav",
    "assets/test_files/beep.wav",
];

pub struct Sampler {
    pub paused: bool,
    pub stop_other_voices: bool,
    pub volume: f32,

    node_id: NodeID,
    sample: Arc<dyn SampleResource>,
}

impl Sampler {
    fn play(&mut self, graph: &mut AudioGraph) {
        if !self.paused {
            graph.queue_event(NodeEvent {
                node_id: self.node_id,
                delay: EventDelay::Immediate,
                event: NodeEventType::PlaySample {
                    sample: Arc::clone(&self.sample),
                    normalized_volume: self.volume / 100.0,
                    stop_other_voices: self.stop_other_voices,
                },
            });
        }
    }

    fn pause(&mut self, graph: &mut AudioGraph) {
        if !self.paused {
            self.paused = true;

            graph.queue_event(NodeEvent {
                node_id: self.node_id,
                delay: EventDelay::Immediate,
                event: NodeEventType::Pause,
            });
        }
    }

    fn resume(&mut self, graph: &mut AudioGraph) {
        if self.paused {
            self.paused = false;

            graph.queue_event(NodeEvent {
                node_id: self.node_id,
                delay: EventDelay::Immediate,
                event: NodeEventType::Resume,
            });
        }
    }

    fn stop(&mut self, graph: &mut AudioGraph) {
        self.paused = false;

        graph.queue_event(NodeEvent {
            node_id: self.node_id,
            delay: EventDelay::Immediate,
            event: NodeEventType::Stop,
        });
    }
}

pub struct AudioSystem {
    cx: FirewheelCpalCtx,

    pub samplers: Vec<Sampler>,
}

impl AudioSystem {
    pub fn new() -> Self {
        let mut cx = FirewheelCpalCtx::new(Default::default());
        cx.activate(Default::default()).unwrap();

        let sample_rate = cx.stream_info().unwrap().sample_rate;

        let mut loader = SymphoniumLoader::new();

        let graph = cx.graph_mut().unwrap();
        let graph_out = graph.graph_out_node();

        let mix_node_id = graph
            .add_node(
                Box::new(MixNode),
                Some(ChannelConfig {
                    num_inputs: (2 * SAMPLE_PATHS.len()).into(),
                    num_outputs: 2.into(),
                }),
            )
            .unwrap();
        graph
            .connect(mix_node_id, graph_out, &[(0, 0), (1, 1)], false)
            .unwrap();

        let samplers = SAMPLE_PATHS
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let sample =
                    firewheel::load_audio_file(&mut loader, path, sample_rate, Default::default())
                        .unwrap();

                let node_id = graph
                    .add_node(Box::new(OneShotSamplerNode::new(Default::default())), None)
                    .unwrap();
                graph
                    .connect(
                        node_id,
                        mix_node_id,
                        &[(0, i as u32 * 2), (1, (i as u32 * 2) + 1)],
                        false,
                    )
                    .unwrap();

                Sampler {
                    paused: false,
                    stop_other_voices: false,
                    volume: 100.0,
                    node_id,
                    sample: Arc::new(sample),
                }
            })
            .collect();

        Self { cx, samplers }
    }

    pub fn is_activated(&self) -> bool {
        self.cx.is_activated()
    }

    pub fn play(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].play(graph);
    }

    pub fn pause(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].pause(graph);
    }

    pub fn resume(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].resume(graph);
    }

    pub fn stop(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].stop(graph);
    }

    pub fn update(&mut self) {
        match self.cx.update() {
            UpdateStatus::Inactive => {}
            UpdateStatus::Active { graph_error } => {
                if let Some(e) = graph_error {
                    log::error!("audio graph error: {}", e);
                }
            }
            UpdateStatus::Deactivated { error, .. } => {
                if let Some(e) = error {
                    log::error!("Stream disconnected: {}", e);
                } else {
                    log::error!("Stream disconnected");
                }
            }
        }

        self.cx.flush_events();
    }
}
