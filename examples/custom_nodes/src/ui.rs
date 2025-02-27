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

            ui.add(
                egui::Slider::new(
                    &mut self.audio_system.noise_gen_node.normalized_volume,
                    0.0..=1.0,
                )
                .text("volume"),
            );

            ui.checkbox(&mut self.audio_system.noise_gen_node.enabled, "enabled");

            self.audio_system.noise_gen_node.update_memo(
                &mut self
                    .audio_system
                    .cx
                    .event_queue(self.audio_system.noise_gen_node_id),
            );

            ui.separator();
            ui.label("Filter");

            ui.add(
                egui::Slider::new(
                    &mut self.audio_system.filter_node.normalized_volume,
                    0.0..=1.0,
                )
                .text("volume"),
            );

            ui.add(
                egui::Slider::new(
                    &mut self.audio_system.filter_node.cutoff_hz,
                    20.0..=20_000.0,
                )
                .text("cutoff")
                .logarithmic(true),
            );

            ui.checkbox(&mut self.audio_system.filter_node.enabled, "enabled");

            self.audio_system.filter_node.update_memo(
                &mut self
                    .audio_system
                    .cx
                    .event_queue(self.audio_system.filter_node_id),
            );

            ui.separator();
            ui.label("RMS meter");

            if ui
                .checkbox(&mut self.audio_system.rms_node.enabled, "enabled")
                .changed()
            {
                self.audio_system.rms_node.update_memo(
                    &mut self
                        .audio_system
                        .cx
                        .event_queue(self.audio_system.rms_node_id),
                );
            }

            let rms_value = self.audio_system.rms_node.rms_value();

            // The rms value is quite low, so scale it up to register on the meter better.
            ui.add(ProgressBar::new(rms_value * 2.0).fill(Color32::DARK_GREEN));
        });

        self.audio_system.update();

        cx.request_repaint();
    }
}
