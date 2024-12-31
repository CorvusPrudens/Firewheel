use std::time::{Duration, Instant};

use firewheel::{
    basic_nodes::beep_test::{self, BeepTestNode},
    FirewheelCpalCtx, UpdateStatus,
};

const BEEP_FREQUENCY_HZ: f32 = 440.0;
const BEEP_NORMALIZED_VOLUME: f32 = 0.4;
const BEEP_DURATION: Duration = Duration::from_secs(4);
const UPDATE_INTERVAL: Duration = Duration::from_millis(15);

fn main() {
    simple_log::quick!("info");

    println!("Firewheel beep test...");

    let mut cpal_cx = FirewheelCpalCtx::new(Default::default(), Default::default()).unwrap();

    let beep_test_node = BeepTestNode::new(
        beep_test::Params {
            freq_hz: BEEP_FREQUENCY_HZ,
            normalized_volume: BEEP_NORMALIZED_VOLUME,
            enabled: true,
        },
        &mut cpal_cx.cx,
    );
    let graph_out_id = cpal_cx.cx.graph_out_node();

    cpal_cx
        .cx
        .connect(beep_test_node.id(), graph_out_id, &[(0, 0), (0, 1)], false)
        .unwrap();

    let mut cpal_cx = Some(cpal_cx);

    let start = Instant::now();
    while start.elapsed() < BEEP_DURATION {
        std::thread::sleep(UPDATE_INTERVAL);

        let Some(cx) = cpal_cx.take() else {
            break;
        };

        match cx.update() {
            UpdateStatus::Ok {
                cx,
                graph_compile_error,
            } => {
                cpal_cx = Some(cx);

                if let Some(e) = graph_compile_error {
                    log::error!("graph compile error: {}", e);
                }
            }
            UpdateStatus::Deactivated { error } => {
                log::error!("Deactivated unexpectedly: {:?}", error);

                break;
            }
        }
    }

    println!("finished");
}
