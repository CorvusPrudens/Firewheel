use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Duration,
};

use firewheel::{
    channel_config::NonZeroChannelCount,
    error::UpdateError,
    nodes::stream::{
        reader::{StreamReaderConfig, StreamReaderNode},
        writer::{StreamWriterConfig, StreamWriterNode},
        ReadStatus, ResamplingChannelConfig,
    },
    FirewheelContext,
};

const CHANNEL_CAPACITY_SECONDS: f64 = 4.0;
const JITTER_THRESHOLD_SECONDS: f64 = 3.5;
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

    let graph_out_node_id = cx.graph_out_node_id();

    let mut stream_writer_node = StreamWriterNode::new(
        StreamWriterConfig {
            channel_config: ResamplingChannelConfig {
                // By default this is set to `0.4` (400 ms). You will probably want a larger
                // capacity buffer depending on your use case. Generally this value should
                // be at least twice as large as the size of packets you intend to send.
                capacity_seconds: CHANNEL_CAPACITY_SECONDS,
                ..Default::default()
            },
            ..Default::default()
        },
        NUM_CHANNELS,
    );

    let mut stream_reader_node = StreamReaderNode::new(
        StreamReaderConfig {
            channel_config: ResamplingChannelConfig {
                // For stream readers, the `latency_seconds` value should also be at least
                // the size of packets you intend to read. Here, we use twice that size to
                // be safe.
                latency_seconds: 0.3,
                // By default this is set to `0.4` (400 ms). You will probably want a larger
                // capacity buffer depending on your use case. Generally this value should
                // be at least twice as large as the size of packets you intend to send.
                //
                // This value should also be at least twice as large as `latency_seconds`.
                capacity_seconds: 0.6,
                ..Default::default()
            },
            ..Default::default()
        },
        NUM_CHANNELS,
    );

    let stream_writer_id = cx.add_node(stream_writer_node.clone(), None);
    let stream_reader_id = cx.add_node(stream_reader_node.clone(), None);

    cx.connect(
        stream_writer_id,
        graph_out_node_id,
        &[(0, 0), (1, 1)],
        false,
    )
    .unwrap();
    cx.connect(stream_writer_id, stream_reader_id, &[(0, 0), (1, 1)], false)
        .unwrap();

    let event = stream_writer_node
        .start_stream(IN_SAMPLE_RATE, output_stream_sample_rate)
        .unwrap();
    // This event must be sent to the node's processor for the stream to take effect.
    cx.queue_event_for(stream_writer_id, event.into());

    let event = stream_reader_node
        .start_stream(OUT_SAMPLE_RATE, output_stream_sample_rate)
        .unwrap();
    // This event must be sent to the node's processor for the stream to take effect.
    cx.queue_event_for(stream_reader_id, event.into());

    // Wrap the handles in an `Arc<Mutex<T>>>` so that we can send them to other threads.
    let stream_writer_handle = Arc::new(Mutex::new(stream_writer_node));
    let stream_reader_handle = Arc::new(Mutex::new(stream_reader_node));

    let stream_writer_handle_2 = Arc::clone(&stream_writer_handle);
    std::thread::spawn(move || {
        let mut phasor: f32 = 0.0;
        let phasor_inc: f32 = 440.0 / IN_SAMPLE_RATE.get() as f32;

        // We will send packets of data that are 1 second long.
        let packet_frames = IN_SAMPLE_RATE.get() as usize;

        let mut in_buf = vec![0.0; packet_frames * NUM_CHANNELS.get().get() as usize];

        loop {
            let mut handle = stream_writer_handle_2.lock().unwrap();

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamWriterConfig::channel_config.latency_seconds`].
            if handle.underflow_occurred() {
                println!("Underflow occured in stream writer node!");
            }

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamWriterConfig::channel_config.capacity_seconds`]. For
            // example, if you are streaming data from a network, you may want to
            // increase the capacity to several seconds.
            if handle.overflow_occurred() {
                println!("Overflow occured in stream writer node!");
            }

            // Wait until the node's processor is ready to receive data.
            if handle.is_ready() {
                // The "jitter value" can be used to get the difference in speed between the
                // input and output channels.
                //
                // Here, if the value drops below the size of a packet `1.0`, then we know we
                // should push a new packet of data.
                //
                // Alternatively you could do:
                //
                // while handle.occupied_seconds().unwrap() < handle.latency_seconds() {
                //
                // or
                //
                // while handle.available_frames() >= packet_frames {
                //
                while handle.occupied_seconds().unwrap() < 1.0 {
                    // Generate a sine wave on all channels.
                    for chunk in in_buf.chunks_exact_mut(NUM_CHANNELS.get().get() as usize) {
                        let val = (phasor * std::f32::consts::TAU).sin() * 0.5;
                        phasor = (phasor + phasor_inc).fract();

                        for s in chunk.iter_mut() {
                            *s = val;
                        }
                    }

                    handle.push_interleaved(&in_buf);

                    println!("Stream writer pushed data.");
                }
            }

            std::thread::sleep(UPDATE_INTERVAL);
        }
    });

    let stream_reader_handle_2 = Arc::clone(&stream_reader_handle);
    std::thread::spawn(move || {
        // We will read packets of data that are 15 ms long, this time in
        // de-interleaved format.
        let packet_frames =
            (OUT_SAMPLE_RATE.get() as f32 * UPDATE_INTERVAL.as_secs_f32()).round() as usize;
        let mut out_buf: Vec<Vec<f32>> = (0..NUM_CHANNELS.get().get())
            .map(|_| vec![0.0; packet_frames])
            .collect();

        loop {
            let mut handle = stream_reader_handle_2.lock().unwrap();

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamReaderConfig::channel_config.latency_seconds`].
            if handle.underflow_occurred() {
                println!("Underflow occured in stream reader node!");
            }

            // If this happens excessively in Release mode, you may want to consider
            // increasing [`StreamReaderConfig::channel_config.capacity_seconds`]. For
            // example, if you are streaming data from a network, you may want to
            // increase the capacity to several seconds.
            if handle.overflow_occurred() {
                println!("Overflow occured in stream reader node!");
            }

            // Wait until the node's processor is ready to send data.
            if handle.is_ready() {
                // Optionally, If the value of `StreamReaderHandle::occupied_seconds()`
                // is greater than the given threshold in seconds, then discard the
                // number of input frames needed to bring the value back down to
                // `StreamReaderHandle::latency_seconds()` to avoid excessive overflows
                // and reduce perceived audible glitchiness.
                //
                // Alternatively, instead of discarding samples, you may choose to read
                // an extra packet of data to correct for the jitter.
                let discarded_frames = handle.discard_jitter(JITTER_THRESHOLD_SECONDS);
                if discarded_frames > 0 {
                    println!("Overflow occured in stream reader node!");
                    println!("Discarded frames in stream reader: {}", discarded_frames);
                }

                let status = handle.read(&mut out_buf, 0..packet_frames).unwrap();

                match status {
                    ReadStatus::Ok => {
                        println!("Successfully read data");
                    }
                    ReadStatus::Underflow => {
                        // An input underflow occured. This may result in audible audio
                        // glitches.
                        println!("Underflow occured in stream reader node!");
                    }
                    ReadStatus::WaitingForFrames => {
                        // The channel is waiting for a certain number of frames to be
                        // filled in the buffer before continuing after an underflow
                        // or a reset. The output will contain silence.
                    }
                }

                // Alternatively, if you just wish to read all available frames in the
                // channel, then you could do:
                //
                // while handle.available_frames() >= packet_frames {
                //     let status = handle.read(&mut out_buf, 0..packet_frames).unwrap();
                //
                //     // Send data over the network, for example.
                // }
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
                stream_writer_handle.lock().unwrap().stop_stream();
                stream_reader_handle.lock().unwrap().stop_stream();

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
