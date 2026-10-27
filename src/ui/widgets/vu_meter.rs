//! Roland SP-404 styled stereo VU meter widget.
//!
//! Renders real-time audio levels and decaying peak-hold needles along a -60 dBFS to 0 dBFS track,
//! transitioning from green to Roland amber (-12 dB) to Roland red (-3 dB) with clip indicators.

use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};

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
    pub fn db_to_norm(db: f32) -> f32 {
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

        let channel_badge_w = 22.0;
        let spacing = 5.0;
        let clip_badge_w = 32.0;
        let readout_w = 52.0;

        // 1. Channel Badge [ L ] / [ R ]
        let chan_rect = Rect::from_min_size(rect.min, Vec2::new(channel_badge_w, rect.height()));
        painter.rect_filled(chan_rect, CornerRadius::same(3), RolandTheme::PANEL_BG);
        painter.rect_stroke(
            chan_rect,
            CornerRadius::same(3),
            Stroke::new(1.0, RolandTheme::BORDER_COLOR),
            StrokeKind::Inside,
        );
        painter.text(
            chan_rect.center(),
            egui::Align2::CENTER_CENTER,
            self.channel_label,
            egui::FontId::monospace(10.5),
            RolandTheme::TEXT_PRIMARY,
        );

        // 2. Meter Track
        let track_start_x = rect.min.x + channel_badge_w + spacing;
        let track_end_x = rect.max.x - (clip_badge_w + spacing + readout_w + spacing);
        let track_rect = Rect::from_min_max(
            Pos2::new(track_start_x, rect.min.y),
            Pos2::new(track_end_x, rect.max.y),
        );

        // Recessed track well
        painter.rect_filled(track_rect, CornerRadius::same(3), RolandTheme::METER_BG);
        painter.rect_stroke(
            track_rect,
            CornerRadius::same(3),
            Stroke::new(1.0, RolandTheme::BORDER_SUBTLE),
            StrokeKind::Inside,
        );

        // Inner drawable area for segments
        let inner_track = track_rect.shrink2(Vec2::new(2.5, 2.0));
        let track_w = inner_track.width();

        if track_w > 10.0 {
            // Segment pitch: 2.8px bar + 1.2px gap = 4.0px pitch
            let seg_pitch = 4.0;
            let seg_width = 2.8;
            let num_segments = (track_w / seg_pitch).floor() as usize;

            for i in 0..num_segments {
                let seg_norm = (i as f32 + 0.5) / (num_segments as f32);
                let seg_db = -60.0 + seg_norm * 60.0;
                let seg_x = inner_track.min.x + i as f32 * seg_pitch;
                let seg_rect = Rect::from_min_size(
                    Pos2::new(seg_x, inner_track.min.y),
                    Vec2::new(seg_width, inner_track.height()),
                );

                if seg_db <= self.level_db {
                    let lit_color = if seg_db >= -3.0 {
                        RolandTheme::METER_RED
                    } else if seg_db >= -12.0 {
                        RolandTheme::METER_AMBER
                    } else {
                        RolandTheme::METER_GREEN
                    };
                    painter.rect_filled(seg_rect, CornerRadius::same(1), lit_color);
                } else {
                    painter.rect_filled(
                        seg_rect,
                        CornerRadius::same(1),
                        RolandTheme::METER_SEGMENT_OFF,
                    );
                }
            }

            // Draw Peak Hold needle
            let norm_peak = Self::db_to_norm(self.peak_hold_db);
            if norm_peak > 0.04 {
                let peak_x = (inner_track.min.x + track_w * norm_peak)
                    .clamp(inner_track.min.x, inner_track.max.x - 1.5);
                let peak_color = if self.peak_hold_db >= -3.0 {
                    RolandTheme::METER_RED
                } else if self.peak_hold_db >= -12.0 {
                    RolandTheme::METER_AMBER
                } else {
                    Color32::WHITE
                };

                painter.line_segment(
                    [
                        Pos2::new(peak_x, inner_track.min.y),
                        Pos2::new(peak_x, inner_track.max.y),
                    ],
                    Stroke::new(1.5, peak_color),
                );
            }
        }

        // 3. CLIP Indicator Badge
        let clip_x = track_end_x + spacing;
        let clip_rect = Rect::from_min_size(
            Pos2::new(clip_x, rect.min.y),
            Vec2::new(clip_badge_w, rect.height()),
        );

        if self.is_clipped {
            painter.rect_filled(clip_rect, CornerRadius::same(3), RolandTheme::METER_CLIP);
            painter.rect_stroke(
                clip_rect,
                CornerRadius::same(3),
                Stroke::new(1.0, Color32::from_rgb(255, 120, 150)),
                StrokeKind::Inside,
            );
            painter.text(
                clip_rect.center(),
                egui::Align2::CENTER_CENTER,
                "CLIP",
                egui::FontId::monospace(9.0),
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(clip_rect, CornerRadius::same(3), RolandTheme::PANEL_BG);
            painter.rect_stroke(
                clip_rect,
                CornerRadius::same(3),
                Stroke::new(1.0, RolandTheme::BORDER_SUBTLE),
                StrokeKind::Inside,
            );
            painter.text(
                clip_rect.center(),
                egui::Align2::CENTER_CENTER,
                "CLIP",
                egui::FontId::monospace(9.0),
                RolandTheme::TEXT_MUTED,
            );
        }

        // 4. Numeric Readout Chip
        let readout_x = clip_x + clip_badge_w + spacing;
        let readout_rect = Rect::from_min_max(
            Pos2::new(readout_x, rect.min.y),
            Pos2::new(rect.max.x, rect.max.y),
        );

        painter.rect_filled(readout_rect, CornerRadius::same(3), RolandTheme::WELL_BG);
        painter.rect_stroke(
            readout_rect,
            CornerRadius::same(3),
            Stroke::new(1.0, RolandTheme::BORDER_SUBTLE),
            StrokeKind::Inside,
        );

        let readout_text = if self.level_db <= -59.5 {
            "-∞ dB".to_string()
        } else {
            format!("{:.1}dB", self.level_db)
        };

        let readout_color = if self.level_db >= -3.0 {
            RolandTheme::METER_RED
        } else if self.level_db >= -12.0 {
            RolandTheme::METER_AMBER
        } else {
            RolandTheme::TEXT_PRIMARY
        };

        painter.text(
            readout_rect.center(),
            egui::Align2::CENTER_CENTER,
            readout_text,
            egui::FontId::monospace(10.0),
            readout_color,
        );

        let tooltip_text = if self.level_db <= -59.5 {
            format!("Channel {}: Silent (-∞ dBFS)", self.channel_label)
        } else {
            format!(
                "Channel {}: {:.1} dBFS | Peak Hold: {:.1} dBFS",
                self.channel_label, self.level_db, self.peak_hold_db
            )
        };

        response.on_hover_text(tooltip_text)
    }

    /// Renders a mathematically calibrated decibel scale directly aligned with the meter tracks.
    pub fn draw_scale(ui: &mut Ui, width: f32, height: f32) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(rect);

        let channel_badge_w = 22.0;
        let spacing = 5.0;
        let clip_badge_w = 32.0;
        let readout_w = 52.0;

        let track_start_x = rect.min.x + channel_badge_w + spacing + 2.5;
        let track_end_x = rect.max.x - (clip_badge_w + spacing + readout_w + spacing + 2.5);
        let track_w = (track_end_x - track_start_x).max(10.0);

        let marks: &[(f32, &str)] = &[
            (-60.0, "-60"),
            (-36.0, "-36"),
            (-24.0, "-24"),
            (-18.0, "-18"),
            (-12.0, "-12"),
            (-6.0, "-6"),
            (-3.0, "-3"),
            (0.0, "0"),
        ];

        for &(db, label) in marks {
            let norm = Self::db_to_norm(db);
            let tick_x = track_start_x + norm * track_w;

            // Hairline tick pointing upwards towards the meter track
            painter.line_segment(
                [
                    Pos2::new(tick_x, rect.min.y),
                    Pos2::new(tick_x, rect.min.y + 3.0),
                ],
                Stroke::new(1.0, RolandTheme::BORDER_COLOR),
            );

            let color = if db >= 0.0 {
                RolandTheme::METER_RED
            } else if db >= -12.0 {
                RolandTheme::METER_AMBER
            } else {
                RolandTheme::TEXT_MUTED
            };

            painter.text(
                Pos2::new(tick_x, rect.min.y + 4.0),
                egui::Align2::CENTER_TOP,
                label,
                egui::FontId::monospace(8.5),
                color,
            );
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_to_norm_boundaries() {
        assert_eq!(VuMeterWidget::db_to_norm(-60.0), 0.0);
        assert_eq!(VuMeterWidget::db_to_norm(0.0), 1.0);
        assert_eq!(VuMeterWidget::db_to_norm(-30.0), 0.5);
    }

    #[test]
    fn test_db_to_norm_clamping() {
        assert_eq!(VuMeterWidget::db_to_norm(-100.0), 0.0);
        assert_eq!(VuMeterWidget::db_to_norm(-60.001), 0.0);
        assert_eq!(VuMeterWidget::db_to_norm(3.0), 1.0);
        assert_eq!(VuMeterWidget::db_to_norm(12.0), 1.0);
    }

    #[test]
    fn test_db_to_norm_intermediate() {
        let norm_12 = VuMeterWidget::db_to_norm(-12.0);
        assert!((norm_12 - 0.8).abs() < f32::EPSILON);

        let norm_6 = VuMeterWidget::db_to_norm(-6.0);
        assert!((norm_6 - 0.9).abs() < f32::EPSILON);
    }
}
