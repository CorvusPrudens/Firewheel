use firewheel::{
    diff::Memo,
    error::UpdateError,
    node::NodeID,
    nodes::{
        sampler::{RepeatMode, SamplerHandle, SamplerParams},
        spatial_basic::SpatialBasicParams,
    },
    FirewheelContext,
};
use symphonium::SymphoniumLoader;

pub struct AudioSystem {
    pub cx: FirewheelContext,

    pub _sampler_params: SamplerParams,
    pub _sampler_handle: SamplerHandle,
    pub _sampler_node: NodeID,

    pub spatial_basic_params: Memo<SpatialBasicParams>,
    pub spatial_basic_node: NodeID,
}

impl AudioSystem {
    pub fn new() -> Self {
        let mut cx = FirewheelContext::new(Default::default());
        cx.start_stream(Default::default()).unwrap();

        let sample_rate = cx.stream_info().unwrap().sample_rate;

        let mut loader = SymphoniumLoader::new();
        let sample = firewheel::load_audio_file(
            &mut loader,
            "assets/test_files/dpren_very-lush-and-swag-loop.ogg",
            sample_rate,
            Default::default(),
        )
        .unwrap()
        .into_dyn_resource();

        let graph_out = cx.graph_out_node();

        let mut sampler_params = SamplerParams::default();
        sampler_params.set_sample(sample, 1.0, RepeatMode::RepeatEndlessly);

        let sampler_handle = SamplerHandle::new();
        let sampler_node =
            cx.add_node(sampler_handle.constructor(sampler_params.clone(), Default::default()));

        let spatial_basic_params = SpatialBasicParams::default();
        let spatial_basic_node = cx.add_node(spatial_basic_params.constructor(Default::default()));

        cx.connect(sampler_node, spatial_basic_node, &[(0, 0), (1, 1)], false)
            .unwrap();
        cx.connect(spatial_basic_node, graph_out, &[(0, 0), (1, 1)], false)
            .unwrap();

        cx.queue_event_for(
            sampler_node,
            sampler_handle.start_or_restart_event(&sampler_params, None),
        );

        Self {
            cx,
            _sampler_params: sampler_params,
            _sampler_handle: sampler_handle,
            _sampler_node: sampler_node,
            spatial_basic_params: Memo::new(spatial_basic_params),
            spatial_basic_node,
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
