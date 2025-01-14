use std::{sync::Arc, time::Duration};

use clap::Parser;
use firewheel::{
    error::UpdateError,
    sample_resource::SampleResource,
    sampler::{PlaybackState, RepeatMode, SamplerState},
    FirewheelContext,
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

    // --- Start the context and get the sample rate of the audio stream. ----------------

    let mut cx = FirewheelContext::new(Default::default());
    cx.start_stream(Default::default()).unwrap();

    let sample_rate = cx.stream_info().unwrap().sample_rate;

    // --- Create a sampler state, and add it as a node in the audio graph. --------------

    let mut sampler_state = SamplerState::default();

    let sampler_id = cx.add_node(sampler_state.clone());
    let graph_out_id = cx.graph_out_node();

    cx.connect(sampler_id, graph_out_id, &[(0, 0), (1, 1)], false)
        .unwrap();

    // --- Load a sample into memory, and tell the node to use it and play it. -----------

    let mut loader = SymphoniumLoader::new();
    let sample: Arc<dyn SampleResource> = Arc::new(
        firewheel::load_audio_file(&mut loader, args.path, sample_rate, Default::default())
            .unwrap(),
    );

    sampler_state.set_sample(sample, 1.0, RepeatMode::PlayOnce);

    cx.queue_event_for(sampler_id, sampler_state.sync_sequence_event(true));

    // Alternatively, instead of setting `start_immediately` to `true`, you can
    // tell the sampler to start playing its sequence like this:
    //
    // cx.queue_event_for(
    //    sampler_id,
    //    sampler_state.start_or_restart_event(EventDelay::Immediate),
    // );

    // --- Simulated update loop ---------------------------------------------------------

    loop {
        if sampler_state.playback_state() == PlaybackState::Stopped {
            // Sample has finished playing.
            break;
        }

        if let Err(e) = cx.update() {
            log::error!("{:?}", &e);

            if let UpdateError::StreamStoppedUnexpectedly(_) = e {
                // The stream has stopped unexpectedly (i.e the user has
                // unplugged their headphones.)
                //
                // Typically you should start a new stream as soon as
                // possible to resume processing (event if it's a dummy
                // output device).
                //
                // In this example we just quit the application.
                break;
            }
        }

        std::thread::sleep(UPDATE_INTERVAL);
    }

    println!("finished");
}
