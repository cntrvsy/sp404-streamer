//! Studio-grade 270° Rotary Encoder Knob widget (`0..=127` MIDI CC).
//!
//! Supports vertical mouse drag, `Shift` fine adjustment, and double-click to reset.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Stroke, StrokeKind, Ui, Vec2};

use crate::ui::theme::RolandTheme;

pub struct RotaryKnob<'a> {
    label: &'a str,
    cc_badge: &'a str,
    value: &'a mut u8,
    default_value: u8,
}

impl<'a> RotaryKnob<'a> {
    pub fn new(label: &'a str, cc_badge: &'a str, value: &'a mut u8) -> Self {
        Self {
            label,
            cc_badge,
            value,
            default_value: 64,
        }
    }

    /// Renders the rotary encoder knob and returns `true` if the value changed this frame.
    pub fn show(self, ui: &mut Ui) -> bool {
        let size = Vec2::new(72.0, 74.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let mut changed = false;

        if response.double_clicked() && *self.value != self.default_value {
            *self.value = self.default_value;
            changed = true;
        } else if response.dragged() {
            let delta_y = -response.drag_delta().y;
            if delta_y.abs() > 0.05 {
                let shift = ui.input(|i| i.modifiers.shift);
                let step = if shift {
                    (delta_y * 0.35).round() as i16
                } else {
                    (delta_y * 1.1).round() as i16
                };
                if step != 0 {
                    let next = (*self.value as i16 + step).clamp(0, 127) as u8;
                    if next != *self.value {
                        *self.value = next;
                        changed = true;
                    }
                }
            }
        }

        if !ui.is_rect_visible(rect) {
            return changed;
        }

        let painter = ui.painter_at(rect);

        // Card well
        painter.rect_filled(rect, CornerRadius::same(4), RolandTheme::WELL_BG);
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(
                1.0,
                if response.hovered() || response.dragged() {
                    RolandTheme::ACCENT_ORANGE
                } else {
                    RolandTheme::BORDER_SUBTLE
                },
            ),
            StrokeKind::Inside,
        );

        // Top Label
        painter.text(
            Pos2::new(rect.center().x, rect.top() + 9.0),
            egui::Align2::CENTER_CENTER,
            self.label,
            egui::FontId::monospace(9.5),
            RolandTheme::TEXT_PRIMARY,
        );

        // Knob geometry
        let center = Pos2::new(rect.center().x, rect.top() + 36.0);
        let radius = 16.0;
        let norm = *self.value as f32 / 127.0;

        // Angles: 135 deg (bottom-left) to 405 deg (bottom-right) in screen space
        let start_angle = std::f32::consts::FRAC_PI_4 * 3.0;
        let sweep = std::f32::consts::FRAC_PI_2 * 3.0;

        // Draw background arc segments
        let n_seg = 24;
        for i in 0..n_seg {
            let t0 = i as f32 / n_seg as f32;
            let t1 = (i + 1) as f32 / n_seg as f32;
            let a0 = start_angle + t0 * sweep;
            let a1 = start_angle + t1 * sweep;
            let p0 = Pos2::new(center.x + radius * a0.cos(), center.y + radius * a0.sin());
            let p1 = Pos2::new(center.x + radius * a1.cos(), center.y + radius * a1.sin());
            let col = if t1 <= norm + 0.01 {
                RolandTheme::ACCENT_ORANGE
            } else {
                Color32::from_rgb(30, 33, 44)
            };
            painter.line_segment([p0, p1], Stroke::new(2.4, col));
        }

        // Inner rotary cap
        painter.circle_filled(center, radius - 4.0, Color32::from_rgb(20, 22, 29));
        painter.circle_stroke(
            center,
            radius - 4.0,
            Stroke::new(1.0, Color32::from_rgb(48, 52, 66)),
        );

        // Indicator needle
        let active_angle = start_angle + norm * sweep;
        let inner_pt = Pos2::new(
            center.x + 3.0 * active_angle.cos(),
            center.y + 3.0 * active_angle.sin(),
        );
        let outer_pt = Pos2::new(
            center.x + (radius - 4.5) * active_angle.cos(),
            center.y + (radius - 4.5) * active_angle.sin(),
        );
        painter.line_segment(
            [inner_pt, outer_pt],
            Stroke::new(2.0, RolandTheme::ACCENT_AMBER),
        );

        // Bottom readout + CC badge
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - 9.0),
            egui::Align2::CENTER_CENTER,
            format!("{} • {:03}", self.cc_badge, *self.value),
            egui::FontId::monospace(8.5),
            RolandTheme::TEXT_SECONDARY,
        );

        changed
    }
}
