use std::sync::Arc;

use firewheel::{
    basic_nodes::MixNode,
    clock::EventDelay,
    node::RepeatMode,
    sample_resource::SampleResource,
    sampler::{Sampler, SamplerNode, SamplerStatus},
    ChannelConfig, FirewheelCpalCtx, UpdateStatus,
};
use symphonium::SymphoniumLoader;

pub const SAMPLE_PATHS: [&'static str; 4] = [
    "assets/test_files/sword_swing.flac",
    "assets/test_files/bird_cherp.wav",
    "assets/test_files/beep.wav",
    "assets/test_files/bird_ambiance.ogg",
];

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
                let sample: Arc<dyn SampleResource> = Arc::new(
                    firewheel::load_audio_file(&mut loader, path, sample_rate, Default::default())
                        .unwrap(),
                );

                let node_id = graph
                    .add_node(Box::new(SamplerNode::new(Default::default())), None)
                    .unwrap();
                graph
                    .connect(
                        node_id,
                        mix_node_id,
                        &[(0, i as u32 * 2), (1, (i as u32 * 2) + 1)],
                        false,
                    )
                    .unwrap();

                let mut sampler = Sampler::new(node_id);

                sampler.set_sample(
                    Some(&sample),
                    1.0,
                    RepeatMode::PlayOnce,
                    EventDelay::Immediate,
                    graph,
                );

                sampler
            })
            .collect();

        Self { cx, samplers }
    }

    pub fn is_activated(&self) -> bool {
        self.cx.is_activated()
    }

    pub fn start_or_restart(
        &mut self,
        sampler_i: usize,
        normalized_volume: f32,
        repeat_mode: RepeatMode,
    ) {
        let graph = self.cx.graph_mut().unwrap();
        let sampler = &mut self.samplers[sampler_i];

        if normalized_volume != sampler.normalized_volume() || repeat_mode != sampler.repeat_mode()
        {
            sampler.set_sample(
                None,
                normalized_volume,
                repeat_mode,
                EventDelay::Immediate,
                graph,
            );
        }

        sampler.start_or_restart(EventDelay::Immediate, graph);
    }

    pub fn pause(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].pause(EventDelay::Immediate, graph);
    }

    pub fn resume(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].resume(EventDelay::Immediate, graph);
    }

    pub fn stop(&mut self, sampler_i: usize) {
        let graph = self.cx.graph_mut().unwrap();
        self.samplers[sampler_i].stop(graph);
    }

    pub fn sampler_status(&self, sampler_i: usize) -> SamplerStatus {
        self.samplers[sampler_i].status(self.cx.graph())
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
