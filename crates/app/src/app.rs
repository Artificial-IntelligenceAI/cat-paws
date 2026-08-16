//! The editor shell: panels, toolbar, variables, diagnostics and the console.

use crate::canvas::{Interaction, View};
use crate::icons::{icon_button, Icon};
use crate::theme::{Mode, Palette};
use cat_paws_core::compile::{compile as compile_graph, Diagnostic, Program};
use cat_paws_core::{vm, Category, DataType, Graph, NodeId, NodeKind, PinRef, Side, Value};
use egui::{Rect, RichText};
use std::collections::HashSet;

/// What the last compile or run did, shown in the status strip.
#[derive(Clone, Debug)]
pub enum Status {
    /// Nothing compiled yet, or the graph changed since it was.
    Stale(String),
    Ok(String),
    Failed(String),
}

pub struct CatPaws {
    pub graph: Graph,
    pub view: View,
    pub interaction: Interaction,
    pub selected: Option<NodeId>,
    pub mode: Mode,

    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
    pub output: Vec<String>,
    pub status: Status,
    pub show_listing: bool,

    new_var_name: String,
    new_var_type: DataType,
    /// Remembered so palette buttons can drop new nodes into the middle of the view.
    pub(crate) last_canvas: Rect,
}

impl CatPaws {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let app = CatPaws {
            graph: sample_graph(),
            view: View::default(),
            interaction: Interaction::Idle,
            selected: None,
            mode: Mode::Dark,
            program: None,
            diagnostics: Vec::new(),
            output: Vec::new(),
            status: Status::Stale("not compiled yet".to_string()),
            show_listing: false,
            new_var_name: String::new(),
            new_var_type: DataType::Int,
            last_canvas: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
        };
        Palette::new(app.mode).apply(&cc.egui_ctx);
        app
    }

    pub fn palette(&self) -> Palette {
        Palette::new(self.mode)
    }

    /// Called whenever the graph changes: the last compile no longer describes it.
    pub(crate) fn mark_stale(&mut self) {
        self.program = None;
        self.status = Status::Stale("graph changed — compile again".to_string());
    }

    pub(crate) fn set_wire_error(&mut self, message: &str) {
        self.status = Status::Failed(message.to_string());
    }

    /// Nodes the last compile complained about, so the canvas can outline them.
    pub(crate) fn failing_nodes(&self) -> HashSet<NodeId> {
        self.diagnostics.iter().filter_map(|d| d.node).collect()
    }

    fn compile(&mut self) -> bool {
        match compile_graph(&self.graph) {
            Ok(program) => {
                let count = program.instrs.len();
                self.program = Some(program);
                self.diagnostics.clear();
                self.status = Status::Ok(format!(
                    "compiled — {count} instruction{}",
                    if count == 1 { "" } else { "s" }
                ));
                true
            }
            Err(diags) => {
                let count = diags.len();
                self.program = None;
                self.diagnostics = diags;
                self.status = Status::Failed(format!(
                    "{count} problem{} — nothing to run",
                    if count == 1 { "" } else { "s" }
                ));
                false
            }
        }
    }

    fn compile_and_run(&mut self) {
        if !self.compile() {
            return;
        }
        let Some(program) = &self.program else { return };
        let result = vm::run(program);
        self.output = result.output;
        self.status = match result.error {
            Some(err) => Status::Failed(format!("stopped: {err}")),
            None => Status::Ok(format!(
                "ran {} step{}, printed {} line{}",
                result.steps,
                if result.steps == 1 { "" } else { "s" },
                self.output.len(),
                if self.output.len() == 1 { "" } else { "s" }
            )),
        };
    }

    /// Adds a node in the middle of the current view.
    fn add_node(&mut self, kind: NodeKind) {
        let center = self
            .view
            .to_world(self.last_canvas, self.last_canvas.center());
        let id = self.graph.add_node(kind, (center.x - 98.0, center.y - 40.0));
        self.selected = Some(id);
        self.mark_stale();
    }

    fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Cat Paws")
                        .size(17.0)
                        .strong()
                        .color(palette.text_strong),
                );
                ui.label(
                    RichText::new("blueprint editor")
                        .size(11.0)
                        .color(palette.text_faint),
                );

                ui.add_space(16.0);

                if icon_button(
                    ui,
                    Icon::Hammer,
                    "Compile",
                    palette.category_color(Category::Pure),
                    palette.on_category(),
                )
                .on_hover_text("Check the graph and build the program, without running it")
                .clicked()
                {
                    self.compile();
                }

                ui.add_space(6.0);

                if icon_button(
                    ui,
                    Icon::Play,
                    "Compile & Run",
                    palette.category_color(Category::Event),
                    palette.on_category(),
                )
                .on_hover_text("Build the program and execute it")
                .clicked()
                {
                    self.compile_and_run();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, next) = match self.mode {
                        Mode::Dark => ("Solarized Light", Mode::Light),
                        Mode::Light => ("Solarized Dark", Mode::Dark),
                    };
                    if ui.button(label).clicked() {
                        self.mode = next;
                        Palette::new(self.mode).apply(ui.ctx());
                    }
                    ui.checkbox(&mut self.show_listing, "Show compiled code");
                });
            });
            ui.add_space(4.0);
        });
    }

    fn ui_side_panel(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        egui::Panel::left("side")
            .default_size(248.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("ADD NODE").size(11.0).color(palette.text_faint));
                    ui.add_space(4.0);

                    let buttons: Vec<(&str, NodeKind)> = vec![
                        ("Event start", NodeKind::EventStart),
                        ("Branch", NodeKind::Branch),
                        ("Print", NodeKind::Print),
                        ("Less than", NodeKind::LessThan),
                        ("Number", NodeKind::LitInt(0)),
                        ("Text", NodeKind::LitStr("hello".to_string())),
                        ("True / false", NodeKind::LitBool(true)),
                    ];
                    for (label, kind) in buttons {
                        let color = palette.category_color(kind.category());
                        if ui
                            .add(egui::Button::new(RichText::new(label).color(palette.text_strong))
                                .fill(color.gamma_multiply(0.28))
                                .min_size(egui::vec2(ui.available_width(), 24.0)))
                            .clicked()
                        {
                            self.add_node(kind);
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.label(RichText::new("VARIABLES").size(11.0).color(palette.text_faint));
                    ui.add_space(4.0);
                    self.ui_variables(ui);

                    ui.add_space(12.0);
                    ui.separator();
                    ui.label(RichText::new("SELECTED NODE").size(11.0).color(palette.text_faint));
                    ui.add_space(4.0);
                    self.ui_inspector(ui);
                });
            });
    }

    fn ui_variables(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self.graph.vars.keys().cloned().collect();
        let mut to_remove: Option<String> = None;
        let mut to_add: Option<NodeKind> = None;

        for name in &names {
            let ty = self.graph.vars[name].ty;
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(ty.label()).size(10.0));
            });
            ui.horizontal(|ui| {
                ui.label("starts at");
                let decl = self.graph.vars.get_mut(name).expect("just listed");
                match &mut decl.initial {
                    Value::Int(v) => {
                        ui.add(egui::DragValue::new(v));
                    }
                    Value::Float(v) => {
                        ui.add(egui::DragValue::new(v).speed(0.1));
                    }
                    Value::Bool(v) => {
                        ui.checkbox(v, "");
                    }
                    Value::Str(v) => {
                        ui.text_edit_singleline(v);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.small_button("get").clicked() {
                    to_add = Some(NodeKind::GetVar {
                        name: name.clone(),
                        ty,
                    });
                }
                if ui.small_button("set").clicked() {
                    to_add = Some(NodeKind::SetVar {
                        name: name.clone(),
                        ty,
                    });
                }
                if ui.small_button("remove").clicked() {
                    to_remove = Some(name.clone());
                }
            });
            ui.add_space(6.0);
        }

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_var_name)
                    .hint_text("new variable")
                    .desired_width(96.0),
            );
            egui::ComboBox::from_id_salt("new_var_type")
                .selected_text(self.new_var_type.label())
                .width(78.0)
                .show_ui(ui, |ui| {
                    for ty in DataType::ALL {
                        ui.selectable_value(&mut self.new_var_type, ty, ty.label());
                    }
                });
        });
        let can_add = !self.new_var_name.trim().is_empty();
        if ui
            .add_enabled(can_add, egui::Button::new("Add variable"))
            .clicked()
        {
            let name = self.new_var_name.trim().to_string();
            self.graph.declare_var(name, self.new_var_type);
            self.new_var_name.clear();
            self.mark_stale();
        }

        if let Some(kind) = to_add {
            self.add_node(kind);
        }
        if let Some(name) = to_remove {
            self.graph.vars.remove(&name);
            self.mark_stale();
        }
    }

    fn ui_inspector(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        let Some(id) = self.selected else {
            ui.label(
                RichText::new("click a node to edit its value")
                    .size(11.0)
                    .color(palette.text_faint),
            );
            return;
        };
        let Some(node) = self.graph.node_mut(id) else {
            self.selected = None;
            return;
        };

        ui.label(RichText::new(node.kind.title()).strong());
        let mut changed = false;
        match &mut node.kind {
            NodeKind::LitInt(v) => {
                changed |= ui.add(egui::DragValue::new(v)).changed();
            }
            NodeKind::LitFloat(v) => {
                changed |= ui.add(egui::DragValue::new(v).speed(0.1)).changed();
            }
            NodeKind::LitBool(v) => {
                changed |= ui.checkbox(v, "true").changed();
            }
            NodeKind::LitStr(v) => {
                changed |= ui.text_edit_singleline(v).changed();
            }
            _ => {
                ui.label(
                    RichText::new("nothing to edit on this node")
                        .size(11.0)
                        .color(palette.text_faint),
                );
            }
        }
        if changed {
            self.mark_stale();
        }

        if ui.button("Delete node").clicked() {
            self.graph.remove_node(id);
            self.selected = None;
            self.mark_stale();
        }
    }

    fn ui_bottom(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        egui::Panel::bottom("console")
            .resizable(true)
            .default_size(168.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                let (text, color) = match &self.status {
                    Status::Stale(m) => (m.clone(), palette.text_faint),
                    Status::Ok(m) => (m.clone(), palette.category_color(Category::Event)),
                    Status::Failed(m) => (m.clone(), palette.error),
                };
                ui.label(RichText::new(text).color(color).strong());
                ui.add_space(4.0);

                let mut jump_to: Option<NodeId> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if !self.diagnostics.is_empty() {
                        for d in &self.diagnostics {
                            let label = ui.add(
                                egui::Label::new(
                                    RichText::new(format!("• {}", d.message)).color(palette.error),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if label.clicked() {
                                jump_to = d.node;
                            }
                        }
                        ui.add_space(6.0);
                    }

                    if self.show_listing {
                        if let Some(program) = &self.program {
                            ui.label(
                                RichText::new("compiled program")
                                    .size(11.0)
                                    .color(palette.text_faint),
                            );
                            for line in program.listing() {
                                ui.label(RichText::new(line).monospace().color(palette.text));
                            }
                            ui.add_space(6.0);
                        }
                    }

                    if self.output.is_empty() {
                        ui.label(
                            RichText::new("output appears here when you run")
                                .size(11.0)
                                .color(palette.text_faint),
                        );
                    } else {
                        for line in &self.output {
                            ui.label(RichText::new(line).monospace().color(palette.text_strong));
                        }
                    }
                });

                if let Some(id) = jump_to {
                    self.selected = Some(id);
                }
            });
    }
}

impl eframe::App for CatPaws {
    // egui 0.36 hands the app a `Ui` rather than a `Context`, and panels are
    // nested inside it.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui_toolbar(ui);
        self.ui_side_panel(ui);
        self.ui_bottom(ui);

        let palette = self.palette();
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::NONE.fill(palette.canvas))
            .show(ui, |ui| {
                self.last_canvas = ui.available_rect_before_wrap();
                self.ui_canvas(ui);
            });
    }
}

/// The starting graph: the example from the reference image, so there is
/// something to run the first time the app opens.
fn sample_graph() -> Graph {
    let mut g = Graph::new();
    g.declare_var("Health".to_string(), DataType::Int);
    if let Some(decl) = g.vars.get_mut("Health") {
        decl.initial = Value::Int(20);
    }

    let start = g.add_node(NodeKind::EventStart, (40.0, 150.0));
    let branch = g.add_node(NodeKind::Branch, (330.0, 140.0));
    let get = g.add_node(
        NodeKind::GetVar {
            name: "Health".to_string(),
            ty: DataType::Int,
        },
        (40.0, 380.0),
    );
    let fifty = g.add_node(NodeKind::LitInt(50), (40.0, 500.0));
    let less = g.add_node(NodeKind::LessThan, (330.0, 390.0));
    let low = g.add_node(NodeKind::Print, (640.0, 60.0));
    let fine = g.add_node(NodeKind::Print, (640.0, 250.0));
    let low_text = g.add_node(NodeKind::LitStr("low health".to_string()), (330.0, 620.0));
    let fine_text = g.add_node(NodeKind::LitStr("fine".to_string()), (330.0, 730.0));

    let mut wire = |from: NodeId, fi: usize, to: NodeId, ti: usize| {
        let _ = g.connect(
            PinRef {
                node: from,
                side: Side::Out,
                index: fi,
            },
            PinRef {
                node: to,
                side: Side::In,
                index: ti,
            },
        );
    };
    wire(start, 0, branch, 0);
    wire(get, 0, less, 0);
    wire(fifty, 0, less, 1);
    wire(less, 0, branch, 1);
    wire(branch, 0, low, 0);
    wire(branch, 1, fine, 0);
    wire(low_text, 0, low, 1);
    wire(fine_text, 0, fine, 1);

    g
}
