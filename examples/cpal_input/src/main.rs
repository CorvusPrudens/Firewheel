use std::time::Duration;

use firewheel::{
    channel_config::NonZeroChannelCount, error::UpdateError, input::CpalInputNodeHandle,
    FirewheelContext,
};

const UPDATE_INTERVAL: Duration = Duration::from_millis(15);

fn main() {
    simple_log::quick!("info");

    let mut cx = FirewheelContext::new(Default::default());
    cx.start_stream(Default::default()).unwrap();
    let output_stream_sample_rate = cx.stream_info().unwrap().sample_rate;

    dbg!(output_stream_sample_rate);

    let graph_out_node = cx.graph_out_node();

    let mut input_node_handle =
        CpalInputNodeHandle::new(Default::default(), NonZeroChannelCount::MONO);
    let input_node_id = cx.add_node(input_node_handle.clone(), None);

    cx.connect(input_node_id, graph_out_node, &[(0, 0), (0, 1)], false)
        .unwrap();

    match input_node_handle.start_stream(Default::default(), output_stream_sample_rate) {
        Ok((input_stream_info, event)) => {
            dbg!(input_stream_info);

            // Notify the input node's processor that a new input stream has
            // been started.
            cx.queue_event_for(input_node_id, event.into());
        }
        Err(e) => {
            log::error!("Failed to open input stream: {}", e);
        }
    }

    loop {
        if input_node_handle.underflow_occurred() {
            println!("underflow occured!");
        }
        if input_node_handle.overflow_occurred() {
            println!("overflow occured!");
        }

        if let Err(e) = input_node_handle.poll_status() {
            // The input stream has been stopped unexpectedly (i.e. the user
            // unplugged their microphone).
            log::error!("Input stream stopped unexpectedly: {}", e);

            // Typically you should continue as normal or start a new input
            // stream, but in this example we just quit the application.
            break;
        }

        if let Err(e) = cx.update() {
            log::error!("{:?}", &e);

            if let UpdateError::StreamStoppedUnexpectedly(_) = e {
                // Notify the input node that the output stream has stopped. This
                // will automatically stop any running input audio streams.
                input_node_handle.stop_stream();

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
}
