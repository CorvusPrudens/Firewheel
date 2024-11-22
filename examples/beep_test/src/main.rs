use std::time::{Duration, Instant};

use firewheel::{basic_nodes::beep_test::BeepTestNode, FirewheelCpalCtx, UpdateStatus};

const BEEP_FREQUENCY_HZ: f32 = 440.0;
const BEEP_GAIN_DB: f32 = -12.0;
const BEEP_DURATION: Duration = Duration::from_secs(4);
const UPDATE_INTERVAL: Duration = Duration::from_millis(15);

fn main() {
    simple_log::quick!("info");

    println!("Firewheel beep test...");

    let mut cx = FirewheelCpalCtx::new(Default::default());
    cx.activate(Default::default()).unwrap();

    let graph = cx.graph_mut().unwrap();
    let beep_test_node = graph
        .add_node(
            Box::new(BeepTestNode::new(BEEP_FREQUENCY_HZ, BEEP_GAIN_DB, true)),
            None,
        )
        .unwrap();
    graph
        .connect(beep_test_node, 0, graph.graph_out_node(), 0, false)
        .unwrap();
    graph
        .connect(beep_test_node, 1, graph.graph_out_node(), 1, false)
        .unwrap();

    let start = Instant::now();
    while start.elapsed() < BEEP_DURATION {
        std::thread::sleep(UPDATE_INTERVAL);

        cx.flush_events();
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
    }

    println!("finished");
}
