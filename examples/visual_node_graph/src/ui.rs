use eframe::App;
use egui::{Color32, Id, Ui};
use egui_snarl::{
    ui::{AnyPins, PinInfo, SnarlStyle, SnarlViewer},
    InPin, InPinId, OutPin, OutPinId, Snarl,
};
use firewheel::nodes::{
    beep_test::BeepTestParams, volume::VolumeParams, volume_pan::VolumePanParams,
};

use crate::system::{AudioSystem, NodeType};

const CABLE_COLOR: Color32 = Color32::from_rgb(0xb0, 0x00, 0xb0);

pub enum GuiAudioNode {
    #[allow(unused)]
    SystemIn,
    SystemOut,
    BeepTest {
        id: firewheel::node::NodeID,
        params: BeepTestParams,
    },
    StereoToMono {
        id: firewheel::node::NodeID,
    },
    VolumeMono {
        id: firewheel::node::NodeID,
        params: VolumeParams,
    },
    VolumeStereo {
        id: firewheel::node::NodeID,
        params: VolumeParams,
    },
    VolumePan {
        id: firewheel::node::NodeID,
        params: VolumePanParams,
    },
}

impl GuiAudioNode {
    fn node_id(&self, audio_system: &AudioSystem) -> firewheel::node::NodeID {
        match self {
            &Self::SystemIn => audio_system.graph_in_node(),
            &Self::SystemOut => audio_system.graph_out_node(),
            &Self::BeepTest { id, .. } => id,
            &Self::StereoToMono { id } => id,
            &Self::VolumeMono { id, .. } => id,
            &Self::VolumeStereo { id, .. } => id,
            &Self::VolumePan { id, .. } => id,
        }
    }

    fn title(&self) -> String {
        match self {
            &Self::SystemIn => "System In",
            &Self::SystemOut => "System Out",
            &Self::BeepTest { .. } => "Beep Test",
            &Self::StereoToMono { .. } => "Stereo To Mono",
            &Self::VolumeMono { .. } => "Volume (Mono)",
            &Self::VolumeStereo { .. } => "Volume (Stereo)",
            &Self::VolumePan { .. } => "Volume & Pan",
        }
        .into()
    }

    fn num_inputs(&self) -> usize {
        match self {
            &Self::SystemIn => 0,
            &Self::SystemOut => 2,
            &Self::BeepTest { .. } => 0,
            &Self::StereoToMono { .. } => 2,
            &Self::VolumeMono { .. } => 1,
            &Self::VolumeStereo { .. } => 2,
            &Self::VolumePan { .. } => 2,
        }
    }

    fn num_outputs(&self) -> usize {
        match self {
            &Self::SystemIn => 1,
            &Self::SystemOut => 0,
            &Self::BeepTest { .. } => 1,
            &Self::StereoToMono { .. } => 1,
            &Self::VolumeMono { .. } => 1,
            &Self::VolumeStereo { .. } => 2,
            &Self::VolumePan { .. } => 2,
        }
    }
}

struct DemoViewer<'a> {
    audio_system: &'a mut AudioSystem,
}

impl<'a> DemoViewer<'a> {
    fn remove_edge(&mut self, from: OutPinId, to: InPinId, snarl: &mut Snarl<GuiAudioNode>) {
        let Some(src_node) = snarl.get_node(from.node) else {
            return;
        };
        let Some(dst_node) = snarl.get_node(to.node) else {
            return;
        };
        let src_node = src_node.node_id(&self.audio_system);
        let dst_node = dst_node.node_id(&self.audio_system);

        self.audio_system
            .disconnect(src_node, dst_node, from.output as u32, to.input as u32);

        snarl.disconnect(from, to);
    }
}

impl<'a> SnarlViewer<GuiAudioNode> for DemoViewer<'a> {
    fn drop_inputs(&mut self, pin: &InPin, snarl: &mut Snarl<GuiAudioNode>) {
        for from in pin.remotes.iter() {
            self.remove_edge(*from, pin.id, snarl);
        }
    }

    fn drop_outputs(&mut self, pin: &OutPin, snarl: &mut Snarl<GuiAudioNode>) {
        for to in pin.remotes.iter() {
            self.remove_edge(pin.id, *to, snarl);
        }
    }

    fn disconnect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<GuiAudioNode>) {
        self.remove_edge(from.id, to.id, snarl);
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<GuiAudioNode>) {
        let src_node = snarl
            .get_node(from.id.node)
            .unwrap()
            .node_id(&self.audio_system);
        let dst_node = snarl
            .get_node(to.id.node)
            .unwrap()
            .node_id(&self.audio_system);

        if let Err(e) = self.audio_system.connect(
            src_node,
            dst_node,
            from.id.output as u32,
            to.id.input as u32,
        ) {
            log::error!("{}", e);
            return;
        }

        snarl.connect(from.id, to.id);
    }

    fn title(&mut self, node: &GuiAudioNode) -> String {
        node.title()
    }

    fn inputs(&mut self, node: &GuiAudioNode) -> usize {
        node.num_inputs()
    }

    fn outputs(&mut self, node: &GuiAudioNode) -> usize {
        node.num_outputs()
    }

    fn show_input(
        &mut self,
        _pin: &InPin,
        _ui: &mut Ui,
        _scale: f32,
        _snarl: &mut Snarl<GuiAudioNode>,
    ) -> PinInfo {
        PinInfo::square().with_fill(CABLE_COLOR)
    }

    fn show_output(
        &mut self,
        _pin: &OutPin,
        _ui: &mut Ui,
        _scale: f32,
        _snarl: &mut Snarl<GuiAudioNode>,
    ) -> PinInfo {
        PinInfo::square().with_fill(CABLE_COLOR)
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<GuiAudioNode>) -> bool {
        true
    }

    fn show_graph_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<GuiAudioNode>,
    ) {
        ui.label("Add node");
        if ui.button("Beep Test").clicked() {
            let node = self.audio_system.add_node(NodeType::BeepTest);
            snarl.insert_node(pos, node);
            ui.close_menu();
        }
        if ui.button("Stereo To Mono").clicked() {
            let node = self.audio_system.add_node(NodeType::StereoToMono);
            snarl.insert_node(pos, node);
            ui.close_menu();
        }
        if ui.button("Volume (mono)").clicked() {
            let node = self.audio_system.add_node(NodeType::VolumeMono);
            snarl.insert_node(pos, node);
            ui.close_menu();
        }
        if ui.button("Volume (stereo)").clicked() {
            let node = self.audio_system.add_node(NodeType::VolumeStereo);
            snarl.insert_node(pos, node);
            ui.close_menu();
        }
        if ui.button("Volume & Pan").clicked() {
            let node = self.audio_system.add_node(NodeType::VolumePan);
            snarl.insert_node(pos, node);
            ui.close_menu();
        }
    }

    fn has_dropped_wire_menu(
        &mut self,
        _src_pins: AnyPins,
        _snarl: &mut Snarl<GuiAudioNode>,
    ) -> bool {
        false
    }

    fn has_node_menu(&mut self, _node: &GuiAudioNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: egui_snarl::NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<GuiAudioNode>,
    ) {
        let n = snarl.get_node(node).unwrap();

        match &n {
            GuiAudioNode::SystemIn | GuiAudioNode::SystemOut => {}
            _ => {
                ui.label("Node menu");
                if ui.button("Remove").clicked() {
                    self.audio_system.remove_node(n.node_id(&self.audio_system));
                    snarl.remove_node(node);
                    ui.close_menu();
                }
            }
        }
    }

    fn has_on_hover_popup(&mut self, _: &GuiAudioNode) -> bool {
        false
    }

    fn has_body(&mut self, node: &GuiAudioNode) -> bool {
        match node {
            GuiAudioNode::VolumeMono { .. }
            | GuiAudioNode::VolumeStereo { .. }
            | GuiAudioNode::VolumePan { .. }
            | GuiAudioNode::BeepTest { .. } => true,
            _ => false,
        }
    }

    fn show_body(
        &mut self,
        node: egui_snarl::NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<GuiAudioNode>,
    ) {
        match snarl.get_node_mut(node).unwrap() {
            GuiAudioNode::BeepTest { id, params } => {
                ui.vertical(|ui| {
                    if ui
                        .add(
                            egui::Slider::new(&mut params.normalized_volume, 0.0..=1.0)
                                .text("volume"),
                        )
                        .changed()
                    {
                        self.audio_system
                            .queue_event(*id, params.sync_volume_event());
                    }
                    if ui
                        .add(
                            egui::Slider::new(&mut params.freq_hz, 20.0..=20_000.0)
                                .logarithmic(true)
                                .text("frequency"),
                        )
                        .changed()
                    {
                        self.audio_system
                            .queue_event(*id, params.sync_freq_hz_event());
                    }
                    if ui.checkbox(&mut params.enabled, "enabled").changed() {
                        self.audio_system
                            .queue_event(*id, params.sync_enabled_event());
                    }
                });
            }
            GuiAudioNode::VolumeMono { id, params } => {
                if ui
                    .add(egui::Slider::new(&mut params.normalized_volume, 0.0..=2.0).text("volume"))
                    .changed()
                {
                    self.audio_system
                        .queue_event(*id, params.sync_volume_event());
                }
            }
            GuiAudioNode::VolumeStereo { id, params } => {
                if ui
                    .add(egui::Slider::new(&mut params.normalized_volume, 0.0..=2.0).text("volume"))
                    .changed()
                {
                    self.audio_system
                        .queue_event(*id, params.sync_volume_event());
                }
            }
            GuiAudioNode::VolumePan { id, params } => {
                ui.vertical(|ui| {
                    if ui
                        .add(
                            egui::Slider::new(&mut params.normalized_volume, 0.0..=2.0)
                                .text("volume"),
                        )
                        .changed()
                    {
                        self.audio_system
                            .queue_event(*id, params.sync_volume_event());
                    }
                    if ui
                        .add(egui::Slider::new(&mut params.pan, -1.0..=1.0).text("pan"))
                        .changed()
                    {
                        self.audio_system.queue_event(*id, params.sync_pan_event());
                    }
                });
            }
            _ => {}
        }
    }
}

pub struct DemoApp {
    snarl: Snarl<GuiAudioNode>,
    style: SnarlStyle,
    snarl_ui_id: Option<Id>,
    audio_system: AudioSystem,
}

impl DemoApp {
    pub fn new() -> Self {
        let mut snarl = Snarl::new();
        let style = SnarlStyle::new();

        snarl.insert_node(egui::Pos2 { x: 0.0, y: 0.0 }, GuiAudioNode::SystemOut);

        DemoApp {
            snarl,
            style,
            snarl_ui_id: None,
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

                if ui.button("Clear All").clicked() {
                    self.audio_system.reset();

                    self.snarl = Default::default();
                    self.snarl
                        .insert_node(egui::Pos2 { x: 0.0, y: 0.0 }, GuiAudioNode::SystemOut);
                }
            });
        });

        egui::CentralPanel::default().show(cx, |ui| {
            self.snarl_ui_id = Some(ui.id());

            self.snarl.show(
                &mut DemoViewer {
                    audio_system: &mut self.audio_system,
                },
                &self.style,
                "snarl",
                ui,
            );
        });

        self.audio_system.update();

        if !self.audio_system.is_activated() {
            // TODO: Don't panic.
            panic!("Audio system disconnected");
        }
    }
}
