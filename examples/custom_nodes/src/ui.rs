use eframe::App;
use egui::{Color32, ProgressBar};

use crate::system::AudioSystem;

pub struct DemoApp {
    audio_system: AudioSystem,
}

impl DemoApp {
    pub fn new() -> Self {
        Self {
            audio_system: AudioSystem::new(),
        }
    }
}

impl App for DemoApp {
    fn update(&mut self, cx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(cx, |ui| {
            egui::menu::bar(ui, |ui| {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.menu_button("Menu", |ui| {
                        if ui.button("Quit").clicked() {
                            cx.send_viewport_cmd(egui::ViewportCommand::Close)
                        }
                    });
                    ui.add_space(16.0);
                }

                egui::widgets::global_theme_preference_switch(ui);
            });
        });

        egui::CentralPanel::default().show(cx, |ui| {
            ui.label("Noise gen");

            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.noise_gen_params.normalized_volume,
                        0.0..=1.0,
                    )
                    .text("volume"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.noise_gen_node,
                    self.audio_system.noise_gen_params.sync_volume_event(),
                );
            }

            if ui
                .checkbox(&mut self.audio_system.noise_gen_params.enabled, "enabled")
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.noise_gen_node,
                    self.audio_system.noise_gen_params.sync_enabled_event(),
                );
            }

            ui.separator();
            ui.label("Filter");

            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.filter_params.normalized_volume,
                        0.0..=1.0,
                    )
                    .text("volume"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.filter_node,
                    self.audio_system.filter_params.sync_volume_event(),
                );
            }

            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.filter_params.cutoff_hz,
                        20.0..=20_000.0,
                    )
                    .text("cutoff")
                    .logarithmic(true),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.filter_node,
                    self.audio_system.filter_params.sync_cutoff_event(),
                );
            }

            if ui
                .checkbox(&mut self.audio_system.filter_params.enabled, "enabled")
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.filter_node,
                    self.audio_system.filter_params.sync_enabled_event(),
                );
            }

            ui.separator();
            ui.label("RMS meter");

            if ui
                .checkbox(&mut self.audio_system.rms_params.enabled, "enabled")
                .changed()
            {
                self.audio_system
                    .rms_handle
                    .sync_params(self.audio_system.rms_params);
            }

            let rms_value = self.audio_system.rms_handle.rms_value();

            // The rms value is quite low, so scale it up to register on the meter better.
            ui.add(ProgressBar::new(rms_value * 2.0).fill(Color32::DARK_GREEN));
        });

        self.audio_system.update();

        cx.request_repaint();
    }
}
