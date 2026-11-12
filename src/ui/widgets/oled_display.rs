//! Authentic `128 × 64` SP-404MKII Monochrome OLED Display Simulator Widget.
//!
//! Renders a `MonoFrame` inside a recessed glass bezel with a 1px dot-matrix gap
//! between pixels so users can preview exact 1-bit dithering before syncing to SD.

use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use crate::oled_studio::{MonoFrame, OLED_HEIGHT, OLED_WIDTH};
use crate::ui::theme::RolandTheme;

pub struct OledDisplayWidget<'a> {
    frame: &'a MonoFrame,
    amber_tint: bool,
    frame_index: usize,
    total_frames: usize,
}

impl<'a> OledDisplayWidget<'a> {
    pub fn new(
        frame: &'a MonoFrame,
        amber_tint: bool,
        frame_index: usize,
        total_frames: usize,
    ) -> Self {
        Self {
            frame,
            amber_tint,
            frame_index,
            total_frames,
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let avail_w = ui.available_width();
        // Fit 128x64 cleanly with a 3.0x pixel pitch (384x192) inside the 480px rack card
        let scale = ((avail_w - 16.0) / OLED_WIDTH as f32).clamp(2.0, 3.2);
        let screen_w = OLED_WIDTH as f32 * scale;
        let screen_h = OLED_HEIGHT as f32 * scale;
        let bezel_h = screen_h + 26.0;

        let (bezel_rect, response) =
            ui.allocate_exact_size(Vec2::new(avail_w, bezel_h), Sense::click());

        if !ui.is_rect_visible(bezel_rect) {
            return response;
        }

        let painter = ui.painter_at(bezel_rect);

        // 1. Outer recessed glass bezel
        painter.rect_filled(
            bezel_rect,
            CornerRadius::same(6),
            Color32::from_rgb(6, 7, 9),
        );
        painter.rect_stroke(
            bezel_rect,
            CornerRadius::same(6),
            Stroke::new(1.0, RolandTheme::BORDER_COLOR),
            StrokeKind::Inside,
        );

        // 2. Top bezel telemetry strip
        let header_y = bezel_rect.top() + 10.0;
        painter.text(
            Pos2::new(bezel_rect.left() + 10.0, header_y),
            egui::Align2::LEFT_CENTER,
            "OLED 128×64 • 1-BIT MONOCHROME BMP",
            egui::FontId::monospace(8.5),
            RolandTheme::TEXT_MUTED,
        );
        painter.text(
            Pos2::new(bezel_rect.right() - 10.0, header_y),
            egui::Align2::RIGHT_CENTER,
            format!("FRAME {:02}/{:02}", self.frame_index + 1, self.total_frames),
            egui::FontId::monospace(8.5),
            RolandTheme::ACCENT_AMBER,
        );

        // 3. Active 128x64 pixel matrix area centered horizontally
        let screen_left = bezel_rect.center().x - screen_w * 0.5;
        let screen_top = bezel_rect.top() + 20.0;
        let screen_rect = Rect::from_min_size(
            Pos2::new(screen_left, screen_top),
            Vec2::new(screen_w, screen_h),
        );

        painter.rect_filled(screen_rect, CornerRadius::ZERO, Color32::BLACK);

        let lit_color = if self.amber_tint {
            RolandTheme::ACCENT_AMBER
        } else {
            Color32::from_rgb(242, 246, 252)
        };
        let unlit_dot = Color32::from_rgb(15, 17, 22);
        let dot_size = (scale - 0.65).max(1.0);

        for y in 0..OLED_HEIGHT {
            let py = screen_top + y as f32 * scale;
            for x in 0..OLED_WIDTH {
                let px = screen_left + x as f32 * scale;
                let dot_rect =
                    Rect::from_min_size(Pos2::new(px, py), Vec2::new(dot_size, dot_size));
                if self.frame.get(x, y) {
                    painter.rect_filled(dot_rect, CornerRadius::ZERO, lit_color);
                } else if scale >= 2.5 {
                    painter.rect_filled(dot_rect, CornerRadius::ZERO, unlit_dot);
                }
            }
        }

        // Subtle inner screen border
        painter.rect_stroke(
            screen_rect.expand(1.0),
            CornerRadius::same(2),
            Stroke::new(1.0, Color32::from_rgb(28, 32, 42)),
            StrokeKind::Outside,
        );

        response
    }
}
