use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub struct RolandTheme;

impl RolandTheme {
    // Roland SP-404 Color Palette
    pub const BG_CHARCOAL: Color32 = Color32::from_rgb(22, 22, 25);
    pub const PANEL_BG: Color32 = Color32::from_rgb(28, 28, 33);
    pub const BORDER_COLOR: Color32 = Color32::from_rgb(46, 46, 54);

    // Roland signature amber / orange LEDs
    pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 122, 0);
    pub const ACCENT_AMBER: Color32 = Color32::from_rgb(255, 179, 0);
    pub const ACCENT_AMBER_DIM: Color32 = Color32::from_rgb(90, 60, 0);

    // Audio status & meter colors
    pub const METER_GREEN: Color32 = Color32::from_rgb(34, 197, 94);
    pub const METER_AMBER: Color32 = Color32::from_rgb(245, 158, 11);
    pub const METER_RED: Color32 = Color32::from_rgb(239, 68, 68);
    pub const METER_BG: Color32 = Color32::from_rgb(18, 18, 22);

    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(240, 240, 245);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(140, 140, 155);

    /// Applies the SP-404 dark matte hardware style to the egui context.
    pub fn apply(ctx: &egui::Context) {
        let mut visuals = Visuals::dark();

        visuals.panel_fill = Self::BG_CHARCOAL;
        visuals.window_fill = Self::BG_CHARCOAL;
        visuals.window_stroke = Stroke::new(1.0, Self::BORDER_COLOR);
        visuals.window_corner_radius = CornerRadius::same(8);

        // Widgets (Inactive)
        visuals.widgets.inactive.bg_fill = Self::PANEL_BG;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Self::BORDER_COLOR);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(4);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Self::TEXT_PRIMARY);

        // Widgets (Hovered)
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(36, 36, 44);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Self::ACCENT_ORANGE);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(4);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

        // Widgets (Active / Clicked)
        visuals.widgets.active.bg_fill = Self::ACCENT_ORANGE;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, Self::ACCENT_AMBER);
        visuals.widgets.active.corner_radius = CornerRadius::same(4);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::BLACK);

        // Selection / Accent
        visuals.selection.bg_fill = Self::ACCENT_ORANGE;
        visuals.selection.stroke = Stroke::new(1.0, Self::ACCENT_AMBER);

        ctx.set_visuals(visuals);
    }
}
