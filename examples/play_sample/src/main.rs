use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use clap::Parser;
use firewheel::{
    clock::EventDelay,
    node::{NodeEvent, NodeEventType},
    sampler::one_shot::{OneShotSamplerNode, DEFAULT_MAX_VOICES},
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

    let sample =
        firewheel::load_audio_file(&mut loader, args.path, sample_rate, Default::default())
            .unwrap();
    let sample_duration = Duration::from_secs_f64(sample.duration_seconds());

    let graph = cx.graph_mut().unwrap();
    let sampler_node = graph
        .add_node(
            OneShotSamplerNode::<DEFAULT_MAX_VOICES>::new(Default::default()).into(),
            None,
        )
        .unwrap();
    graph
        .connect(sampler_node, 0, graph.graph_out_node(), 0, false)
        .unwrap();
    graph
        .connect(sampler_node, 1, graph.graph_out_node(), 1, false)
        .unwrap();

    graph.queue_event(NodeEvent {
        node_id: sampler_node,
        delay: EventDelay::Immediate,
        event: NodeEventType::PlaySample {
            sample: Arc::new(sample),
            percent_volume: 100.0,
            stop_other_voices: false,
        },
    });

    let start = Instant::now();
    // Give a little bit of leeway to account for latency.
    let duration = sample_duration + Duration::from_millis(100);
    while start.elapsed() < duration {
        std::thread::sleep(UPDATE_INTERVAL);

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
