use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Duration,
};

use firewheel::{
    channel_config::NonZeroChannelCount,
    error::UpdateError,
    nodes::stream::{
        input::{StreamInputConfig, StreamInputHandle},
        output::{StreamOutputConfig, StreamOutputHandle},
        ReadStatus, ResamplingChannelConfig,
    },
    FirewheelContext,
};

const CHANNEL_CAPACITY_SECONDS: f64 = 3.0;
const JITTER_THRESHOLD_SECONDS: f64 = 2.5;
const UPDATE_INTERVAL: Duration = Duration::from_millis(15);
const IN_SAMPLE_RATE: NonZeroU32 = NonZeroU32::new(44100).unwrap();
const OUT_SAMPLE_RATE: NonZeroU32 = NonZeroU32::new(48000).unwrap();
const NUM_CHANNELS: NonZeroChannelCount = NonZeroChannelCount::STEREO;

fn main() {
    simple_log::quick!("info");

    let mut cx = FirewheelContext::new(Default::default());
    cx.start_stream(Default::default()).unwrap();
    let output_stream_sample_rate = cx.stream_info().unwrap().sample_rate;

    dbg!(output_stream_sample_rate);

    let graph_out_node = cx.graph_out_node();

    let mut in_stream_handle = StreamInputHandle::new(
        StreamInputConfig {
            channel_config: ResamplingChannelConfig {
                // By default this is set to `0.4` (400 ms). You will probably want a larger
                // capacity buffer depending on your use case.
                capacity_seconds: CHANNEL_CAPACITY_SECONDS,
                ..Default::default()
            },
            ..Default::default()
        },
        NUM_CHANNELS,
    );

    let mut out_stream_handle = StreamOutputHandle::new(
        StreamOutputConfig {
            channel_config: ResamplingChannelConfig {
                // By default this is set to `0.4` (400 ms). You will probably want a larger
                // capacity buffer depending on your use case.
                capacity_seconds: CHANNEL_CAPACITY_SECONDS,
                ..Default::default()
            },
            ..Default::default()
        },
        NUM_CHANNELS,
    );

    let in_stream_id = cx.add_node(in_stream_handle.constructor());
    let out_stream_id = cx.add_node(out_stream_handle.constructor());

    cx.connect(in_stream_id, graph_out_node, &[(0, 0), (1, 1)], false)
        .unwrap();
    cx.connect(in_stream_id, out_stream_id, &[(0, 0), (1, 1)], false)
        .unwrap();

    let event = in_stream_handle
        .start_stream(IN_SAMPLE_RATE, output_stream_sample_rate)
        .unwrap();
    // This event must be sent to the node's processor for the stream to take effect.
    cx.queue_event_for(in_stream_id, event.into());

    let event = out_stream_handle
        .start_stream(OUT_SAMPLE_RATE, output_stream_sample_rate)
        .unwrap();
    // This event must be sent to the node's processor for the stream to take effect.
    cx.queue_event_for(out_stream_id, event.into());

    // Wrap the handles in an `Arc<Mutex<T>>>` so that we can send them to other threads.
    let in_stream_handle = Arc::new(Mutex::new(in_stream_handle));
    let out_stream_handle = Arc::new(Mutex::new(out_stream_handle));

    let in_stream_handle_2 = Arc::clone(&in_stream_handle);
    std::thread::spawn(move || {
        let mut phasor: f32 = 0.0;
        let phasor_inc: f32 = 440.0 / IN_SAMPLE_RATE.get() as f32;

        // We will send packets of data that are 2 seconds long.
        let mut in_buf =
            vec![0.0; IN_SAMPLE_RATE.get() as usize * NUM_CHANNELS.get().get() as usize];

        loop {
            let mut handle = in_stream_handle_2.lock().unwrap();

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamInputConfig::channel_config.latency_seconds`].
            if handle.underflow_occurred() {
                println!("Underflow occured in input stream node!");
            }

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamInputConfig::channel_config.capacity_seconds`]. For
            // example, if you are streaming data from a network, you may want to
            // increase the capacity to several seconds.
            if handle.overflow_occurred() {
                println!("Overflow occured in input stream node!");
            }

            // Wait until the node's processor is ready to receive data.
            if handle.is_ready() {
                // The "jitter value" can be used to get the difference in speed between the
                // input and output channels.
                //
                // Here, if the value drops below `0.0`, then we know we should push a new
                // packet of data.
                if handle.jitter_seconds().unwrap() <= 0.0 {
                    // Generate a sine wave on all channels.
                    for chunk in in_buf.chunks_exact_mut(NUM_CHANNELS.get().get() as usize) {
                        let val = (phasor * std::f32::consts::TAU).sin() * 0.5;
                        phasor = (phasor + phasor_inc).fract();

                        for s in chunk.iter_mut() {
                            *s = val;
                        }
                    }

                    handle.push_interleaved(&in_buf);

                    println!("Input stream pushed data.");
                }
            }

            std::thread::sleep(UPDATE_INTERVAL);
        }
    });

    let out_stream_handle_2 = Arc::clone(&out_stream_handle);
    std::thread::spawn(move || {
        // We will read packets of data that are 15 ms long, this time in
        // de-interleaved format.
        let packet_frames =
            (OUT_SAMPLE_RATE.get() as f32 * UPDATE_INTERVAL.as_secs_f32()).round() as usize;
        let mut out_buf: Vec<Vec<f32>> = (0..NUM_CHANNELS.get().get())
            .map(|_| vec![0.0; packet_frames])
            .collect();

        loop {
            let mut handle = out_stream_handle_2.lock().unwrap();

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamOutputConfig::channel_config.latency_seconds`].
            if handle.underflow_occurred() {
                println!("Underflow occured in output stream node!");
            }

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamOutputConfig::channel_config.capacity_seconds`]. For
            // example, if you are streaming data from a network, you may want to
            // increase the capacity to several seconds.
            if handle.overflow_occurred() {
                println!("Overflow occured in output stream node!");
            }

            // Wait until the node's processor is ready to send data.
            if handle.is_ready() {
                // Optionally, we can discard frames if the jitter value exceeds a given
                // threshold to avoid excessive overflows.
                //
                // Alternatively, instead of discarding samples, you may choose to read
                // an extra packet of data to correct for the jitter.
                let discarded_frames = handle.discard_jitter(JITTER_THRESHOLD_SECONDS);
                if discarded_frames > 0 {
                    println!("Overflow occured in output stream node!");
                    println!("Discarded frames in output stream: {}", discarded_frames);
                }

                let status = handle.read(&mut out_buf, 0..packet_frames).unwrap();

                // Alternatively, if you just wish to read all available frames in the
                // channel, the number of available frames can be gotten with
                // `handle.available_frames()`. Then read like normal into a buffer of
                // that length.

                match status {
                    ReadStatus::Ok => {
                        println!("Successfully read data");
                    }
                    ReadStatus::Underflow => {
                        // An input underflow occured. This may result in audible audio
                        // glitches.
                        println!("Underflow occured in output stream node!");
                    }
                    ReadStatus::WaitingForFrames => {
                        // The channel is waiting for a certain number of frames to be
                        // filled in the buffer before continuing after an underflow
                        // or a reset. The output will contain silence.
                    }
                }
            }

            std::thread::sleep(UPDATE_INTERVAL);
        }
    });

    loop {
        if let Err(e) = cx.update() {
            log::error!("{:?}", &e);

            if let UpdateError::StreamStoppedUnexpectedly(_) = e {
                // Notify the stream node handles that the output stream has stopped.
                // This will automatically stop any active streams on the nodes.
                in_stream_handle.lock().unwrap().stop_stream();
                out_stream_handle.lock().unwrap().stop_stream();

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
