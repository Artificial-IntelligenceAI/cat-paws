//! Hand-drawn toolbar icons.
//!
//! egui's bundled fonts don't reliably carry a hammer glyph across platforms, so
//! the two toolbar icons are painted from primitives instead. That also keeps
//! them crisp at any size and correctly coloured in both themes.

use egui::{pos2, vec2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, Ui};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Compile.
    Hammer,
    /// Compile and run.
    Play,
    /// Undo the last change.
    Undo,
}

/// Draws the icon centred in `rect`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.height().min(rect.width());

    match icon {
        Icon::Hammer => {
            // Handle: a stroke running from lower-left to upper-right.
            let handle_start = c + vec2(-s * 0.30, s * 0.36);
            let handle_end = c + vec2(s * 0.10, s * -0.04);
            painter.line_segment(
                [handle_start, handle_end],
                Stroke::new((s * 0.13).max(1.5), color),
            );

            // Head: a stubby quad sitting at the top of the handle, tilted to
            // match it, so the whole thing reads as a hammer rather than a "T".
            let dir = vec2(0.7071, -0.7071); // 45 degrees up-right
            let perp = vec2(dir.y, -dir.x);
            let head_center = c + vec2(s * 0.16, s * -0.12);
            let half_len = s * 0.30;
            let half_thick = s * 0.15;
            let quad = vec![
                head_center + perp * half_len + dir * half_thick,
                head_center - perp * half_len + dir * half_thick,
                head_center - perp * half_len - dir * half_thick,
                head_center + perp * half_len - dir * half_thick,
            ];
            painter.add(egui::Shape::convex_polygon(
                quad,
                color,
                Stroke::NONE,
            ));
        }
        Icon::Undo => {
            // An arc over the top, with an arrowhead dropping off the left end.
            let r = s * 0.30;
            let steps = 24;
            let a0 = 15.0_f32.to_radians();
            let a1 = 168.0_f32.to_radians();
            let arc: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let a = a0 + (a1 - a0) * (i as f32 / steps as f32);
                    // Screen y grows downward, so the sine is negated.
                    c + vec2(a.cos() * r, -a.sin() * r * 0.92)
                })
                .collect();
            let tip = *arc.last().expect("arc is never empty");
            painter.add(egui::Shape::line(
                arc,
                Stroke::new((s * 0.11).max(1.4), color),
            ));

            let w = s * 0.16;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip + vec2(0.0, w * 1.25),
                    tip + vec2(-w, -w * 0.3),
                    tip + vec2(w, -w * 0.3),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Play => {
            let r = s * 0.34;
            let triangle = vec![
                c + vec2(r, 0.0),
                c + vec2(-r * 0.72, r * 0.92),
                c + vec2(-r * 0.72, -r * 0.92),
            ];
            painter.add(egui::Shape::convex_polygon(
                triangle,
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// A toolbar button: hand-painted icon plus a label.
pub fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    fill: Color32,
    fg: Color32,
) -> Response {
    let font = FontId::proportional(14.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, fg);

    let icon_size = 16.0;
    let pad = 9.0;
    let gap = 7.0;
    let size = vec2(
        pad + icon_size + gap + galley.size().x + pad,
        26.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bg = if response.is_pointer_button_down_on() {
            fill.gamma_multiply(0.75)
        } else if response.hovered() {
            fill.gamma_multiply(1.15)
        } else {
            fill
        };
        painter.rect_filled(rect, 5.0, bg);

        let icon_rect = Rect::from_min_size(
            pos2(rect.min.x + pad, rect.center().y - icon_size / 2.0),
            vec2(icon_size, icon_size),
        );
        paint_icon(painter, icon_rect, icon, fg);

        let text_pos: Pos2 = pos2(
            icon_rect.max.x + gap,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(text_pos, galley, fg);
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}
