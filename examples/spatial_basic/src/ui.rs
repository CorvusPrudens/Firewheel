use std::ops::RangeInclusive;

use eframe::App;
use egui::{epaint::CircleShape, Color32, Pos2, Sense, Stroke};

use crate::system::AudioSystem;

const RANGE: RangeInclusive<f32> = -25.0..=25.0;

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
            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.spatial_basic_params.normalized_volume,
                        0.0..=2.0,
                    )
                    .step_by(0.0)
                    .text("volume"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system.spatial_basic_params.sync_volume_event(),
                );
            }

            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.spatial_basic_params.offset[0],
                        RANGE.clone(),
                    )
                    .step_by(0.0)
                    .text("x"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system.spatial_basic_params.sync_offset_event(),
                );
            }
            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.spatial_basic_params.offset[1],
                        RANGE.clone(),
                    )
                    .step_by(0.0)
                    .text("y"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system.spatial_basic_params.sync_offset_event(),
                );
            }
            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.spatial_basic_params.offset[2],
                        RANGE.clone(),
                    )
                    .step_by(0.0)
                    .text("z"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system.spatial_basic_params.sync_offset_event(),
                );
            }

            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.spatial_basic_params.damping_factor,
                        0.5..=50.0,
                    )
                    .step_by(0.0)
                    .text("damping factor")
                    .logarithmic(true),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system
                        .spatial_basic_params
                        .sync_damping_factor_event(),
                );
            }

            if ui
                .add(
                    egui::Slider::new(
                        &mut self.audio_system.spatial_basic_params.panning_threshold,
                        0.0..=1.0,
                    )
                    .step_by(0.0)
                    .text("panning threshold"),
                )
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system
                        .spatial_basic_params
                        .sync_panning_threshold_event(),
                );
            }

            let (x, yz) = self
                .audio_system
                .spatial_basic_params
                .offset
                .split_first_mut()
                .unwrap();
            let z = &mut yz[1];
            if ui
                .add(XYPad::new(x, z, RANGE.clone(), RANGE.clone(), 200.0))
                .changed()
            {
                self.audio_system.cx.queue_event_for(
                    self.audio_system.spatial_basic_node,
                    self.audio_system.spatial_basic_params.sync_offset_event(),
                );
            }
        });

        self.audio_system.update();
    }
}

struct XYPad<'a> {
    x_value: &'a mut f32,
    y_value: &'a mut f32,
    x_range: RangeInclusive<f32>,
    y_range: RangeInclusive<f32>,
    width: f32,
}

impl<'a> XYPad<'a> {
    pub fn new(
        x_value: &'a mut f32,
        y_value: &'a mut f32,
        x_range: RangeInclusive<f32>,
        y_range: RangeInclusive<f32>,
        width: f32,
    ) -> Self {
        Self {
            x_value,
            y_value,
            x_range,
            y_range,
            width,
        }
    }
}

impl<'a> egui::Widget for XYPad<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (mut response, painter) = ui.allocate_painter(
            egui::Vec2::new(self.width, self.width),
            Sense::click_and_drag(),
        );

        let x_range_span = *self.x_range.end() - *self.x_range.start();
        let y_range_span = *self.y_range.end() - *self.y_range.start();

        let x_normal = (*self.x_value - *self.x_range.start()) / x_range_span;
        let y_normal = (*self.y_value - *self.y_range.start()) / y_range_span;

        let handle_pos = Pos2::new(
            response.rect.left() + (response.rect.size().x * x_normal),
            response.rect.top() + (response.rect.size().y * y_normal),
        );

        painter.rect_stroke(
            response.rect.expand(-1.0),
            0.0,
            Stroke::new(1.0, Color32::DARK_GRAY),
        );

        painter.add(CircleShape::filled(
            response.rect.center(),
            3.0,
            Color32::DARK_GRAY,
        ));

        painter.add(CircleShape::filled(handle_pos, 3.0, Color32::DARK_GREEN));

        let point_id = response.id.with(0);
        let point_response = ui.interact(response.rect, point_id, Sense::click_and_drag());

        if let Some(point_pos) = point_response.interact_pointer_pos() {
            let x_normal = (point_pos.x - response.rect.left()) / response.rect.size().x;
            let y_normal = (point_pos.y - response.rect.top()) / response.rect.size().y;

            *self.x_value = (*self.x_range.start() + (x_range_span * x_normal))
                .clamp(*self.x_range.start(), *self.x_range.end());
            *self.y_value = (*self.y_range.start() + (y_range_span * y_normal))
                .clamp(*self.y_range.start(), *self.y_range.end());

            response.mark_changed();
        }

        response
    }
}
