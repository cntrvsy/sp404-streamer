//! Mini vector waveform widget for sample transient and peak visualization with live playhead.

use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::ui::theme::RolandTheme;

/// High-resolution vector waveform visualizer rendering 512 transient peak bins with live playhead.
pub struct WaveformView<'a> {
    bins: &'a [f32],
    is_playing: bool,
    progress: f32,
    height: f32,
}

impl<'a> WaveformView<'a> {
    pub fn new(bins: &'a [f32], is_playing: bool, progress: f32) -> Self {
        Self {
            bins,
            is_playing,
            progress,
            height: 48.0,
        }
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Renders the waveform and returns `(Response, Option<seek_ratio>)` if the user clicks/drags to scrub.
    pub fn show(self, ui: &mut Ui) -> (Response, Option<f32>) {
        let desired_size = Vec2::new(ui.available_width(), self.height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        if !ui.is_rect_visible(rect) {
            return (response, None);
        }

        let painter = ui.painter_at(rect);

        // 1. Recessed background frame
        painter.rect(
            rect,
            4.0,
            Color32::from_rgb(10, 11, 15),
            Stroke::new(1.0, RolandTheme::BORDER_SUBTLE),
            egui::StrokeKind::Inside,
        );

        if self.bins.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No audio waveform data",
                egui::FontId::proportional(11.0),
                Color32::from_rgb(90, 95, 110),
            );
            return (response, None);
        }

        // Center zero-crossing line
        let center_y = rect.center().y;
        painter.line_segment(
            [
                Pos2::new(rect.left() + 2.0, center_y),
                Pos2::new(rect.right() - 2.0, center_y),
            ],
            Stroke::new(1.0, Color32::from_rgb(25, 28, 38)),
        );

        // 2. Draw high-resolution continuous vertical waveform lines (per-pixel column)
        let inner_width = (rect.width() - 8.0).max(1.0);
        let n_bins = self.bins.len();
        let max_half_h = (rect.height() / 2.0 - 4.0).max(1.0);
        let playhead_norm = self.progress.clamp(0.0, 1.0);

        let n_columns = (inner_width.floor() as usize).max(1);
        for col in 0..n_columns {
            let x = rect.left() + 4.0 + col as f32;
            let norm = col as f32 / n_columns as f32;

            // Map pixel slice to corresponding bins to find maximum peak amplitude
            let bin_start = (col * n_bins) / n_columns;
            let bin_end = (((col + 1) * n_bins) / n_columns)
                .max(bin_start + 1)
                .min(n_bins);

            let mut amp = 0.0f32;
            for &b in &self.bins[bin_start..bin_end] {
                if b > amp {
                    amp = b;
                }
            }

            let h = (amp.clamp(0.0, 1.0) * max_half_h).max(0.5);

            // Shading: Played portion glows in vibrant amber, unplayed in deep Roland copper
            let line_color = if self.is_playing && norm <= playhead_norm {
                Color32::from_rgb(255, 185, 30) // Active illuminated amber
            } else if self.is_playing {
                Color32::from_rgb(125, 62, 14) // Dimmed unplayed copper
            } else {
                RolandTheme::ACCENT_ORANGE // Roland orange/amber
            };

            painter.line_segment(
                [Pos2::new(x, center_y - h), Pos2::new(x, center_y + h)],
                Stroke::new(1.0, line_color),
            );
        }

        // 3. Hover scrub guide line
        if response.hovered()
            && !response.dragged()
            && let Some(pos) = response.hover_pos()
        {
            let hover_x = pos.x.clamp(rect.left() + 4.0, rect.right() - 4.0);
            painter.line_segment(
                [
                    Pos2::new(hover_x, rect.top() + 2.0),
                    Pos2::new(hover_x, rect.bottom() - 2.0),
                ],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 60)),
            );
        }

        // 4. Playhead Cursor Needle with guide pip
        if self.is_playing {
            let playhead_x = rect.left() + 4.0 + (playhead_norm * inner_width);
            painter.line_segment(
                [
                    Pos2::new(playhead_x, rect.top() + 2.0),
                    Pos2::new(playhead_x, rect.bottom() - 2.0),
                ],
                Stroke::new(1.5, Color32::WHITE),
            );

            // Small indicator pip at the top of the playhead
            let top_pip = [
                Pos2::new(playhead_x - 3.0, rect.top() + 2.0),
                Pos2::new(playhead_x + 3.0, rect.top() + 2.0),
                Pos2::new(playhead_x, rect.top() + 6.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                top_pip.to_vec(),
                Color32::WHITE,
                Stroke::NONE,
            ));
        }

        // 5. Scrubbing interaction (Click or Drag)
        let seek_ratio = if response.clicked() || response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let ratio = ((pos.x - (rect.left() + 4.0)) / inner_width).clamp(0.0, 1.0);
                Some(ratio)
            } else {
                None
            }
        } else {
            None
        };

        (response, seek_ratio)
    }
}
