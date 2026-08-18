//! The editor shell: panels, toolbar, variables, diagnostics and the console.

use crate::canvas::{Interaction, View};
use crate::icons::{icon_button, Icon};
use crate::theme::{Mode, Palette};
use cat_paws_core::compile::{compile as compile_graph, Diagnostic, Program};
use cat_paws_core::graph::ArithOp;
use cat_paws_core::{Category, DataType, Graph, NodeId, NodeKind, PinRef, Side, Value};
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
    /// The module the hammer produced. This is what runs.
    pub wasm: Option<Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
    pub output: Vec<String>,
    pub status: Status,
    pub show_listing: bool,
    /// What is typed in the write box. Never saved — it makes nodes and is cleared.
    pub written: String,
    pub written_problems: Vec<cat_paws_core::written::Problem>,

    new_var_name: String,
    new_var_type: DataType,
    /// Remembered so palette buttons can drop new nodes into the middle of the view.
    pub(crate) last_canvas: Rect,
    /// The node being dragged out of the palette, if one is.
    ///
    /// Held on the app rather than passed around because the drag begins in the side
    /// panel and ends over the canvas — two different `Ui`s, which share nothing but
    /// this struct and the pointer.
    dragging_new: Option<NodeKind>,

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
    pub(crate) fn fresh() -> Self {
        CatPaws {
            graph: sample_graph(),
            view: View::default(),
            interaction: Interaction::Idle,
            selected: None,
            mode: Mode::Dark,
            program: None,
            wasm: None,
            diagnostics: Vec::new(),
            output: Vec::new(),
            status: Status::Stale("not compiled yet".to_string()),
            show_listing: false,
            written: String::new(),
            written_problems: Vec::new(),
            new_var_name: String::new(),
            new_var_type: DataType::Int,
            last_canvas: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            dragging_new: None,
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
        self.wasm = None;
        let left = self.undo_stack.len();
        self.status = Status::Stale(format!(
            "undone — {left} step{} left",
            if left == 1 { "" } else { "s" }
        ));
    }

    /// Called whenever the graph changes: the last compile no longer describes it.
    pub(crate) fn mark_stale(&mut self) {
        self.program = None;
        self.wasm = None;
        self.status = Status::Stale("graph changed — compile again".to_string());
    }

    pub(crate) fn set_wire_error(&mut self, message: &str) {
        self.status = Status::Failed(message.to_string());
    }

    /// Bring a node into the middle of the view.
    ///
    /// Selecting it is not enough on its own: the node a diagnostic points at is often
    /// off-screen, which is exactly when the reader needs help finding it, and a click
    /// that highlights something they cannot see looks like a click that did nothing.
    pub(crate) fn centre_on(&mut self, id: NodeId) {
        let Some(node) = self.graph.node(id) else { return };
        let size = crate::canvas::node_size(&node.kind);
        let middle = egui::vec2(node.pos.0 + size.x / 2.0, node.pos.1 + size.y / 2.0);
        // to_screen is `rect.min + pan + world * zoom`, so the pan that puts `middle` at
        // the centre of the canvas is the centre offset minus the scaled world position.
        self.view.pan = self.last_canvas.size() / 2.0 - middle * self.view.zoom;
    }

    /// Nodes the last compile complained about, so the canvas can outline them.
    pub(crate) fn failing_nodes(&self) -> HashSet<NodeId> {
        self.diagnostics.iter().filter_map(|d| d.node).collect()
    }

    fn compile(&mut self) -> bool {
        // Both backends run. The WebAssembly module is what actually executes; the
        // bytecode feeds the listing panel and stays available as a second opinion,
        // since two implementations are rarely wrong the same way.
        match cat_paws_core::wasm::emit(&self.graph) {
            Ok(bytes) => {
                let size = bytes.len();
                self.wasm = Some(bytes);
                self.program = compile_graph(&self.graph).ok();
                self.diagnostics.clear();
                self.status = Status::Ok(format!("compiled — {size} bytes of WebAssembly"));
                true
            }
            Err(diags) => {
                let count = diags.len();
                self.program = None;
                self.wasm = None;
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
        let Some(bytes) = self.wasm.clone() else { return };
        let outcome = crate::runner::run(&bytes);
        self.output = outcome.output;
        self.status = match outcome.error {
            Some(err) => Status::Failed(format!("stopped: {err}")),
            None => Status::Ok(format!(
                "ran {} bytes of WebAssembly, printed {} line{}",
                bytes.len(),
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

    /// How far the node hangs below and right of the pointer while carried.
    ///
    /// The same offset is used to draw it and to place it, so what you let go of is what
    /// you get — you are holding the node by its header, not by its corner.
    const GRAB: egui::Vec2 = egui::vec2(28.0, 16.0);

    /// Where a carried node is drawn, in screen space.
    pub(crate) fn carried_rect(view: &View, pointer: egui::Pos2, kind: &NodeKind) -> Rect {
        Rect::from_min_size(
            pointer - Self::GRAB * view.zoom,
            crate::canvas::node_size(kind) * view.zoom,
        )
    }

    /// Where it lands, in world space.
    ///
    /// Paired with `carried_rect` on purpose: these two have to describe the same place
    /// or the node jumps the instant you let go of it, which is the whole feel of the
    /// gesture. A test holds them together.
    pub(crate) fn dropped_position(view: &View, canvas: Rect, pointer: egui::Pos2) -> (f32, f32) {
        let at = view.to_world(canvas, pointer);
        (at.x - Self::GRAB.x, at.y - Self::GRAB.y)
    }

    /// Draw whatever is being carried out of the palette, and let go of it.
    fn ui_dragging_node(&mut self, ui: &mut egui::Ui) {
        let Some(kind) = self.dragging_new.clone() else {
            return;
        };
        let ctx = ui.ctx().clone();
        let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
            // The pointer left the window mid-drag; drop the whole gesture rather than
            // leaving a node stuck to a cursor that is not there.
            self.dragging_new = None;
            return;
        };

        let zoom = self.view.zoom;
        let palette = self.palette();
        let over_canvas = self.last_canvas.contains(pointer);

        // Painted on the foreground layer so it rides over the side panel it came from.
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("cp-carried-node"),
        ));
        let rect = Self::carried_rect(&self.view, pointer, &kind);
        // Faded, and fainter still when it would not land anywhere — the preview says
        // whether letting go will do something.
        let fade = if over_canvas { 0.85 } else { 0.35 };
        let header = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width(), crate::canvas::HEADER_H * zoom),
        );
        painter.rect_filled(rect, 6.0, palette.node_body.gamma_multiply(fade));
        painter.rect_filled(
            header,
            6.0,
            palette.category_color(kind.category()).gamma_multiply(fade),
        );
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.0, palette.node_outline),
            egui::StrokeKind::Inside,
        );
        painter.text(
            egui::pos2(header.min.x + 10.0 * zoom, header.center().y),
            egui::Align2::LEFT_CENTER,
            kind.title(),
            egui::FontId::proportional(crate::canvas::title_font_px(zoom)),
            palette.on_category().gamma_multiply(fade),
        );

        ctx.set_cursor_icon(if over_canvas {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });

        if ctx.input(|i| i.pointer.any_released()) {
            self.dragging_new = None;
            if over_canvas {
                let at = Self::dropped_position(&self.view, self.last_canvas, pointer);
                self.push_undo();
                let id = self.graph.add_node(kind, at);
                self.selected = Some(id);
                self.mark_stale();
            }
        }
        // A carried node has to keep up with the pointer between events.
        ctx.request_repaint();
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
                    ui.label(RichText::new("WRITE NODES").size(12.5).color(palette.text_faint));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.written)
                            .desired_rows(5)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("declare 'health' = integer '20'"),
                    );
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Create nodes").color(palette.text_strong),
                            )
                            .fill(palette.category_color(Category::Flow).gamma_multiply(0.28))
                            .min_size(egui::vec2(ui.available_width(), 24.0)),
                        )
                        .clicked()
                        && !self.written.trim().is_empty()
                    {
                        self.push_undo();
                        // Added beside what is already there, never replacing it, so
                        // typing can never destroy something built by hand.
                        match cat_paws_core::written::generate(&mut self.graph, &self.written) {
                            Ok(made) => {
                                self.written.clear();
                                self.written_problems.clear();
                                self.mark_stale();
                                self.status = Status::Ok(format!(
                                    "made {} node{}",
                                    made.len(),
                                    if made.len() == 1 { "" } else { "s" }
                                ));
                            }
                            Err(problems) => {
                                self.status = Status::Failed(format!(
                                    "{} problem{} in what you wrote",
                                    problems.len(),
                                    if problems.len() == 1 { "" } else { "s" }
                                ));
                                self.written_problems = problems;
                            }
                        }
                    }
                    for p in &self.written_problems {
                        ui.add(egui::Label::new(
                            RichText::new(format!("line {}: {}", p.line, p.message))
                                .size(12.0)
                                .color(palette.error),
                        ));
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.add(egui::Label::new(
                                RichText::new(format!("try: {}", p.fix))
                                    .size(11.5)
                                    .color(palette.text_faint),
                            ));
                        });
                    }

                    ui.add_space(10.0);
                    ui.label(RichText::new("ADD NODE").size(12.5).color(palette.text_faint));
                    ui.add_space(4.0);

                    let buttons: Vec<(&str, NodeKind)> = vec![
                        ("Event start", NodeKind::EventStart),
                        ("Branch", NodeKind::Branch),
                        ("Repeat", NodeKind::Repeat),
                        ("Print text", NodeKind::Print { ty: DataType::Str }),
                        ("Print number", NodeKind::Print { ty: DataType::Int }),
                        ("Less than", NodeKind::LessThan),
                        (
                            "Add +",
                            NodeKind::Arith {
                                op: ArithOp::Add,
                                ty: DataType::Int,
                            },
                        ),
                        (
                            "Subtract −",
                            NodeKind::Arith {
                                op: ArithOp::Subtract,
                                ty: DataType::Int,
                            },
                        ),
                        (
                            "Multiply ×",
                            NodeKind::Arith {
                                op: ArithOp::Multiply,
                                ty: DataType::Int,
                            },
                        ),
                        (
                            "Divide ÷",
                            NodeKind::Arith {
                                op: ArithOp::Divide,
                                ty: DataType::Int,
                            },
                        ),
                        ("Number", NodeKind::LitInt(0)),
                        ("Text", NodeKind::LitStr("hello".to_string())),
                        ("True / false", NodeKind::LitBool(true)),
                    ];
                    for (label, kind) in buttons {
                        let color = palette.category_color(kind.category());
                        let r = ui.add(
                            egui::Button::new(RichText::new(label).color(palette.text_strong))
                                .fill(color.gamma_multiply(0.28))
                                .min_size(egui::vec2(ui.available_width(), 24.0))
                                .sense(egui::Sense::click_and_drag()),
                        );
                        // Drag to place it where you let go; click to drop it in the
                        // middle of the view. Clicking still works because a button that
                        // does nothing when clicked reads as broken, whatever it says.
                        if r.drag_started() {
                            self.dragging_new = Some(kind.clone());
                        } else if r.clicked() {
                            self.add_node(kind);
                        }
                        if r.hovered() || r.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
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
                        let show_rule = rules_to_show(&self.diagnostics);
                        for (i, d) in self.diagnostics.iter().enumerate() {
                            // What went wrong, then what to do about it. The second line
                            // is quieter so the eye reads the problem first, but it is
                            // always there — a beginner needs it more than the first.
                            let code = d.code.render();
                            let rule = d.code.rule();
                            if show_rule[i] {
                                ui.horizontal_wrapped(|ui| {
                                    ui.add_space(10.0);
                                    ui.add(egui::Label::new(
                                        RichText::new(format!("the rule: {rule}"))
                                            .size(12.5)
                                            .italics()
                                            .color(palette.text_faint),
                                    ));
                                });
                            }

                            let clickable = d.node.is_some();
                            let label = ui.add(
                                egui::Label::new(
                                    RichText::new(format!("• {}", d.message)).color(palette.error),
                                )
                                .sense(if clickable {
                                    egui::Sense::click()
                                } else {
                                    egui::Sense::hover()
                                }),
                            );
                            if clickable {
                                // A plain label gives no sign it can be clicked. The
                                // pointer changes and the text underlines on hover, so
                                // the link is discoverable rather than a secret.
                                let label = label.on_hover_cursor(egui::CursorIcon::PointingHand);
                                if label.hovered() {
                                    let r = label.rect;
                                    ui.painter().line_segment(
                                        [
                                            egui::pos2(r.left(), r.bottom() - 1.0),
                                            egui::pos2(r.right(), r.bottom() - 1.0),
                                        ],
                                        egui::Stroke::new(1.0, palette.error),
                                    );
                                }
                                if label.clicked() {
                                    jump_to = d.node;
                                }
                            }
                            if !d.fix.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(10.0);
                                    ui.add(egui::Label::new(
                                        RichText::new(format!("try: {}", d.fix))
                                            .size(12.5)
                                            .color(palette.text_faint),
                                    ));
                                });
                            }
                            // The code is a handle to search for or look up, so it is
                            // present but quiet — a beginner reads the sentence, and
                            // only needs this when they want to go and find out more.
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.add(egui::Label::new(
                                    RichText::new(code)
                                        .size(11.0)
                                        .monospace()
                                        .color(palette.text_faint),
                                ));
                            });
                            ui.add_space(3.0);
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
                    self.centre_on(id);
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

        // After the panels, so the carried node is drawn over all of them.
        self.ui_dragging_node(ui);
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
    let low = g.add_node(NodeKind::Print { ty: DataType::Str }, (640.0, 60.0));
    let fine = g.add_node(NodeKind::Print { ty: DataType::Str }, (640.0, 250.0));
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

#[cfg(test)]
mod centre_tests {
    use super::*;
    use cat_paws_core::{DataType, NodeKind};

    /// Centring is pure arithmetic on the view, so it is testable without a window.
    #[test]
    fn centring_puts_a_node_in_the_middle_of_the_canvas() {
        let mut app = CatPaws::fresh();
        app.last_canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let far = app.graph.add_node(NodeKind::LitInt(1), (4000.0, 3000.0));

        app.centre_on(far);

        let node = app.graph.node(far).unwrap();
        let size = crate::canvas::node_size(&node.kind);
        let middle = egui::pos2(node.pos.0 + size.x / 2.0, node.pos.1 + size.y / 2.0);
        let on_screen = app.view.to_screen(app.last_canvas, middle);
        let want = app.last_canvas.center();
        assert!(
            (on_screen - want).length() < 0.5,
            "expected the node at {want:?}, it landed at {on_screen:?}"
        );
    }

    #[test]
    fn centring_works_at_any_zoom() {
        for zoom in [0.25_f32, 0.5, 1.0, 2.0] {
            let mut app = CatPaws::fresh();
            app.last_canvas =
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
            app.view.zoom = zoom;
            let id = app.graph.add_node(
                NodeKind::GetVar {
                    name: "x".into(),
                    ty: DataType::Int,
                },
                (-900.0, 1200.0),
            );

            app.centre_on(id);

            let node = app.graph.node(id).unwrap();
            let size = crate::canvas::node_size(&node.kind);
            let middle = egui::pos2(node.pos.0 + size.x / 2.0, node.pos.1 + size.y / 2.0);
            let on_screen = app.view.to_screen(app.last_canvas, middle);
            assert!(
                (on_screen - app.last_canvas.center()).length() < 0.5,
                "zoom {zoom}: landed at {on_screen:?}"
            );
        }
    }
}

/// Which diagnostics should carry their rule.
///
/// The first time a code appears in a run, and not on repeats. Meeting a rule once
/// teaches it; eight copies of the same paragraph is a wall to scroll past.
///
/// Pure, so the behaviour is testable without a window.
pub(crate) fn rules_to_show(diagnostics: &[Diagnostic]) -> Vec<bool> {
    let mut seen: Vec<String> = Vec::new();
    diagnostics
        .iter()
        .map(|d| {
            let code = d.code.render();
            if d.code.rule().is_empty() || seen.contains(&code) {
                false
            } else {
                seen.push(code);
                true
            }
        })
        .collect()
}

#[cfg(test)]
mod rule_display_tests {
    use super::*;
    use cat_paws_core::compile::{EMPTY_PIN, NO_START, NO_SUCH_VAR};

    fn diag(code: cat_paws_core::Code) -> Diagnostic {
        Diagnostic::global(code, "something", "do something")
    }

    #[test]
    fn a_rule_is_stated_once_per_run() {
        let diags = [diag(EMPTY_PIN), diag(EMPTY_PIN), diag(EMPTY_PIN)];
        assert_eq!(rules_to_show(&diags), vec![true, false, false]);
    }

    #[test]
    fn each_different_code_gets_its_own_rule() {
        let diags = [
            diag(EMPTY_PIN),
            diag(NO_START),
            diag(EMPTY_PIN),
            diag(NO_SUCH_VAR),
        ];
        assert_eq!(rules_to_show(&diags), vec![true, true, false, true]);
    }
}

#[cfg(test)]
mod dragging_out_of_the_palette {
    use super::*;
    use crate::canvas::node_size;

    fn canvas() -> Rect {
        Rect::from_min_size(egui::pos2(240.0, 70.0), egui::vec2(900.0, 640.0))
    }

    /// The node has to land exactly where the preview was, or it jumps the moment you
    /// let go — which is the entire feel of dragging something out.
    ///
    /// The two are computed in different spaces: the preview in screen pixels, the drop
    /// in world units. They agree only because the grab offset is scaled by the zoom in
    /// one and not the other, which is precisely the sort of thing that survives review
    /// and fails in the hand.
    #[test]
    fn what_you_let_go_of_is_where_it_lands() {
        let kind = NodeKind::Arith { op: ArithOp::Add, ty: DataType::Int };
        for zoom in [0.4_f32, 0.75, 1.0, 1.6, 2.5] {
            for pointer in [
                egui::pos2(300.0, 120.0),
                egui::pos2(700.0, 400.0),
                egui::pos2(1100.0, 690.0),
            ] {
                let view = View { zoom, ..Default::default() };
                let preview = CatPaws::carried_rect(&view, pointer, &kind);
                let dropped = CatPaws::dropped_position(&view, canvas(), pointer);
                let landed = view.to_screen(canvas(), egui::pos2(dropped.0, dropped.1));
                assert!(
                    (landed - preview.min).length() < 0.01,
                    "at zoom {zoom} from {pointer:?}: preview at {:?}, landed at {landed:?}",
                    preview.min
                );
            }
        }
    }

    /// You hold it by the header, not by a corner — otherwise the pointer sits in empty
    /// space above the node and the drop feels like it went somewhere else.
    #[test]
    fn the_pointer_holds_the_node_by_its_header() {
        let kind = NodeKind::LitInt(0);
        let view = View::default();
        let pointer = egui::pos2(600.0, 300.0);
        let rect = CatPaws::carried_rect(&view, pointer, &kind);
        assert!(rect.contains(pointer), "the pointer should be on the node it is carrying");
        assert!(
            pointer.y - rect.min.y < crate::canvas::HEADER_H,
            "the pointer should be within the header, not down in the body"
        );
    }

    /// Dropping a bigger node must not place it differently from a small one — the grab
    /// point is a fixed offset, not a fraction of the size.
    #[test]
    fn every_node_is_held_at_the_same_spot() {
        let view = View::default();
        let pointer = egui::pos2(500.0, 250.0);
        let small = CatPaws::carried_rect(&view, pointer, &NodeKind::LitBool(true));
        let big = CatPaws::carried_rect(&view, pointer, &NodeKind::Branch);
        assert_eq!(small.min, big.min);
        assert_ne!(
            node_size(&NodeKind::LitBool(true)),
            node_size(&NodeKind::Branch),
            "this test is meaningless if the two are the same size"
        );
    }

    /// Clicking still works. A button that does nothing when clicked reads as broken,
    /// whatever the label says.
    #[test]
    fn clicking_a_palette_button_still_adds_a_node() {
        let mut app = CatPaws::fresh();
        let before = app.graph.node_ids().len();
        app.add_node(NodeKind::LitInt(7));
        assert_eq!(app.graph.node_ids().len(), before + 1);
    }
}
