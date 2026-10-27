use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

pub struct RolandTheme;

impl RolandTheme {
    // Pro Studio Rackmount Palette (FabFilter / Universal Audio style)
    pub const BG_CHARCOAL: Color32 = Color32::from_rgb(11, 12, 16); // Deep Obsidian
    pub const PANEL_BG: Color32 = Color32::from_rgb(17, 18, 23); // Sleek Studio Card
    pub const WELL_BG: Color32 = Color32::from_rgb(9, 10, 13); // Recessed Meter/Display Well
    pub const BORDER_COLOR: Color32 = Color32::from_rgb(36, 39, 50); // Fine Hairline Border
    pub const BORDER_SUBTLE: Color32 = Color32::from_rgb(26, 28, 36);

    // Roland signature amber / orange LEDs
    pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 122, 0);
    pub const ACCENT_AMBER: Color32 = Color32::from_rgb(255, 179, 0);
    pub const ACCENT_AMBER_DIM: Color32 = Color32::from_rgb(90, 60, 0);
    pub const ACCENT_CYAN: Color32 = Color32::from_rgb(56, 189, 248);

    // Audio status & meter colors
    pub const METER_GREEN: Color32 = Color32::from_rgb(16, 185, 129); // Studio emerald
    pub const METER_AMBER: Color32 = Color32::from_rgb(245, 158, 11); // Studio warm amber
    pub const METER_RED: Color32 = Color32::from_rgb(239, 68, 68); // Studio crimson red
    pub const METER_CLIP: Color32 = Color32::from_rgb(255, 45, 85); // Glowing clip red
    pub const METER_BG: Color32 = Color32::from_rgb(9, 10, 13); // Recessed track
    pub const METER_SEGMENT_OFF: Color32 = Color32::from_rgb(20, 22, 28); // Unlit LED ghosting

    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(243, 244, 246);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(156, 163, 175);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(100, 108, 124);
    /// High-contrast text color designed specifically to sit atop ACCENT_ORANGE / ACCENT_AMBER
    pub const TEXT_ON_ACCENT: Color32 = Color32::BLACK;

    /// Applies the Pro Studio dark obsidian hardware style to the egui context.
    pub fn apply(ctx: &egui::Context) {
        let mut visuals = Visuals::dark();

        visuals.panel_fill = Self::BG_CHARCOAL;
        visuals.window_fill = Self::BG_CHARCOAL;
        visuals.window_stroke = Stroke::new(1.0, Self::BORDER_COLOR);
        visuals.window_corner_radius = CornerRadius::same(6);

        // Widgets (Inactive) - Recessed well for sliders/inputs, visible hairline border
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(10, 11, 15);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(46, 50, 64));
        visuals.widgets.inactive.corner_radius = CornerRadius::same(4);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.5, Self::TEXT_PRIMARY);

        // Widgets (Hovered)
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(22, 24, 32);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Self::ACCENT_ORANGE);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(4);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);

        // Widgets (Active / Clicked)
        visuals.widgets.active.bg_fill = Self::ACCENT_ORANGE;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, Self::ACCENT_AMBER);
        visuals.widgets.active.corner_radius = CornerRadius::same(4);
        visuals.widgets.active.fg_stroke = Stroke::new(1.5, Self::TEXT_ON_ACCENT);

        // Widgets (Open / Dropdowns / Popups)
        visuals.widgets.open.bg_fill = Color32::from_rgb(22, 24, 32);
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, Self::ACCENT_ORANGE);
        visuals.widgets.open.corner_radius = CornerRadius::same(4);
        visuals.widgets.open.fg_stroke = Stroke::new(1.5, Self::TEXT_PRIMARY);

        // Selection / Accent (used for Slider trailing_fill active track and selectable labels)
        visuals.selection.bg_fill = Self::ACCENT_ORANGE;
        visuals.selection.stroke = Stroke::new(1.0, Self::TEXT_ON_ACCENT);

        // Track and well backgrounds
        visuals.extreme_bg_color = Self::WELL_BG;

        ctx.set_visuals(visuals);
    }

    /// Standardized navigation tab button with guaranteed contrast across all states.
    pub fn tab_button(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
        let text = egui::RichText::new(label)
            .color(if selected {
                Self::TEXT_ON_ACCENT
            } else {
                Self::TEXT_SECONDARY
            })
            .strong()
            .size(11.0);
        ui.selectable_label(selected, text)
    }

    /// Helper for rendering sleek rackmount card panels
    pub fn rack_card_frame() -> egui::Frame {
        egui::Frame::new()
            .fill(Self::PANEL_BG)
            .stroke(Stroke::new(1.0, Self::BORDER_COLOR))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(12, 10))
    }
}
