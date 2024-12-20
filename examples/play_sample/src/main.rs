use std::{sync::Arc, time::Duration};

use clap::Parser;
use firewheel::{
    clock::EventDelay,
    node::RepeatMode,
    sample_resource::SampleResource,
    sampler::{Sampler, SamplerNode},
    FirewheelCpalCtx, UpdateStatus,
};
use symphonium::SymphoniumLoader;

const UPDATE_INTERVAL: Duration = Duration::from_millis(15);

#[derive(clap::Parser)]
struct Cli {
    /// The path to the audio file to play
    path: std::path::PathBuf,
}

fn main() {
    simple_log::quick!("info");

    let args = Cli::parse();

    let mut loader = SymphoniumLoader::new();

    let mut cx = FirewheelCpalCtx::new(Default::default());
    cx.activate(Default::default()).unwrap();

    let sample_rate = cx.stream_info().unwrap().sample_rate;

    let sample: Arc<dyn SampleResource> = Arc::new(
        firewheel::load_audio_file(&mut loader, args.path, sample_rate, Default::default())
            .unwrap(),
    );

    let graph = cx.graph_mut().unwrap();
    let sampler_node_id = graph
        .add_node(SamplerNode::new(Default::default()).into(), None)
        .unwrap();
    graph
        .connect(
            sampler_node_id,
            graph.graph_out_node(),
            &[(0, 0), (1, 1)],
            false,
        )
        .unwrap();

    let mut sampler = Sampler::new(sampler_node_id);
    sampler.set_sample(
        Some(&sample),
        1.0,
        RepeatMode::PlayOnce,
        EventDelay::Immediate,
        graph,
    );
    sampler.start_or_restart(EventDelay::Immediate, graph);

    loop {
        std::thread::sleep(UPDATE_INTERVAL);

        if sampler.status(cx.graph()).finished() {
            break;
        }

        match cx.update() {
            UpdateStatus::Inactive => {}
            UpdateStatus::Active { graph_error } => {
                if let Some(e) = graph_error {
                    log::error!("graph error: {}", e);
                }
            }
            UpdateStatus::Deactivated { error, .. } => {
                log::error!("Deactivated unexpectedly: {:?}", error);

                break;
            }
        }
        cx.flush_events();
    }

    println!("finished");
}
