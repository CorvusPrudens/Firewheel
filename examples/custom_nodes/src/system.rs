use firewheel::{diff::Memo, error::UpdateError, node::NodeID, FirewheelContext};

use crate::nodes::{
    filter::FilterParams,
    noise_gen::NoiseGenParams,
    rms::{RmsHandle, RmsParams},
};

pub struct AudioSystem {
    pub cx: FirewheelContext,

    pub noise_gen_params: Memo<NoiseGenParams>,
    pub filter_params: Memo<FilterParams>,
    pub rms_params: RmsParams,
    pub rms_handle: RmsHandle,

    pub noise_gen_node: NodeID,
    pub filter_node: NodeID,
}

impl AudioSystem {
    pub fn new() -> Self {
        let mut cx = FirewheelContext::new(Default::default());
        cx.start_stream(Default::default()).unwrap();

        let noise_gen_params = NoiseGenParams::default();
        let filter_params = FilterParams::default();
        let rms_params = RmsParams::default();
        let rms_handle = RmsHandle::new(rms_params);

        let noise_gen_node = cx.add_node(noise_gen_params.constructor(None));
        let filter_node = cx.add_node(filter_params.constructor());
        let rms_node = cx.add_node(rms_handle.constructor(Default::default()));

        let graph_out = cx.graph_out_node();

        cx.connect(noise_gen_node, filter_node, &[(0, 0)], false)
            .unwrap();
        cx.connect(filter_node, rms_node, &[(0, 0)], false).unwrap();
        cx.connect(filter_node, graph_out, &[(0, 0), (0, 1)], false)
            .unwrap();

        Self {
            cx,
            noise_gen_params: Memo::new(noise_gen_params),
            filter_params: Memo::new(filter_params),
            rms_params,
            rms_handle,
            noise_gen_node,
            filter_node,
        }
    }

    pub fn update(&mut self) {
        if let Err(e) = self.cx.update() {
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
                panic!("Stream stopped unexpectedly!");
            }
        }
    }
}
