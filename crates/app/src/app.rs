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

    /// Graph snapshots from *before* each change, oldest first.
    ///
    /// Whole-graph copies rather than a diff: a graph is a handful of small
    /// structs, so a clone costs almost nothing, and it makes undo impossible to
    /// get subtly wrong as new node kinds and edit paths are added.
    undo_stack: Vec<Graph>,
}

/// How many changes can be undone before the oldest is forgotten.
const UNDO_LIMIT: usize = 200;

impl CatPaws {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let app = CatPaws::fresh();
        Palette::new(app.mode).apply(&cc.egui_ctx);
        app
    }

    /// The initial state, with no egui context involved so tests can build one.
    fn fresh() -> Self {
        CatPaws {
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
            undo_stack: Vec::new(),
        }
    }

    pub fn palette(&self) -> Palette {
        Palette::new(self.mode)
    }

    /// Records the current graph so the change about to happen can be undone.
    /// Call this *before* mutating.
    pub(crate) fn push_undo(&mut self) {
        let snapshot = self.graph.clone();
        self.remember(snapshot);
    }

    /// Stores an already-taken snapshot. Used where the widget mutates in place
    /// and the "before" copy had to be taken earlier in the frame.
    pub(crate) fn remember(&mut self, snapshot: Graph) {
        self.undo_stack.push(snapshot);
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub(crate) fn undo(&mut self) {
        let Some(previous) = self.undo_stack.pop() else {
            return;
        };
        self.graph = previous;

        // Whatever was selected or being dragged may no longer exist.
        if self.selected.is_some_and(|id| self.graph.node(id).is_none()) {
            self.selected = None;
        }
        self.interaction = Interaction::Idle;

        self.program = None;
        let left = self.undo_stack.len();
        self.status = Status::Stale(format!(
            "undone — {left} step{} left",
            if left == 1 { "" } else { "s" }
        ));
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
        self.push_undo();
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
                        .size(12.5)
                        .color(palette.text_faint),
                );

                ui.add_space(16.0);

                let can_undo = self.can_undo();
                let undo_clicked = icon_button(
                    ui,
                    Icon::Undo,
                    "Undo",
                    if can_undo {
                        palette.node_body
                    } else {
                        palette.node_body.gamma_multiply(0.55)
                    },
                    if can_undo {
                        palette.text_strong
                    } else {
                        palette.text_faint
                    },
                )
                .on_hover_text("Undo the last change  (Cmd+Z / Ctrl+Z)")
                .clicked();
                if undo_clicked && can_undo {
                    self.undo();
                }

                ui.add_space(6.0);

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
                    ui.label(RichText::new("ADD NODE").size(12.5).color(palette.text_faint));
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
                    ui.label(RichText::new("VARIABLES").size(12.5).color(palette.text_faint));
                    ui.add_space(4.0);
                    self.ui_variables(ui);

                    ui.add_space(12.0);
                    ui.separator();
                    ui.label(RichText::new("SELECTED NODE").size(12.5).color(palette.text_faint));
                    ui.add_space(4.0);
                    self.ui_inspector(ui);
                });
            });
    }

    fn ui_variables(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self.graph.vars.keys().cloned().collect();
        let mut to_remove: Option<String> = None;
        let mut to_add: Option<NodeKind> = None;

        // These widgets edit the graph in place, so the "before" copy has to be
        // taken now, and is only kept if an edit actually begins this frame.
        let before = self.graph.clone();
        let mut edit_began = false;
        let mut edited = false;

        for name in &names {
            let ty = self.graph.vars[name].ty;
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(ty.label()).size(12.0));
            });
            ui.horizontal(|ui| {
                ui.label("starts at");
                let decl = self.graph.vars.get_mut(name).expect("just listed");
                // One undo entry per editing session, not one per frame or per
                // keystroke: snapshot when a drag or a focus begins. A checkbox
                // has no such session, so its single change is the whole edit.
                let (r, one_shot) = match &mut decl.initial {
                    Value::Int(v) => (ui.add(egui::DragValue::new(v)), false),
                    Value::Float(v) => (ui.add(egui::DragValue::new(v).speed(0.1)), false),
                    Value::Bool(v) => (ui.checkbox(v, ""), true),
                    Value::Str(v) => (ui.text_edit_singleline(v), false),
                };
                edit_began |= if one_shot {
                    r.changed()
                } else {
                    r.drag_started() || r.gained_focus()
                };
                edited |= r.changed();
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
            self.push_undo();
            self.graph.declare_var(name, self.new_var_type);
            self.new_var_name.clear();
            self.mark_stale();
        }

        if edit_began {
            self.remember(before);
        }
        if edited {
            self.mark_stale();
        }

        if let Some(kind) = to_add {
            self.add_node(kind);
        }
        if let Some(name) = to_remove {
            self.push_undo();
            self.graph.vars.remove(&name);
            self.mark_stale();
        }
    }

    fn ui_inspector(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        let Some(id) = self.selected else {
            ui.label(
                RichText::new("click a node to edit its value")
                    .size(12.5)
                    .color(palette.text_faint),
            );
            return;
        };
        if self.graph.node(id).is_none() {
            self.selected = None;
            return;
        }

        // Same story as the variable editors: snapshot before, keep only if an
        // edit actually starts. See `ui_variables`.
        let before = self.graph.clone();
        let mut edit_began = false;
        let mut edited = false;

        {
            let node = self.graph.node_mut(id).expect("checked just above");
            ui.label(RichText::new(node.kind.title()).strong());

            let widget = match &mut node.kind {
                NodeKind::LitInt(v) => Some((ui.add(egui::DragValue::new(v)), false)),
                NodeKind::LitFloat(v) => {
                    Some((ui.add(egui::DragValue::new(v).speed(0.1)), false))
                }
                NodeKind::LitBool(v) => Some((ui.checkbox(v, "true"), true)),
                NodeKind::LitStr(v) => Some((ui.text_edit_singleline(v), false)),
                _ => None,
            };

            match widget {
                Some((r, one_shot)) => {
                    edit_began = if one_shot {
                        r.changed()
                    } else {
                        r.drag_started() || r.gained_focus()
                    };
                    edited = r.changed();
                }
                None => {
                    ui.label(
                        RichText::new("nothing to edit on this node")
                            .size(12.5)
                            .color(palette.text_faint),
                    );
                }
            }
        }

        if edit_began {
            self.remember(before);
        }
        if edited {
            self.mark_stale();
        }

        if ui.button("Delete node").clicked() {
            self.push_undo();
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
                                    .size(12.5)
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
                                .size(12.5)
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
        // `command` is already Cmd on macOS and Ctrl elsewhere; `ctrl` is also
        // accepted so Ctrl+Z works on a Mac too.
        let undo_shortcut = ui.input(|i| {
            (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(egui::Key::Z)
        });
        // Don't steal Cmd+Z from a text field the user is typing in.
        //
        // This must ask specifically about a *text edit*. `memory().focused()` is
        // Some for any focusable widget -- including the canvas -- so guarding on
        // that disabled the shortcut permanently after the first canvas click.
        let typing = ui.ctx().text_edit_focused();
        if undo_shortcut && !typing {
            self.undo();
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use cat_paws_core::{PinKind, Side};

    fn first_node_of(app: &CatPaws, wanted: &NodeKind) -> NodeId {
        app.graph
            .nodes()
            .find(|n| std::mem::discriminant(&n.kind) == std::mem::discriminant(wanted))
            .map(|n| n.id)
            .expect("sample graph should contain this node kind")
    }

    #[test]
    fn nothing_to_undo_on_a_fresh_editor() {
        let mut app = CatPaws::fresh();
        assert!(!app.can_undo());
        let before = app.graph.node_ids();
        // Undoing with an empty stack must be a harmless no-op, not a panic.
        app.undo();
        assert_eq!(app.graph.node_ids(), before);
    }

    #[test]
    fn undo_restores_a_deleted_node_and_its_wires() {
        let mut app = CatPaws::fresh();
        let branch = first_node_of(&app, &NodeKind::Branch);
        let nodes_before = app.graph.node_ids().len();
        let links_before = app.graph.links().len();

        app.push_undo();
        app.graph.remove_node(branch);
        assert_eq!(app.graph.node_ids().len(), nodes_before - 1);
        assert!(app.graph.links().len() < links_before, "wires should have gone too");

        app.undo();
        assert_eq!(app.graph.node_ids().len(), nodes_before);
        assert_eq!(
            app.graph.links().len(),
            links_before,
            "undo must bring the wires back, not just the node"
        );
        assert!(!app.can_undo());
    }

    #[test]
    fn undo_steps_back_one_change_at_a_time() {
        let mut app = CatPaws::fresh();
        let start = app.graph.node_ids().len();

        app.add_node(NodeKind::LitInt(1));
        app.add_node(NodeKind::LitInt(2));
        app.add_node(NodeKind::LitInt(3));
        assert_eq!(app.graph.node_ids().len(), start + 3);

        app.undo();
        assert_eq!(app.graph.node_ids().len(), start + 2);
        app.undo();
        assert_eq!(app.graph.node_ids().len(), start + 1);
        app.undo();
        assert_eq!(app.graph.node_ids().len(), start);
        assert!(!app.can_undo());
    }

    #[test]
    fn undo_restores_a_moved_node_position() {
        let mut app = CatPaws::fresh();
        let branch = first_node_of(&app, &NodeKind::Branch);
        let original = app.graph.node(branch).expect("exists").pos;

        app.push_undo();
        app.graph.node_mut(branch).expect("exists").pos = (999.0, -42.0);

        app.undo();
        assert_eq!(app.graph.node(branch).expect("exists").pos, original);
    }

    #[test]
    fn undo_clears_a_selection_pointing_at_a_vanished_node() {
        let mut app = CatPaws::fresh();
        // Add a node (which selects it), then undo the addition.
        app.add_node(NodeKind::LitInt(7));
        let added = app.selected.expect("adding a node selects it");
        assert!(app.graph.node(added).is_some());

        app.undo();
        assert!(app.graph.node(added).is_none());
        assert_eq!(
            app.selected, None,
            "selection must not dangle on a node undo removed"
        );
    }

    #[test]
    fn undo_stack_is_capped() {
        let mut app = CatPaws::fresh();
        for _ in 0..(UNDO_LIMIT + 25) {
            app.push_undo();
        }
        assert_eq!(app.undo_stack.len(), UNDO_LIMIT);
    }

    /// A refused connection changes nothing, so it must not leave an undo step
    /// that appears to do nothing when used.
    #[test]
    fn undo_is_not_recorded_for_a_rejected_wire() {
        let mut app = CatPaws::fresh();
        let text = first_node_of(&app, &NodeKind::LitStr(String::new()));
        let branch = first_node_of(&app, &NodeKind::Branch);

        let from = PinRef { node: text, side: Side::Out, index: 0 };
        let to = PinRef { node: branch, side: Side::In, index: 1 };
        // Sanity: this is a string into a boolean condition.
        assert!(matches!(app.graph.pin_kind(to), Some(PinKind::Data(_))));
        assert!(app.graph.connect(from, to).is_err());
        assert!(!app.can_undo());
    }
}
