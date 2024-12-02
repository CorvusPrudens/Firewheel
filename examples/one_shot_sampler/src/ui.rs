use eframe::App;

use crate::system::{AudioSystem, SAMPLE_PATHS};

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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.menu_button("Menu", |ui| {
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                        }
                    });
                    ui.add_space(16.0);
                }

                egui::widgets::global_theme_preference_switch(ui);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            for i in 0..SAMPLE_PATHS.len() {
                ui.horizontal(|ui| {
                    let txt = match i {
                        0 => "swish",
                        1 => "birds",
                        _ => "beep",
                    };

                    ui.label(txt);

                    if ui.button("Play").clicked() {
                        self.audio_system.play(i);
                    }

                    if self.audio_system.samplers[i].paused {
                        if ui.button("Resume").clicked() {
                            self.audio_system.resume(i);
                        }
                    } else {
                        if ui.button("Pause").clicked() {
                            self.audio_system.pause(i);
                        }
                    }

                    if ui.button("Stop").clicked() {
                        self.audio_system.stop(i);
                    }

                    ui.checkbox(
                        &mut self.audio_system.samplers[i].stop_other_voices,
                        "stop other voices",
                    );

                    ui.add(
                        egui::Slider::new(&mut self.audio_system.samplers[i].volume, 0.0..=100.0)
                            .text("volume"),
                    );
                });

                ui.separator();
            }
        });

        self.audio_system.update();

        if !self.audio_system.is_activated() {
            // TODO: Don't panic.
            panic!("Audio system disconnected");
        }
    }
}
