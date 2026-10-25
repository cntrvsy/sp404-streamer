use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::ui::theme::RolandTheme;

pub struct VuMeterWidget<'a> {
    channel_label: &'a str,
    level_db: f32,
    peak_hold_db: f32,
    is_clipped: bool,
}

impl<'a> VuMeterWidget<'a> {
    pub fn new(channel_label: &'a str, level_db: f32, peak_hold_db: f32, is_clipped: bool) -> Self {
        Self {
            channel_label,
            level_db,
            peak_hold_db,
            is_clipped,
        }
    }

    /// Converts a dBFS value (-60.0 to 0.0) to a normalized 0.0..=1.0 ratio.
    #[inline]
    fn db_to_norm(db: f32) -> f32 {
        if db <= -60.0 {
            0.0
        } else if db >= 0.0 {
            1.0
        } else {
            (db + 60.0) / 60.0
        }
    }

    pub fn ui(self, ui: &mut Ui, width: f32, height: f32) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(rect);

        // Background track
        painter.rect_filled(rect, CornerRadius::same(3), RolandTheme::METER_BG);
        painter.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, RolandTheme::BORDER_COLOR),
            StrokeKind::Inside,
        );

        let norm_level = Self::db_to_norm(self.level_db);
        let norm_peak = Self::db_to_norm(self.peak_hold_db);

        let active_width = (rect.width() - 4.0) * norm_level;

        // Draw segmented or multi-color level bar
        if active_width > 0.0 {
            let left_x = rect.min.x + 2.0;
            let bar_rect = Rect::from_min_size(
                Pos2::new(left_x, rect.min.y + 2.0),
                Vec2::new(active_width, rect.height() - 4.0),
            );

            // Determine color based on current level
            let bar_color = if self.level_db >= -3.0 {
                RolandTheme::METER_RED
            } else if self.level_db >= -12.0 {
                RolandTheme::METER_AMBER
            } else {
                RolandTheme::METER_GREEN
            };

            painter.rect_filled(bar_rect, CornerRadius::same(2), bar_color);
        }

        // Draw Peak Hold tick mark
        if norm_peak > 0.05 {
            let peak_x = rect.min.x + 2.0 + (rect.width() - 4.0) * norm_peak;
            let peak_color = if self.peak_hold_db >= -3.0 {
                RolandTheme::METER_RED
            } else {
                Color32::WHITE
            };

            painter.line_segment(
                [
                    Pos2::new(peak_x, rect.min.y + 1.0),
                    Pos2::new(peak_x, rect.max.y - 1.0),
                ],
                Stroke::new(1.5, peak_color),
            );
        }

        // Channel Label on the left
        painter.text(
            Pos2::new(rect.min.x + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            self.channel_label,
            egui::FontId::monospace(10.0),
            Color32::from_white_alpha(180),
        );

        // Clip warning dot if clipped
        if self.is_clipped {
            painter.circle_filled(
                Pos2::new(rect.max.x - 48.0, rect.center().y),
                3.0,
                RolandTheme::METER_RED,
            );
        }

        // Readout dB text on the right
        let readout_text = if self.level_db <= -59.5 {
            "-∞ dB".to_string()
        } else {
            format!("{:.1} dB", self.level_db)
        };

        painter.text(
            Pos2::new(rect.max.x - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            readout_text,
            egui::FontId::monospace(10.0),
            Color32::from_white_alpha(200),
        );

        response
    }
}
