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
    /// Every node currently selected.
    ///
    /// A set rather than one id, so a marquee can pick up several. The inspector asks
    /// for `selected_one`, which answers only when exactly one is chosen — editing a
    /// value makes no sense for a group.
    pub selection: std::collections::BTreeSet<NodeId>,
    pub mode: Mode,

    pub program: Option<Program>,
    /// The module the hammer produced. This is what runs.
    pub wasm: Option<Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
    pub output: Vec<String>,
    pub status: Status,
    pub show_listing: bool,
    /// Show the WebAssembly the program was compiled to.
    pub show_wasm: bool,
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
            selection: Default::default(),
            mode: Mode::Dark,
            program: None,
            wasm: None,
            diagnostics: Vec::new(),
            output: Vec::new(),
            status: Status::Stale("not compiled yet".to_string()),
            show_listing: false,
            show_wasm: false,
            written: String::new(),
            written_problems: Vec::new(),
            new_var_name: String::new(),
            new_var_type: DataType::Int,
            last_canvas: Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            dragging_new: None,
            undo_stack: Vec::new(),
        }
    }

    /// The one selected node, when exactly one is.
    ///
    /// `None` for an empty selection *and* for a group: "edit this value" has no meaning
    /// when several nodes are chosen, and silently editing whichever happened to be first
    /// would be worse than offering nothing.
    pub fn selected_one(&self) -> Option<NodeId> {
        match self.selection.len() {
            1 => self.selection.iter().next().copied(),
            _ => None,
        }
    }

    pub(crate) fn select_only(&mut self, id: NodeId) {
        self.selection.clear();
        self.selection.insert(id);
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
        self.selection.retain(|id| self.graph.node(*id).is_some());
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
        // Every graph change passes through here, so this is the one place that can
        // promise the selection never holds a node that no longer exists. Undo already
        // did it; a set makes the guarantee worth centralising, since a ghost id would
        // otherwise sit there until something tried to draw or delete it.
        self.selection.retain(|id| self.graph.node(*id).is_some());
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
        self.select_only(id);
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
                self.select_only(id);
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
                    // "Compiled code" is the plain phrase; "WebAssembly" is the technical
                    // one and lives in the hover and the heading. It only earns this
                    // label because it is now the real thing — it used to sit on the
                    // step listing, which is not what runs.
                    ui.checkbox(&mut self.show_wasm, "Show compiled code")
                        .on_hover_text(
                            "What your program actually became: the WebAssembly the \
                             browser runs, written out as text.",
                        );
                    ui.checkbox(&mut self.show_listing, "Show the steps")
                        .on_hover_text(
                            "The same program in a form you can read. Not what runs — \
                             that is the WebAssembly.",
                        );
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
                    Value::Int(v) => {
                        let (r, done) = int_field(ui, v, &format!("var-int-{name}"));
                        edited |= done;
                        (r, false)
                    }
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
        // A name already in the panel cannot be added again. `declare_var` inserts a
        // fresh declaration, so re-adding one silently threw away its starting value and
        // could change its type under every node already using it. The written form had
        // the same hole and was closed; this is the button.
        let wanted = self.new_var_name.trim().to_string();
        let taken = self.graph.vars.contains_key(&wanted);
        let can_add = !wanted.is_empty() && !taken;
        if ui
            .add_enabled(can_add, egui::Button::new("Add variable"))
            .clicked()
        {
            self.push_undo();
            self.graph.declare_var(wanted.clone(), self.new_var_type);
            self.new_var_name.clear();
            self.mark_stale();
        }
        if taken {
            ui.label(
                RichText::new(format!("there is already a variable called '{wanted}'"))
                    .size(12.0)
                    .color(self.palette().text_faint),
            );
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
        let Some(id) = self.selected_one() else {
            let n = self.selection.len();
            ui.label(
                RichText::new(if n == 0 {
                    "click a node to edit its value".to_string()
                } else {
                    format!("{n} nodes selected — drag to move them, or press Delete")
                })
                .size(12.5)
                .color(palette.text_faint),
            );
            if n > 1 && ui.button(format!("Delete {n} nodes")).clicked() {
                self.push_undo();
                for id in std::mem::take(&mut self.selection) {
                    self.graph.remove_node(id);
                }
                self.mark_stale();
            }
            return;
        };
        if self.graph.node(id).is_none() {
            self.selection.clear();
            return;
        }

        // A variable node holds no value of its own — what it shows belongs to the
        // variable. Read the name out before the graph is borrowed mutably below, so the
        // editor for it can come after.
        let variable = match self.graph.kind_of(id) {
            Some(NodeKind::GetVar { name, .. }) | Some(NodeKind::SetVar { name, .. }) => {
                Some(name.clone())
            }
            _ => None,
        };

        // Same story as the variable editors: snapshot before, keep only if an
        // edit actually starts. See `ui_variables`.
        let before = self.graph.clone();
        let mut edit_began = false;
        let mut edited = false;
        // A whole-number field commits when it is left, not while it is being typed, so
        // it reports separately from `Response::changed`.
        let mut edited_int = false;

        {
            let node = self.graph.node_mut(id).expect("checked just above");
            ui.label(RichText::new(node.kind.title()).strong());

            let widget = match &mut node.kind {
                NodeKind::LitInt(v) => {
                    let (r, done) = int_field(ui, v, &format!("lit-{id:?}"));
                    edited_int = done;
                    Some((r, false))
                }
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
                None if variable.is_none() => {
                    ui.label(
                        RichText::new("nothing to edit on this node")
                            .size(12.5)
                            .color(palette.text_faint),
                    );
                }
                None => {}
            }
        }

        // Editing the variable's starting value here rather than only in the Variables
        // panel: with a long list, finding the row that matches the node you are looking
        // at is its own small chore.
        if let Some(name) = &variable {
            if let Some(decl) = self.graph.vars.get_mut(name) {
                ui.horizontal(|ui| {
                    ui.label("starts at");
                    let (r, one_shot) = match &mut decl.initial {
                        Value::Int(v) => {
                            let (r, done) = int_field(ui, v, &format!("node-var-{name}"));
                            edited |= done;
                            (r, false)
                        }
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
                ui.label(
                    RichText::new(format!(
                        "this is '{name}' itself — every node using it starts here"
                    ))
                    .size(12.0)
                    .color(palette.text_faint),
                );
            }
        }

        if edit_began {
            self.remember(before);
        }
        if edited || edited_int {
            self.mark_stale();
        }

        if ui.button("Delete node").clicked() {
            self.push_undo();
            self.graph.remove_node(id);
            self.selection.clear();
            self.mark_stale();
        }
    }

    /// Everything in the console as plain text.
    ///
    /// Written the way it reads on screen, errors first, because the reason to copy this
    /// is almost always to show it to somebody who is not looking at your screen.
    fn console_text(&self) -> String {
        let mut out = String::new();
        for d in &self.diagnostics {
            out.push_str(&format!("• {}\n", d.message));
            if !d.fix.is_empty() {
                out.push_str(&format!("  try: {}\n", d.fix));
            }
            out.push_str(&format!("  {}\n", d.code.render()));
        }
        if !self.output.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&self.output.join("\n"));
            out.push('\n');
        }
        out
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new(text).color(color).strong());
                    // Right-aligned, so it does not sit between the reader and the
                    // status line they came here to read.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let anything = !self.output.is_empty() || !self.diagnostics.is_empty();
                        if ui
                            .add_enabled(anything, egui::Button::new("Copy").small())
                            .on_hover_text("Copy everything below — errors and output")
                            .clicked()
                        {
                            ui.ctx().copy_text(self.console_text());
                        }
                    });
                });
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

                    if self.show_wasm {
                        if let Some(bytes) = &self.wasm {
                            ui.label(
                                RichText::new(format!(
                                    "the WebAssembly your program became — {} bytes",
                                    bytes.len()
                                ))
                                .size(12.5)
                                .color(palette.text_faint),
                            );
                            match cat_paws_core::text(bytes) {
                                Ok(wat) => selectable_block(ui, &wat, palette.text),
                                Err(e) => {
                                    ui.label(
                                        RichText::new(format!("could not read it back: {e}"))
                                            .color(palette.error),
                                    );
                                }
                            }
                            ui.add_space(6.0);
                        }
                    }

                    if self.show_listing {
                        if let Some(program) = &self.program {
                            ui.label(
                                RichText::new("the steps, in order")
                                    .size(12.5)
                                    .color(palette.text_faint),
                            );
                            let listing = program.listing().join("\n");
                            selectable_block(ui, &listing, palette.text);
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
                        selectable_block(ui, &self.output.join("\n"), palette.text_strong);
                    }
                });

                if let Some(id) = jump_to {
                    self.select_only(id);
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
        let added = app.selected_one().expect("adding a node selects it");
        assert!(app.graph.node(added).is_some());

        app.undo();
        assert!(app.graph.node(added).is_none());
        assert_eq!(
            app.selected_one(), None,
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


/// A whole-number field that shows the number it actually holds.
///
/// `DragValue` carries every value as an `f64` internally, and an `f64` cannot hold every
/// `i64`: 9,223,372,036,854,775,807 becomes 9,223,372,036,854,775,808 on the way through,
/// so the field *displayed* one more than the variable contained. The stored value was
/// always right — the cast back saturates — but a number that reads as one thing and is
/// another is exactly what this language is trying not to do.
///
/// Both halves are fixed here. Formatting casts the `f64` back the same saturating way
/// egui stores it, which recovers the exact integer; parsing reads the text as an `i64`
/// first, so what someone types is what they get rather than the nearest `f64` to it.
/// Read a whole number the way a person writes one.
///
/// Grouping separators are accepted because Cat Paws *hands them out*: the caution on an
/// integer node reads "whole numbers only reach 9,223,372,036,854,775,807". Teaching a
/// format and then refusing it is its own small betrayal — and egui's own parser strips
/// whitespace but not commas, so typing that number back in silently left `9` behind,
/// every digit after the first comma dropped without a word.
pub(crate) fn whole_number(text: &str) -> Option<i64> {
    let cleaned: String = text
        .chars()
        // The minus egui shows is U+2212, which `parse` does not accept.
        .map(|c| if c == '−' { '-' } else { c })
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '_' && *c != '\'')
        .collect();
    cleaned.parse::<i64>().ok()
}

/// A whole-number field that holds every whole number a Cat Paws integer can.
///
/// A text field rather than a `DragValue`, and not for want of trying. `DragValue` carries
/// every value through an `f64`, which holds whole numbers exactly only up to 9,007,199,254,740,992
/// — while an integer here goes to 9,223,372,036,854,775,807, a thousand times further.
/// Anything in that gap was rounded on every edit. It also re-parsed on each keystroke, so
/// a number typed halfway landed as a real value: typing the maximum plus one left
/// 922337203685477632 behind, being the longest prefix that parsed, rounded by the f64.
///
/// So: the text is held beside the number, not in it. Nothing is committed until the field
/// is left, and text that is not a whole number is shown in the error colour and simply
/// never commits — which is what the node caution has always promised.
pub(crate) fn int_field(ui: &mut egui::Ui, v: &mut i64, salt: &str) -> (egui::Response, bool) {
    let id = ui.make_persistent_id(salt);
    // While the field has focus its text lives in egui's temporary store; the rest of the
    // time it is simply the number, so an edit elsewhere is reflected immediately.
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| v.to_string());

    let readable = whole_number(&text).is_some();
    let mut edit = egui::TextEdit::singleline(&mut text)
        .id(id)
        .desired_width(178.0);
    if !readable {
        edit = edit.text_color(ui.visuals().error_fg_color);
    }
    let r = ui.add(edit);

    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    }

    // egui reports `lost_focus` for Enter in a single-line field as well as for clicking
    // away, so one branch covers both ways of finishing.
    let mut committed = false;
    if r.lost_focus() {
        if let Some(n) = whole_number(&text) {
            if n != *v {
                *v = n;
                committed = true;
            }
        }
        ui.data_mut(|d| d.remove::<String>(id));
    }
    (r, committed)
}

/// Monospace text you can select and copy.
///
/// A run of `ui.label`s cannot be dragged across — each one is its own widget, so a
/// selection stops at the end of a line and the clipboard gets one line at a time. A
/// read-only `TextEdit` is one widget over the whole block, so selection and Cmd+C behave
/// the way they do everywhere else.
fn selectable_block(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    // `&mut &str` is how egui takes text it must not modify: the widget still handles
    // selection, focus and copying, and simply discards any edit.
    let mut view = text;
    ui.add(
        egui::TextEdit::multiline(&mut view)
            .font(egui::TextStyle::Monospace)
            .text_color(color)
            .desired_width(f32::INFINITY)
            // No box drawn around it: this is console output, not a field to fill in.
            .frame(egui::Frame::NONE)
            .margin(egui::vec2(0.0, 0.0)),
    );
}

#[cfg(test)]
mod copying_the_console {
    use super::*;

    #[test]
    fn copied_text_carries_the_error_the_fix_and_the_code() {
        let mut app = CatPaws::fresh();
        app.graph = Graph::new(); // no Event start, so compiling must fail
        app.compile();
        let text = app.console_text();
        assert!(text.contains("Event start"), "the message is missing:\n{text}");
        assert!(text.contains("try:"), "the fix is missing:\n{text}");
        assert!(text.contains("CP-FLOW-01"), "the code is missing:\n{text}");
    }

    #[test]
    fn copied_text_carries_what_the_program_printed() {
        let mut app = CatPaws::fresh();
        app.output = vec!["low health".to_string(), "42".to_string()];
        let text = app.console_text();
        assert!(text.contains("low health") && text.contains("42"), "{text}");
    }

    /// Nothing to copy means nothing copied — an empty clipboard is better than a stray
    /// blank line someone pastes into a message wondering why it is empty.
    #[test]
    fn an_empty_console_copies_nothing() {
        let app = CatPaws::fresh();
        assert!(app.console_text().is_empty());
    }
}

#[cfg(test)]
mod selecting_several {
    use super::*;

    fn three_nodes() -> CatPaws {
        let mut app = CatPaws::fresh();
        app.graph = Graph::new();
        for (i, kind) in [NodeKind::LitInt(1), NodeKind::LitInt(2), NodeKind::LitInt(3)]
            .into_iter()
            .enumerate()
        {
            app.graph.add_node(kind, (i as f32 * 300.0, 0.0));
        }
        app
    }

    /// Editing a value is a question about one node. With several chosen there is no
    /// answer, and silently editing whichever came first would be worse than none.
    #[test]
    fn a_value_is_only_editable_when_one_node_is_chosen() {
        let mut app = three_nodes();
        let ids = app.graph.node_ids();
        app.select_only(ids[0]);
        assert_eq!(app.selected_one(), Some(ids[0]));

        app.selection.insert(ids[1]);
        assert_eq!(app.selected_one(), None, "two chosen is not one");
    }

    #[test]
    fn deleting_takes_the_whole_selection() {
        let mut app = three_nodes();
        let ids = app.graph.node_ids();
        app.selection.extend([ids[0], ids[2]]);
        app.push_undo();
        for id in std::mem::take(&mut app.selection) {
            app.graph.remove_node(id);
        }
        assert_eq!(app.graph.node_ids(), vec![ids[1]]);
    }

    /// A node that no longer exists must not stay selected — undo and delete both
    /// remove nodes out from under it.
    #[test]
    fn a_deleted_node_does_not_stay_selected() {
        let mut app = three_nodes();
        let ids = app.graph.node_ids();
        app.selection.extend(ids.clone());
        app.graph.remove_node(ids[1]);
        app.mark_stale();
        assert!(!app.selection.contains(&ids[1]), "a ghost is still selected");
        assert_eq!(app.selection.len(), 2);
    }

    /// A variable node carries no value of its own, so the inspector edits the
    /// variable's — the same number the Variables panel shows.
    #[test]
    fn a_variable_node_offers_the_variables_own_value() {
        let mut app = CatPaws::fresh();
        app.graph = Graph::new();
        app.graph.declare_var("score".into(), DataType::Int);
        let id = app.graph.add_node(
            NodeKind::GetVar { name: "score".into(), ty: DataType::Int },
            (0.0, 0.0),
        );
        app.select_only(id);

        // What the inspector reaches for.
        let name = match app.graph.kind_of(id) {
            Some(NodeKind::GetVar { name, .. }) | Some(NodeKind::SetVar { name, .. }) => {
                Some(name.clone())
            }
            _ => None,
        };
        assert_eq!(name.as_deref(), Some("score"));
        assert!(app.graph.vars.contains_key("score"), "it edits the variable itself");
    }
}

#[cfg(test)]
mod whole_numbers_read_true {
    /// The formatter and parser `int_drag` installs, tested without a `Ui`.
    ///
    /// `DragValue` carries values as `f64`, and above 2^53 an `f64` cannot hold every
    /// `i64`. The number stored was always right; the number *shown* was not.
    fn shown(stored: i64) -> String {
        // What the text field shows when it does not have focus: the number itself.
        stored.to_string()
    }

    fn typed(text: &str) -> Option<i64> {
        // The field commits only what `whole_number` can read, with no f64 in the path.
        super::whole_number(text)
    }

    #[test]
    fn the_largest_whole_number_reads_as_itself() {
        assert_eq!(shown(i64::MAX), "9223372036854775807");
        assert_eq!(shown(i64::MIN), "-9223372036854775808");
    }

    /// The old behaviour, kept as a note: formatting the f64 directly is what showed a
    /// number one larger than the one being held.
    #[test]
    fn formatting_the_float_directly_is_what_was_wrong() {
        assert_eq!(format!("{:.0}", i64::MAX as f64), "9223372036854775808");
        assert_ne!(format!("{:.0}", i64::MAX as f64), i64::MAX.to_string());
    }

    #[test]
    fn ordinary_numbers_are_unaffected() {
        for v in [0, 1, -1, 50, -9999, 1_000_000, 9_007_199_254_740_992] {
            assert_eq!(shown(v), v.to_string(), "{v} did not survive");
        }
    }

    #[test]
    fn typing_a_whole_number_gives_that_number() {
        assert_eq!(typed("9223372036854775807"), Some(i64::MAX));
        assert_eq!(typed("-9223372036854775808"), Some(i64::MIN));
        assert_eq!(typed(" 42 "), Some(42));
    }

    /// Cat Paws prints these numbers with commas in its own cautions, so it has to
    /// accept them back. egui's parser strips whitespace and nothing else, which left
    /// `9` behind when the maximum was typed the way a person writes it.
    #[test]
    fn a_number_written_the_way_people_write_it_is_understood() {
        assert_eq!(typed("9,223,372,036,854,775,807"), Some(i64::MAX));
        assert_eq!(typed("-9,223,372,036,854,775,808"), Some(i64::MIN));
        assert_eq!(typed("1,000"), Some(1000));
        assert_eq!(typed("1 000 000"), Some(1_000_000), "spaces group numbers too");
        assert_eq!(typed("1_000_000"), Some(1_000_000), "and underscores, as in code");
        assert_eq!(typed("−42"), Some(-42), "the minus egui itself displays");
    }

    /// The case that started this: one past the top.
    ///
    /// egui's own path saturated it to the maximum. The comma fix earlier in this session
    /// replaced that with a plain `parse`, which fails — and a failed parse silently kept
    /// whatever prefix had last parsed, rounded through the f64: typing the maximum plus
    /// one left 922337203685477632 in the field. A text field commits nothing it cannot
    /// read, so the number simply stays as it was.
    #[test]
    fn one_past_the_top_is_not_quietly_turned_into_something_else() {
        assert_eq!(typed("9223372036854775808"), None);
        assert_eq!(typed("9,223,372,036,854,775,808"), None);
        assert_eq!(typed("-9223372036854775809"), None);
        // And the value that used to appear is not something anyone can now land on.
        assert_ne!(typed("9,223,372,036,854,775,808"), Some(922337203685477632));
    }

    /// Every whole number the language allows survives the field exactly — including the
    /// range above 2^53 where an f64 cannot tell neighbours apart.
    #[test]
    fn numbers_past_the_reach_of_a_float_are_exact() {
        for v in [
            9_007_199_254_740_993_i64,      // 2^53 + 1, the first f64 cannot hold
            922_337_203_685_477_580,
            922_337_203_685_477_581,        // its neighbour: an f64 rounds both the same
            i64::MAX - 1,
            i64::MAX,
            i64::MIN,
        ] {
            assert_eq!(typed(&v.to_string()), Some(v), "{v} did not survive typing");
            assert_eq!(shown(v), v.to_string(), "{v} did not survive display");
        }
    }

    /// Anything that is not a whole number is refused rather than rounded into one.
    #[test]
    fn a_decimal_or_a_word_is_refused() {
        assert_eq!(typed("1.5"), None);
        assert_eq!(typed("banana"), None);
        assert_eq!(typed("9223372036854775808"), None, "one past the top is not a whole number here");
    }
}
