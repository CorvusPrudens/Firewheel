use firewheel::{
    basic_nodes::peak_meter::{PeakMeterSmoother, PeakMeterState},
    dsp::decibel::DbMeterNormalizer,
    error::UpdateError,
    node::NodeID,
    sampler::{PlaybackState, RepeatMode, SamplerState, SequenceType},
    FirewheelContext,
};
use symphonium::SymphoniumLoader;

pub const SAMPLE_PATHS: [&'static str; 4] = [
    "assets/test_files/sword_swing.flac",
    "assets/test_files/bird_cherp.wav",
    "assets/test_files/beep.wav",
    "assets/test_files/bird_ambiance.ogg",
];

struct Sampler {
    pub state: SamplerState,
    pub node_id: NodeID,
}

pub struct AudioSystem {
    cx: FirewheelContext,

    samplers: Vec<Sampler>,

    peak_meter: PeakMeterState<2>,
    peak_meter_smoother: PeakMeterSmoother<2>,
    peak_meter_normalizer: DbMeterNormalizer,
}

impl AudioSystem {
    pub fn new() -> Self {
        let mut cx = FirewheelContext::new(Default::default());
        cx.start_stream(Default::default()).unwrap();

        let sample_rate = cx.stream_info().unwrap().sample_rate;

        let mut loader = SymphoniumLoader::new();

        let graph_out = cx.graph_out_node();

        let peak_meter = PeakMeterState::<2>::new(true);
        let peak_meter_smoother = PeakMeterSmoother::<2>::new(Default::default());

        let peak_meter_id = cx.add_node(peak_meter.clone());
        cx.connect(peak_meter_id, graph_out, &[(0, 0), (1, 1)], false)
            .unwrap();

        let samplers = SAMPLE_PATHS
            .iter()
            .map(|path| {
                let sample =
                    firewheel::load_audio_file(&mut loader, path, sample_rate, Default::default())
                        .unwrap()
                        .into_dyn_resource();

                let state = SamplerState::new(
                    Some(SequenceType::SingleSample {
                        sample,
                        normalized_volume: 1.0,
                        repeat_mode: RepeatMode::PlayOnce,
                    }),
                    Default::default(),
                );

                let node_id = cx.add_node(state.clone());

                cx.connect(node_id, peak_meter_id, &[(0, 0), (1, 1)], false)
                    .unwrap();

                Sampler { state, node_id }
            })
            .collect();

        let peak_meter_normalizer = DbMeterNormalizer::default();
        dbg!(&peak_meter_normalizer);

        Self {
            cx,
            samplers,
            peak_meter,
            peak_meter_smoother,
            peak_meter_normalizer: DbMeterNormalizer::default(),
        }
    }

    pub fn is_activated(&self) -> bool {
        self.cx.is_audio_stream_running()
    }

    pub fn start_or_restart(
        &mut self,
        sampler_i: usize,
        normalized_volume: f32,
        repeat_mode: RepeatMode,
    ) {
        let sampler = &mut self.samplers[sampler_i];

        let Some(SequenceType::SingleSample {
            normalized_volume: old_normalized_volume,
            repeat_mode: old_repeat_mode,
            ..
        }) = &mut sampler.state.sequence
        else {
            return;
        };

        if normalized_volume != *old_normalized_volume || repeat_mode != *old_repeat_mode {
            *old_normalized_volume = normalized_volume;
            *old_repeat_mode = repeat_mode;

            self.cx
                .queue_event_for(sampler.node_id, sampler.state.sync_sequence_event(true));
        } else {
            self.cx
                .queue_event_for(sampler.node_id, sampler.state.start_or_restart_event(None));
        }
    }

    pub fn pause(&mut self, sampler_i: usize) {
        let sampler = &self.samplers[sampler_i];

        self.cx
            .queue_event_for(sampler.node_id, sampler.state.pause_event());
    }

    pub fn resume(&mut self, sampler_i: usize) {
        let sampler = &self.samplers[sampler_i];

        self.cx
            .queue_event_for(sampler.node_id, sampler.state.resume_event());
    }

    pub fn stop(&mut self, sampler_i: usize) {
        let sampler = &self.samplers[sampler_i];

        self.cx
            .queue_event_for(sampler.node_id, sampler.state.stop_event());
    }

    pub fn playback_state(&self, sampler_i: usize) -> PlaybackState {
        self.samplers[sampler_i].state.playback_state()
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

    pub fn update_meters(&mut self, delta_seconds: f32) {
        self.peak_meter_smoother
            .update(self.peak_meter.peak_gain_db(), delta_seconds);
    }

    pub fn peak_meter_values(&self) -> [f32; 2] {
        self.peak_meter_smoother
            .smoothed_peaks_normalized(&self.peak_meter_normalizer)
    }

    pub fn peak_meter_has_clipped(&self) -> [bool; 2] {
        self.peak_meter_smoother.has_clipped()
    }
}
