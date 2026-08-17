//! The node canvas: pan, zoom, node dragging and wire dragging.
//!
//! All of it is painted by hand — there is no node-graph library underneath.
//! The layout is derived from each node's pin list, so adding a node kind to the
//! core crate makes it draggable and wireable here with no extra work.

use crate::app::CatPaws;
use crate::theme::Palette;
use cat_paws_core::{Graph, NodeId, NodeKind, PinKind, PinRef, Side};
use egui::epaint::CubicBezierShape;
use egui::text::LayoutJob;
use egui::{
    pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, TextFormat, Ui,
    Vec2,
};

pub const NODE_WIDTH: f32 = 196.0;
pub const HEADER_H: f32 = 42.0;
pub const ROW_H: f32 = 24.0;
pub const BODY_PAD: f32 = 10.0;
pub const PIN_R: f32 = 5.0;
pub const GRID: f32 = 32.0;

/// Text sizes at 100% zoom.
pub const TITLE_PX: f32 = 15.5;
pub const SUBTITLE_PX: f32 = 12.0;
pub const PIN_LABEL_PX: f32 = 13.0;

// Floors that text stops shrinking at.
//
// Canvas text scales with the zoom, but only down to these sizes. Past that,
// shrinking further just produces a smear -- and zoomed out is precisely when
// you need to tell one node from another, so the title in particular must stay
// legible however far out you go.
const MIN_TITLE_PX: f32 = 10.5;
const MIN_SUBTITLE_PX: f32 = 9.0;
const MIN_PIN_LABEL_PX: f32 = 9.5;

pub fn title_font_px(zoom: f32) -> f32 {
    (TITLE_PX * zoom).max(MIN_TITLE_PX)
}

pub fn subtitle_font_px(zoom: f32) -> f32 {
    (SUBTITLE_PX * zoom).max(MIN_SUBTITLE_PX)
}

pub fn pin_label_font_px(zoom: f32) -> f32 {
    (PIN_LABEL_PX * zoom).max(MIN_PIN_LABEL_PX)
}

/// Extra space between subtitle letters, as a fraction of the font size.
/// Tune here — this is the whole knob.
pub const SUBTITLE_TRACKING: f32 = 0.10;

/// Subtitle letter spacing in pixels.
///
/// Derived from the *rendered* font size rather than raw zoom, so it follows
/// the size floor: when the font stops shrinking, the tracking stops with it
/// instead of collapsing to nothing when zoomed out.
pub fn subtitle_tracking_px(zoom: f32) -> f32 {
    subtitle_font_px(zoom) * SUBTITLE_TRACKING
}

/// Whether the header still has room for the subtitle beneath a floored title.
///
/// The title never disappears; the subtitle is the first thing dropped, because
/// it only ever restates the node's category.
pub fn subtitle_fits(zoom: f32) -> bool {
    HEADER_H * zoom >= title_font_px(zoom) + subtitle_font_px(zoom) + 4.0
}

/// Whether pin rows are far enough apart, and nodes wide enough, to label pins
/// without the labels colliding with each other or with the opposite column.
pub fn pin_labels_fit(zoom: f32) -> bool {
    ROW_H * zoom >= pin_label_font_px(zoom) + 2.0 && NODE_WIDTH * zoom >= 108.0
}

/// How the canvas is currently positioned over the graph.
#[derive(Clone, Copy)]
pub struct View {
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        View {
            pan: vec2(60.0, 40.0),
            zoom: 1.0,
        }
    }
}

impl View {
    pub fn to_screen(&self, rect: Rect, world: Pos2) -> Pos2 {
        rect.min + self.pan + world.to_vec2() * self.zoom
    }

    pub fn to_world(&self, rect: Rect, screen: Pos2) -> Pos2 {
        ((screen - rect.min - self.pan) / self.zoom).to_pos2()
    }
}

/// What the pointer is in the middle of doing.
#[derive(Clone, Copy, PartialEq)]
pub enum Interaction {
    Idle,
    Panning,
    /// Moving a node. `grab` is the world-space offset from the node's origin to
    /// the pointer, so the node doesn't jump when the drag starts.
    DragNode {
        id: NodeId,
        grab: Vec2,
    },
    /// Pulling a wire out of a pin. `origin` may be an output (dragging
    /// forwards) or an input (dragging backwards) — the drop decides direction.
    DragWire {
        origin: PinRef,
    },
}

pub fn node_rows(kind: &NodeKind) -> usize {
    kind.inputs().len().max(kind.outputs().len())
}

pub fn node_size(kind: &NodeKind) -> Vec2 {
    vec2(
        NODE_WIDTH,
        HEADER_H + BODY_PAD * 2.0 + node_rows(kind) as f32 * ROW_H,
    )
}

/// Pin position relative to the node's top-left corner, in world units.
///
/// The node kind is unused while every node is the same width, but it is part of
/// the signature so variable-width nodes won't ripple through the call sites.
pub fn pin_offset(_kind: &NodeKind, side: Side, index: usize) -> Vec2 {
    let x = match side {
        Side::In => 0.0,
        Side::Out => NODE_WIDTH,
    };
    vec2(
        x,
        HEADER_H + BODY_PAD + ROW_H * index as f32 + ROW_H * 0.5,
    )
}

fn node_rect(view: &View, rect: Rect, pos: (f32, f32), kind: &NodeKind) -> Rect {
    let top_left = view.to_screen(rect, pos2(pos.0, pos.1));
    Rect::from_min_size(top_left, node_size(kind) * view.zoom)
}

fn pin_screen_pos(view: &View, rect: Rect, graph: &Graph, r: PinRef) -> Option<Pos2> {
    let node = graph.node(r.node)?;
    let offset = pin_offset(&node.kind, r.side, r.index);
    Some(view.to_screen(rect, pos2(node.pos.0 + offset.x, node.pos.1 + offset.y)))
}

/// Draws a wire as a horizontal-tangent cubic bezier, the way Blueprints do.
fn draw_wire(painter: &Painter, a: Pos2, b: Pos2, color: Color32, width: f32) {
    let dx = ((b.x - a.x).abs() * 0.5).clamp(28.0, 160.0);
    let points = [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b];
    painter.add(CubicBezierShape::from_points_stroke(
        points,
        false,
        Color32::TRANSPARENT,
        Stroke::new(width, color),
    ));
}

/// Draws text with extra stroke weight and letter spacing.
///
/// egui's bundled fonts ship no bold face, so weight is faked by painting the
/// same string a fraction of a pixel apart. At these sizes the offsets land
/// inside the antialiasing and read as a heavier stroke rather than a blur.
/// That added weight closes up the gaps between letters, which is why this also
/// takes `tracking` to open them back out.
///
/// The string is laid out once and painted several times, so the extra passes
/// cost drawing but not text shaping.
fn text_semibold(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    tracking: f32,
) {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font,
            color,
            extra_letter_spacing: tracking,
            ..Default::default()
        },
    );
    let galley = painter.layout_job(job);
    let origin = anchor.anchor_size(pos, galley.size()).min;
    for offset in [vec2(0.0, 0.0), vec2(0.5, 0.0), vec2(0.0, 0.35)] {
        painter.galley(origin + offset, galley.clone(), color);
    }
}

fn pin_color(palette: &Palette, kind: PinKind) -> Color32 {
    match kind {
        PinKind::Exec => palette.exec_wire,
        PinKind::Data(ty) => palette.data_color(ty),
    }
}

impl CatPaws {
    pub(crate) fn ui_canvas(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let palette = self.palette();

        painter.rect_filled(rect, 0.0, palette.canvas);
        self.handle_zoom(ui, rect);
        self.draw_grid(&painter, rect, &palette);

        // The hovered pin drives highlighting and the drag preview only. Anything
        // that acts on a press hit-tests the press position instead — see
        // `handle_interaction`.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let hovered_pin = pointer.and_then(|p| self.pin_at(rect, p));

        self.handle_interaction(ui, &response, rect);
        self.draw_wires(&painter, rect, &palette);
        self.draw_pending_wire(&painter, rect, &palette, pointer, hovered_pin);
        self.draw_nodes(&painter, rect, &palette, hovered_pin);

        if hovered_pin.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if matches!(self.interaction, Interaction::Panning) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        self.draw_hint(&painter, rect, &palette);
    }

    fn handle_zoom(&mut self, ui: &Ui, rect: Rect) {
        if !ui.rect_contains_pointer(rect) {
            return;
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() < 0.01 {
            return;
        }
        let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        // Keep whatever is under the cursor pinned there while zooming.
        let world_before = self.view.to_world(rect, cursor);
        self.view.zoom = (self.view.zoom * (1.0 + scroll * 0.0015)).clamp(0.35, 2.5);
        self.view.pan = cursor - rect.min - world_before.to_vec2() * self.view.zoom;
    }

    fn draw_grid(&self, painter: &Painter, rect: Rect, palette: &Palette) {
        let step = GRID * self.view.zoom;
        if step < 6.0 {
            return;
        }
        let start_x = rect.min.x + self.view.pan.x.rem_euclid(step);
        let start_y = rect.min.y + self.view.pan.y.rem_euclid(step);

        let mut x = start_x;
        let mut i = 0;
        while x < rect.max.x {
            let strong = i % 4 == 0;
            painter.line_segment(
                [pos2(x, rect.min.y), pos2(x, rect.max.y)],
                Stroke::new(1.0, if strong { palette.grid_strong } else { palette.grid }),
            );
            x += step;
            i += 1;
        }
        let mut y = start_y;
        let mut j = 0;
        while y < rect.max.y {
            let strong = j % 4 == 0;
            painter.line_segment(
                [pos2(rect.min.x, y), pos2(rect.max.x, y)],
                Stroke::new(1.0, if strong { palette.grid_strong } else { palette.grid }),
            );
            y += step;
            j += 1;
        }
    }

    /// The topmost pin under a screen position, if any.
    fn pin_at(&self, rect: Rect, screen: Pos2) -> Option<PinRef> {
        let radius = (PIN_R * self.view.zoom + 5.0).max(7.0);
        let mut found = None;
        for node in self.graph.nodes() {
            for (side, count) in [
                (Side::In, node.kind.inputs().len()),
                (Side::Out, node.kind.outputs().len()),
            ] {
                for index in 0..count {
                    let r = PinRef {
                        node: node.id,
                        side,
                        index,
                    };
                    if let Some(p) = pin_screen_pos(&self.view, rect, &self.graph, r) {
                        if p.distance(screen) <= radius {
                            found = Some(r);
                        }
                    }
                }
            }
        }
        found
    }

    /// The topmost node under a screen position. Later nodes win, matching paint order.
    fn node_at(&self, rect: Rect, screen: Pos2) -> Option<NodeId> {
        let mut found = None;
        for node in self.graph.nodes() {
            if node_rect(&self.view, rect, node.pos, &node.kind).contains(screen) {
                found = Some(node.id);
            }
        }
        found
    }

    /// Hit-testing never uses the *hovered* position: what you grabbed has to be
    /// decided where the button went down.
    ///
    /// For the start of a drag that means [`egui::PointerState::press_origin`],
    /// not `interact_pointer_pos`. `interact_pointer_pos` is the pointer's
    /// current position, and egui does not report `drag_started` until the
    /// pointer has travelled past `max_click_dist` — so by the time a drag is
    /// recognised, the current position has already drifted several pixels away
    /// from the press. That drift still lands inside a node body, but it is
    /// enough to miss a pin, whose hit radius is only about ten pixels. Wire
    /// drags silently became canvas pans.
    fn handle_interaction(&mut self, ui: &Ui, response: &egui::Response, rect: Rect) {
        let alt = ui.input(|i| i.modifiers.alt);
        let pointer = response.interact_pointer_pos();
        let press = ui.input(|i| i.pointer.press_origin()).or(pointer);

        // The right button pans, and it pans from anywhere -- including from on
        // top of a node -- so the view can always be moved without hunting for a
        // patch of empty canvas. The left button is reserved for the graph
        // itself: moving nodes and pulling wires.
        let (pan_pressed, pan_held, pointer_delta) = ui.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_down(egui::PointerButton::Secondary),
                i.pointer.delta(),
            )
        });
        if pan_pressed && ui.rect_contains_pointer(rect) {
            self.interaction = Interaction::Panning;
        }
        if matches!(self.interaction, Interaction::Panning) {
            if pan_held {
                self.view.pan += pointer_delta;
                return;
            }
            self.interaction = Interaction::Idle;
        }

        if response.clicked() {
            if let Some(p) = pointer {
                // Alt-click a pin to cut every wire attached to it.
                if alt {
                    if let Some(pin) = self.pin_at(rect, p) {
                        self.push_undo();
                        self.graph.disconnect_pin(pin);
                        self.mark_stale();
                        return;
                    }
                }
                self.selected = self.node_at(rect, p);
            }
        }

        if response.drag_started() {
            self.interaction = match press {
                Some(p) => {
                    if let Some(pin) = self.pin_at(rect, p) {
                        Interaction::DragWire { origin: pin }
                    } else if let Some(id) = self.node_at(rect, p) {
                        self.selected = Some(id);
                        // Snapshot once, at the start of the move -- not on every
                        // frame the node is dragged across.
                        self.push_undo();
                        let node_pos = self.graph.node(id).map(|n| n.pos).unwrap_or((0.0, 0.0));
                        let world = self.view.to_world(rect, p);
                        Interaction::DragNode {
                            id,
                            grab: world - pos2(node_pos.0, node_pos.1),
                        }
                    } else {
                        // Left-dragging empty canvas does nothing; panning is
                        // the right button's job.
                        Interaction::Idle
                    }
                }
                None => Interaction::Idle,
            };
        }

        if response.dragged() {
            match self.interaction {
                Interaction::DragNode { id, grab } => {
                    if let Some(p) = response.interact_pointer_pos() {
                        let world = self.view.to_world(rect, p) - grab;
                        if let Some(node) = self.graph.node_mut(id) {
                            node.pos = (world.x, world.y);
                        }
                    }
                }
                _ => {}
            }
        }

        if response.drag_stopped() {
            if let Interaction::DragWire { origin } = self.interaction {
                // The drop target is decided at the release point.
                if let Some(target) = pointer.and_then(|p| self.pin_at(rect, p)) {
                    // Orient the pair so `from` is the output and `to` the input,
                    // whichever end the drag began at.
                    let (from, to) = match (origin.side, target.side) {
                        (Side::Out, Side::In) => (origin, target),
                        (Side::In, Side::Out) => (target, origin),
                        _ => {
                            self.set_wire_error("wires run from an output pin to an input pin");
                            self.interaction = Interaction::Idle;
                            return;
                        }
                    };
                    // Only record an undo step if the wire is actually accepted;
                    // a refused connection changed nothing to undo.
                    let before = self.graph.clone();
                    match self.graph.connect(from, to) {
                        Ok(()) => {
                            self.remember(before);
                            self.mark_stale();
                        }
                        Err(_) => {
                            let message = self.describe_refusal(from, to);
                            self.set_wire_error(&message);
                        }
                    }
                }
            }
            self.interaction = Interaction::Idle;
        }

        // Backspace must not delete the selected node while the user is typing
        // in the variable-name field. Ask about text edits specifically: the
        // canvas is itself focusable, so a broader check would disable Delete
        // as soon as the canvas was clicked -- which is always.
        let typing = ui.ctx().text_edit_focused();
        if !typing
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            if let Some(id) = self.selected.take() {
                self.push_undo();
                self.graph.remove_node(id);
                self.mark_stale();
            }
        }
    }

    /// A friendlier message than the core's, using the two pin types by name.
    fn describe_refusal(&self, from: PinRef, to: PinRef) -> String {
        match (self.graph.pin_kind(from), self.graph.pin_kind(to)) {
            (Some(PinKind::Data(a)), Some(PinKind::Data(b))) if a != b => {
                format!(
                    "can't wire {} into {} — the types don't match",
                    a.label(),
                    b.label()
                )
            }
            (Some(PinKind::Exec), Some(PinKind::Data(_)))
            | (Some(PinKind::Data(_)), Some(PinKind::Exec)) => {
                "can't wire an execution pin into a data pin".to_string()
            }
            _ => "that connection isn't allowed".to_string(),
        }
    }

    fn draw_wires(&self, painter: &Painter, rect: Rect, palette: &Palette) {
        for link in self.graph.links() {
            let (Some(a), Some(b)) = (
                pin_screen_pos(&self.view, rect, &self.graph, link.from),
                pin_screen_pos(&self.view, rect, &self.graph, link.to),
            ) else {
                continue;
            };
            let kind = self
                .graph
                .pin_kind(link.from)
                .unwrap_or(PinKind::Exec);
            let width = match kind {
                PinKind::Exec => 3.0,
                PinKind::Data(_) => 2.0,
            };
            draw_wire(painter, a, b, pin_color(palette, kind), width * self.view.zoom);
        }
    }

    fn draw_pending_wire(
        &self,
        painter: &Painter,
        rect: Rect,
        palette: &Palette,
        pointer: Option<Pos2>,
        hovered_pin: Option<PinRef>,
    ) {
        let Interaction::DragWire { origin } = self.interaction else {
            return;
        };
        let (Some(start), Some(cursor)) = (
            pin_screen_pos(&self.view, rect, &self.graph, origin),
            pointer,
        ) else {
            return;
        };
        let kind = self.graph.pin_kind(origin).unwrap_or(PinKind::Exec);

        // Snap to a legal target, and go red over an illegal one, so the rule is
        // visible before the mouse is released.
        let (end, color) = match hovered_pin {
            Some(target) if target != origin => {
                let ordered = match (origin.side, target.side) {
                    (Side::Out, Side::In) => Some((origin, target)),
                    (Side::In, Side::Out) => Some((target, origin)),
                    _ => None,
                };
                let legal = ordered
                    .map(|(f, t)| self.graph.can_connect(f, t).is_ok())
                    .unwrap_or(false);
                let p = pin_screen_pos(&self.view, rect, &self.graph, target).unwrap_or(cursor);
                (
                    p,
                    if legal {
                        pin_color(palette, kind)
                    } else {
                        palette.error
                    },
                )
            }
            _ => (cursor, pin_color(palette, kind).gamma_multiply(0.7)),
        };

        let (a, b) = if origin.side == Side::Out {
            (start, end)
        } else {
            (end, start)
        };
        draw_wire(painter, a, b, color, 2.5 * self.view.zoom);
    }

    fn draw_nodes(
        &self,
        painter: &Painter,
        rect: Rect,
        palette: &Palette,
        hovered_pin: Option<PinRef>,
    ) {
        let zoom = self.view.zoom;
        let failing = self.failing_nodes();

        for node in self.graph.nodes() {
            let r = node_rect(&self.view, rect, node.pos, &node.kind);
            if !r.intersects(rect) {
                continue;
            }
            let header_color = palette.category_color(node.kind.category());
            let selected = self.selected == Some(node.id);
            let outline = if failing.contains(&node.id) {
                Stroke::new(2.0, palette.error)
            } else if selected {
                Stroke::new(2.0, palette.selection)
            } else {
                Stroke::new(1.0, palette.node_outline)
            };

            // Body, then header painted over its top half.
            painter.rect_filled(r, 6.0, palette.node_body);
            let header = Rect::from_min_size(r.min, vec2(r.width(), HEADER_H * zoom));
            painter.rect_filled(header, 6.0, header_color);
            // Square off the bottom of the header so it meets the body cleanly.
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(header.min.x, header.max.y - 7.0 * zoom),
                    vec2(header.width(), 7.0 * zoom),
                ),
                0.0,
                header_color,
            );
            painter.rect_stroke(r, 6.0, outline, egui::StrokeKind::Inside);

            // Header text is clipped to the header so a floored font can never
            // spill out of a small node.
            let text = painter.with_clip_rect(header.intersect(rect));
            let title_px = title_font_px(zoom);
            let pad_x = (10.0 * zoom).max(5.0);

            if subtitle_fits(zoom) {
                let subtitle_px = subtitle_font_px(zoom);
                let block = title_px + subtitle_px + 4.0;
                let top = header.center().y - block / 2.0;
                text.text(
                    pos2(header.min.x + pad_x, top + title_px / 2.0),
                    Align2::LEFT_CENTER,
                    node.kind.title(),
                    FontId::proportional(title_px),
                    palette.on_category(),
                );
                text_semibold(
                    &text,
                    pos2(header.min.x + pad_x, top + title_px + 4.0 + subtitle_px / 2.0),
                    Align2::LEFT_CENTER,
                    &node.kind.subtitle(),
                    FontId::proportional(subtitle_px),
                    // Barely dimmed now that it carries weight -- a heavier
                    // stroke plus a strong fade just looks muddy.
                    palette.on_category().gamma_multiply(0.94),
                    subtitle_tracking_px(zoom),
                );
            } else {
                // No room for both, so the title takes the whole header.
                text.text(
                    pos2(header.min.x + pad_x, header.center().y),
                    Align2::LEFT_CENTER,
                    node.kind.title(),
                    FontId::proportional(title_px),
                    palette.on_category(),
                );
            }

            self.draw_pins(painter, rect, palette, node.id, hovered_pin);
        }
    }

    fn draw_pins(
        &self,
        painter: &Painter,
        rect: Rect,
        palette: &Palette,
        id: NodeId,
        hovered_pin: Option<PinRef>,
    ) {
        let Some(node) = self.graph.node(id) else {
            return;
        };
        let zoom = self.view.zoom;
        let inputs = node.kind.inputs();
        let outputs = node.kind.outputs();

        for (side, pins) in [(Side::In, &inputs), (Side::Out, &outputs)] {
            for (index, pin) in pins.iter().enumerate() {
                let r = PinRef { node: id, side, index };
                let Some(center) = pin_screen_pos(&self.view, rect, &self.graph, r) else {
                    continue;
                };
                let color = pin_color(palette, pin.kind);
                let connected = match side {
                    Side::In => self.graph.source_of(r).is_some(),
                    Side::Out => self.graph.target_of(r).is_some(),
                };
                let hovered = hovered_pin == Some(r);
                let radius = PIN_R * zoom * if hovered { 1.35 } else { 1.0 };

                match pin.kind {
                    // Execution pins are arrows, data pins are dots — so the two
                    // kinds of wire are distinguishable without relying on colour.
                    PinKind::Exec => {
                        let h = radius * 1.25;
                        let tri = vec![
                            center + vec2(radius, 0.0),
                            center + vec2(-radius * 0.7, h),
                            center + vec2(-radius * 0.7, -h),
                        ];
                        if connected {
                            painter.add(Shape::convex_polygon(tri, color, Stroke::NONE));
                        } else {
                            painter.add(Shape::convex_polygon(
                                tri,
                                Color32::TRANSPARENT,
                                Stroke::new(1.5 * zoom, color),
                            ));
                        }
                    }
                    PinKind::Data(_) => {
                        if connected {
                            painter.circle_filled(center, radius, color);
                        } else {
                            painter.circle_stroke(center, radius, Stroke::new(1.5 * zoom, color));
                        }
                    }
                }

                // Pin labels are the one thing that genuinely cannot survive a
                // deep zoom-out: the rows themselves become thinner than the
                // floored text, so labels would overlap each other.
                if pin_labels_fit(zoom) {
                    let gap = (11.0 * zoom).max(7.0);
                    let (anchor, x) = match side {
                        Side::In => (Align2::LEFT_CENTER, center.x + gap),
                        Side::Out => (Align2::RIGHT_CENTER, center.x - gap),
                    };
                    painter.text(
                        pos2(x, center.y),
                        anchor,
                        &pin.name,
                        FontId::proportional(pin_label_font_px(zoom)),
                        palette.text_strong,
                    );
                }
            }
        }
    }

    /// A short reminder of the controls, bottom-left of the canvas.
    fn draw_hint(&self, painter: &Painter, rect: Rect, palette: &Palette) {
        painter.text(
            pos2(rect.min.x + 12.0, rect.max.y - 12.0),
            Align2::LEFT_BOTTOM,
            "left-drag a node to move it  ·  right-drag to pan  ·  scroll to zoom  ·  drag a pin to wire  ·  alt-click a pin to cut  ·  delete removes a node",
            FontId::proportional(12.5),
            palette.text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_paws_core::NodeKind;

    fn canvas_rect() -> Rect {
        Rect::from_min_size(pos2(220.0, 60.0), vec2(900.0, 700.0))
    }

    /// Screen and world coordinates must be exact inverses, or hit-testing
    /// drifts away from what is drawn as soon as the view is panned or zoomed.
    #[test]
    fn screen_and_world_round_trip() {
        let rect = canvas_rect();
        for (pan, zoom) in [
            (vec2(0.0, 0.0), 1.0_f32),
            (vec2(60.0, 40.0), 1.0),
            (vec2(-320.0, 175.5), 0.35),
            (vec2(880.0, -640.0), 2.5),
        ] {
            let view = View { pan, zoom };
            for world in [pos2(0.0, 0.0), pos2(330.0, 140.0), pos2(-90.5, 612.25)] {
                let back = view.to_world(rect, view.to_screen(rect, world));
                assert!(
                    (back.x - world.x).abs() < 0.01 && (back.y - world.y).abs() < 0.01,
                    "round trip failed at pan {pan:?} zoom {zoom}: {world:?} -> {back:?}"
                );
            }
        }
    }

    /// Input pins sit on the left edge, output pins on the right, and every pin
    /// stays inside the node's own box.
    #[test]
    fn pins_sit_on_the_node_edges() {
        for kind in [
            NodeKind::EventStart,
            NodeKind::Branch,
            NodeKind::Print,
            NodeKind::LessThan,
            NodeKind::LitInt(7),
        ] {
            let size = node_size(&kind);
            for (side, pins) in [
                (Side::In, kind.inputs()),
                (Side::Out, kind.outputs()),
            ] {
                for index in 0..pins.len() {
                    let offset = pin_offset(&kind, side, index);
                    let expected_x = match side {
                        Side::In => 0.0,
                        Side::Out => NODE_WIDTH,
                    };
                    assert_eq!(offset.x, expected_x, "{kind:?} {side:?} pin {index}");
                    assert!(
                        offset.y > HEADER_H && offset.y < size.y,
                        "{kind:?} {side:?} pin {index} at y={} escapes the node (height {})",
                        offset.y,
                        size.y
                    );
                }
            }
        }
    }

    /// A node has to be tall enough for whichever side has more pins.
    #[test]
    fn node_height_follows_the_busier_side() {
        // Branch: 2 in (exec, condition) and 2 out (true, false).
        assert_eq!(node_rows(&NodeKind::Branch), 2);
        // Event start: nothing in, one exec out.
        assert_eq!(node_rows(&NodeKind::EventStart), 1);
        // Less than: two data inputs, one result out -- the inputs win.
        assert_eq!(node_rows(&NodeKind::LessThan), 2);

        assert!(node_size(&NodeKind::Branch).y > node_size(&NodeKind::EventStart).y);
    }

    /// Two pins on the same node must never land on the same point, or a drag
    /// could not tell them apart.
    #[test]
    fn pins_on_a_node_are_distinct() {
        let kind = NodeKind::Branch;
        let mut seen: Vec<Vec2> = Vec::new();
        for (side, pins) in [(Side::In, kind.inputs()), (Side::Out, kind.outputs())] {
            for index in 0..pins.len() {
                let offset = pin_offset(&kind, side, index);
                assert!(
                    !seen.iter().any(|s| (*s - offset).length() < 1.0),
                    "duplicate pin position {offset:?}"
                );
                seen.push(offset);
            }
        }
        assert_eq!(seen.len(), 4);
    }

    /// The whole point of the floors: however far you zoom out, a node's title
    /// is still rendered at a readable size. This regressed once by hiding text
    /// below a zoom threshold, which made a zoomed-out graph unidentifiable.
    #[test]
    fn titles_stay_readable_at_every_zoom() {
        // 0.35 is the minimum the view clamps to.
        for zoom in [0.35_f32, 0.4, 0.5, 0.75, 1.0, 1.6, 2.5] {
            let px = title_font_px(zoom);
            assert!(
                px >= MIN_TITLE_PX,
                "title font {px} at zoom {zoom} fell under the floor"
            );
        }
    }

    /// Zoomed in, text should track the canvas rather than sit at the floor.
    #[test]
    fn text_scales_up_with_zoom() {
        assert!(title_font_px(2.0) > title_font_px(1.0));
        assert!(pin_label_font_px(2.0) > pin_label_font_px(1.0));
        assert_eq!(title_font_px(1.0), TITLE_PX);
        assert_eq!(pin_label_font_px(1.0), PIN_LABEL_PX);
    }

    /// The title must always fit the header, including when floored.
    #[test]
    fn header_always_has_room_for_the_title() {
        for zoom in [0.35_f32, 0.4, 0.5, 0.75, 1.0, 2.5] {
            let header = HEADER_H * zoom;
            assert!(
                header >= title_font_px(zoom),
                "header {header} too short for a {} title at zoom {zoom}",
                title_font_px(zoom)
            );
        }
    }

    /// The subtitle is dropped only when it genuinely will not fit, and is
    /// present at normal zoom.
    #[test]
    fn subtitle_is_dropped_only_when_it_cannot_fit() {
        assert!(subtitle_fits(1.0), "subtitle should show at 100%");
        assert!(!subtitle_fits(0.35), "subtitle cannot fit when fully zoomed out");

        for zoom in [0.35_f32, 0.5, 0.75, 1.0, 2.5] {
            if subtitle_fits(zoom) {
                assert!(
                    HEADER_H * zoom >= title_font_px(zoom) + subtitle_font_px(zoom),
                    "claimed the subtitle fits at zoom {zoom} when it does not"
                );
            }
        }
    }

    /// Pin labels are hidden exactly when the rows are too thin for them, so
    /// they can never overlap each other.
    #[test]
    fn pin_labels_never_overlap_their_rows() {
        for zoom in [0.35_f32, 0.4, 0.5, 0.6, 0.75, 1.0, 2.5] {
            if pin_labels_fit(zoom) {
                assert!(
                    ROW_H * zoom >= pin_label_font_px(zoom),
                    "labels shown at zoom {zoom} but the row is thinner than the text"
                );
            }
        }
        assert!(pin_labels_fit(1.0), "pin labels should show at 100%");
    }

    /// Tracking follows the rendered font size, so when the font hits its floor
    /// the letter spacing holds too instead of collapsing when zoomed out.
    #[test]
    fn subtitle_tracking_survives_zooming_out() {
        for zoom in [0.35_f32, 0.4, 0.5, 0.75, 1.0, 2.5] {
            let tracking = subtitle_tracking_px(zoom);
            assert!(
                tracking > 0.5,
                "tracking {tracking} at zoom {zoom} is too small to see"
            );
            assert_eq!(tracking, subtitle_font_px(zoom) * SUBTITLE_TRACKING);
        }
        // Zoomed right out the font is floored, so the tracking is floored too.
        assert_eq!(subtitle_tracking_px(0.35), subtitle_tracking_px(0.1));
        // Zoomed in it grows with the text.
        assert!(subtitle_tracking_px(2.5) > subtitle_tracking_px(1.0));
    }
}
