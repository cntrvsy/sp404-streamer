//! OLED Screen Studio UI view (`128 × 64` 1-bit Startup & Screensaver Animator).
//!
//! Lets Linux users drop animated `.gif`, `.png`, `.jpg`, or `.webp` files (or pick
//! built-in procedural presets), preview 1-bit dithering on an authentic `128 × 64`
//! dot-matrix simulator, and sync `startup_1..2.bmp` and `screen_saver_1..16.bmp`
//! directly to `EXPORT/PROJECT/PROJECT_XX/PICTURE/`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Slider, Stroke, Ui};
use strum::IntoEnumIterator;

use crate::oled_studio::{
    DemoPreset, DitherMode, GrayFrame, MAX_SCREENSAVER_FRAMES, MonoFrame, OledAdjustments,
    OledExportTarget, dither_frame, export_oled_pack_to_dir, generate_demo_preset,
    image_to_gray_frame, load_frames_from_path,
};
use crate::sample_crate::model::SdCardInfo;
use crate::sample_crate::sd_detector::detect_sd_card_full;
use crate::ui::theme::RolandTheme;
use crate::ui::widgets::OledDisplayWidget;

pub struct ScreenStudioState {
    pub source_label: String,
    pub active_preset: DemoPreset,
    pub gray_frames: Vec<GrayFrame>,
    pub mono_frames: Vec<MonoFrame>,
    pub adjustments: OledAdjustments,
    pub export_target: OledExportTarget,
    pub target_project_name: String,
    pub is_playing: bool,
    pub current_frame: usize,
    pub fps: u32,
    pub last_frame_step: Instant,
    pub amber_preview: bool,
    pub detected_sd: Option<SdCardInfo>,
    pub last_sd_scan: Instant,
    pub status_message: Option<(String, bool)>,
    pub show_hardware_guide: bool,
}

impl Default for ScreenStudioState {
    fn default() -> Self {
        let preset = DemoPreset::TapeReels;
        let gray_frames = generate_demo_preset(preset);
        let adjustments = OledAdjustments::default();
        let mono_frames = gray_frames
            .iter()
            .map(|g| dither_frame(g, &adjustments))
            .collect();
        let sd_status = detect_sd_card_full();
        let default_proj = sd_status
            .card
            .as_ref()
            .and_then(|c| c.exported_projects.first().cloned())
            .unwrap_or_else(|| "PROJECT_01".to_string());

        Self {
            source_label: format!("Preset: {preset}"),
            active_preset: preset,
            gray_frames,
            mono_frames,
            adjustments,
            export_target: OledExportTarget::Both,
            target_project_name: default_proj,
            is_playing: true,
            current_frame: 0,
            fps: 8,
            last_frame_step: Instant::now(),
            amber_preview: false,
            detected_sd: sd_status.card,
            last_sd_scan: Instant::now(),
            status_message: None,
            show_hardware_guide: false,
        }
    }
}

impl ScreenStudioState {
    pub fn redither_all(&mut self) {
        self.mono_frames = self
            .gray_frames
            .iter()
            .map(|g| dither_frame(g, &self.adjustments))
            .collect();
        if self.current_frame >= self.mono_frames.len() {
            self.current_frame = 0;
        }
    }

    pub fn load_preset(&mut self, preset: DemoPreset) {
        self.active_preset = preset;
        self.source_label = format!("Preset: {preset}");
        self.gray_frames = generate_demo_preset(preset);
        self.current_frame = 0;
        self.redither_all();
    }

    pub fn load_file(&mut self, path: &std::path::Path) {
        match load_frames_from_path(path) {
            Ok(frames) => {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");
                self.source_label = name.to_string();
                self.gray_frames = frames;
                self.current_frame = 0;
                self.redither_all();
                self.status_message = Some((
                    format!("Loaded {} ({} frames)", name, self.mono_frames.len()),
                    false,
                ));
            }
            Err(e) => {
                self.status_message = Some((format!("Failed to decode image: {e}"), true));
            }
        }
    }

    pub fn paste_from_clipboard(&mut self) {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            self.status_message = Some(("Clipboard unavailable on this session".to_string(), true));
            return;
        };

        // 1. Try raw RGBA image data from clipboard
        if let Ok(img_data) = clipboard.get_image()
            && let Some(rgba) = image::RgbaImage::from_raw(
                img_data.width as u32,
                img_data.height as u32,
                img_data.bytes.into_owned(),
            )
        {
            let dyn_img = image::DynamicImage::ImageRgba8(rgba);
            let frame = image_to_gray_frame(&dyn_img);
            self.gray_frames = vec![frame; MAX_SCREENSAVER_FRAMES];
            self.source_label = "Clipboard Image".to_string();
            self.current_frame = 0;
            self.redither_all();
            self.status_message = Some(("Loaded image from clipboard (128×64)".to_string(), false));
            return;
        }

        // 2. Try file path text from clipboard
        if let Ok(text) = clipboard.get_text() {
            let trimmed = text.trim().trim_start_matches("file://");
            let path = PathBuf::from(trimmed);
            if path.is_file() {
                self.load_file(&path);
                return;
            }
        }

        self.status_message = Some((
            "No image or valid file path found in clipboard".to_string(),
            true,
        ));
    }
}

pub fn show_screen_studio_ui(ui: &mut Ui, state: &mut ScreenStudioState) {
    // 1. Handle Wayland / X11 Drag-and-Drop of GIFs or images
    let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect()
    });
    if let Some(first_path) = dropped.first() {
        state.load_file(first_path);
    }

    // 2. Periodic SD Card scan (every 3 seconds)
    if state.last_sd_scan.elapsed() > Duration::from_secs(3) {
        state.last_sd_scan = Instant::now();
        let status = detect_sd_card_full();
        if let Some(ref card) = status.card
            && state.target_project_name.is_empty()
        {
            state.target_project_name = card
                .exported_projects
                .first()
                .cloned()
                .unwrap_or_else(|| "PROJECT_01".to_string());
        }
        state.detected_sd = status.card;
    }

    // 3. Advance flipbook animation timer if playing
    if state.is_playing && !state.mono_frames.is_empty() {
        let interval = Duration::from_millis((1000 / state.fps.max(1)) as u64);
        if state.last_frame_step.elapsed() >= interval {
            state.last_frame_step = Instant::now();
            state.current_frame = (state.current_frame + 1) % state.mono_frames.len();
        }
    }

    // ====================================================================
    // RACK CARD 1: Source Ingestion & Built-in Demo Presets
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if ui
                .button(
                    RichText::new("📂 Load GIF / Image")
                        .size(10.5)
                        .color(RolandTheme::ACCENT_ORANGE)
                        .strong(),
                )
                .on_hover_text(
                    "Load any animated .gif, .png, .jpg, .webp, or .bmp (auto-crops & samples 16 frames)",
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "Animations & Images",
                        &["gif", "png", "jpg", "jpeg", "webp", "bmp"],
                    )
                    .pick_file()
            {
                state.load_file(&path);
            }

            if ui
                .button(RichText::new("📋 Paste").size(10.5))
                .on_hover_text("Paste an image or file path directly from your Linux clipboard")
                .clicked()
            {
                state.paste_from_clipboard();
            }

            ui.separator();

            ui.label(
                RichText::new("Demo:")
                    .size(9.5)
                    .color(RolandTheme::TEXT_MUTED),
            );
            let mut chosen_preset = state.active_preset;
            egui::ComboBox::from_id_salt("oled_demo_preset")
                .selected_text(RichText::new(chosen_preset.to_string()).size(10.0))
                .width(135.0)
                .show_ui(ui, |ui| {
                    for p in DemoPreset::iter() {
                        ui.selectable_value(&mut chosen_preset, p, p.to_string());
                    }
                });
            if chosen_preset != state.active_preset {
                state.load_preset(chosen_preset);
            }
        });
    });

    ui.add_space(6.0);

    // ====================================================================
    // RACK CARD 2: 128×64 OLED Dot-Matrix Simulator & Transport
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        if let Some(frame) = state.mono_frames.get(state.current_frame) {
            let resp = OledDisplayWidget::new(
                frame,
                state.amber_preview,
                state.current_frame,
                state.mono_frames.len(),
            )
            .show(ui);
            if resp.clicked() {
                state.is_playing = !state.is_playing;
            }
        }

        ui.add_space(5.0);

        ui.horizontal(|ui| {
            let play_label = if state.is_playing {
                "⏸ Pause"
            } else {
                "▶ Play"
            };
            if ui
                .button(RichText::new(play_label).size(10.0).strong())
                .clicked()
            {
                state.is_playing = !state.is_playing;
            }

            let max_idx = state.mono_frames.len().saturating_sub(1);
            let mut frame_num = state.current_frame + 1;
            if ui
                .add(
                    Slider::new(&mut frame_num, 1..=(max_idx + 1).max(1))
                        .text("Frame")
                        .trailing_fill(true),
                )
                .changed()
            {
                state.is_playing = false;
                state.current_frame = (frame_num - 1).min(max_idx);
            }

            ui.checkbox(
                &mut state.amber_preview,
                RichText::new("Amber Tint").size(9.5),
            )
            .on_hover_text("Toggle between Classic White OLED and Roland Amber preview");
        });
    });

    ui.add_space(6.0);

    // ====================================================================
    // RACK CARD 3: 1-Bit Dithering & Contrast Engine
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("DITHER:")
                    .size(9.5)
                    .color(RolandTheme::TEXT_SECONDARY)
                    .monospace()
                    .strong(),
            );
            egui::ComboBox::from_id_salt("oled_dither_mode")
                .selected_text(RichText::new(state.adjustments.dither_mode.to_string()).size(10.0))
                .width(175.0)
                .show_ui(ui, |ui| {
                    for mode in DitherMode::iter() {
                        if ui
                            .selectable_value(
                                &mut state.adjustments.dither_mode,
                                mode,
                                mode.to_string(),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });

            if ui
                .checkbox(
                    &mut state.adjustments.invert,
                    RichText::new("Invert B/W").size(10.0),
                )
                .changed()
            {
                changed = true;
            }

            if ui
                .small_button("↺ Reset")
                .on_hover_text("Reset dithering and contrast controls to defaults")
                .clicked()
            {
                state.adjustments = OledAdjustments::default();
                changed = true;
            }
        });

        ui.add_space(3.0);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("THRESH")
                    .size(9.0)
                    .color(RolandTheme::TEXT_MUTED)
                    .monospace(),
            );
            if ui
                .add(
                    Slider::new(&mut state.adjustments.threshold, 8..=248)
                        .show_value(true)
                        .trailing_fill(true),
                )
                .changed()
            {
                changed = true;
            }

            ui.label(
                RichText::new("CONTRAST")
                    .size(9.0)
                    .color(RolandTheme::TEXT_MUTED)
                    .monospace(),
            );
            if ui
                .add(
                    Slider::new(&mut state.adjustments.contrast, -0.7..=0.9)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .changed()
            {
                changed = true;
            }
        });

        if changed {
            state.redither_all();
        }
    });

    ui.add_space(6.0);

    // ====================================================================
    // RACK CARD 4: SD Project Sync & Hardware Round-Trip Guide
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("PROJECT:")
                    .size(9.5)
                    .color(RolandTheme::TEXT_SECONDARY)
                    .monospace()
                    .strong(),
            );

            egui::ComboBox::from_id_salt("oled_project_slot")
                .selected_text(RichText::new(&state.target_project_name).size(10.0))
                .width(105.0)
                .show_ui(ui, |ui| {
                    for i in 1..=16 {
                        let slot = format!("PROJECT_{i:02}");
                        ui.selectable_value(&mut state.target_project_name, slot.clone(), slot);
                    }
                });

            egui::ComboBox::from_id_salt("oled_export_target")
                .selected_text(RichText::new(state.export_target.to_string()).size(10.0))
                .width(195.0)
                .show_ui(ui, |ui| {
                    for t in OledExportTarget::iter() {
                        ui.selectable_value(&mut state.export_target, t, t.to_string());
                    }
                });
        });

        ui.add_space(5.0);

        ui.horizontal(|ui| {
            let sd_ready = state
                .detected_sd
                .as_ref()
                .is_some_and(|sd| !sd.is_read_only);

            let sync_btn = egui::Button::new(
                RichText::new("⚡ SYNC TO SD PROJECT")
                    .size(10.5)
                    .color(if sd_ready {
                        RolandTheme::TEXT_ON_ACCENT
                    } else {
                        RolandTheme::TEXT_MUTED
                    })
                    .strong(),
            )
            .fill(if sd_ready {
                RolandTheme::ACCENT_ORANGE
            } else {
                Color32::from_rgb(24, 26, 34)
            });

            if ui
                .add_enabled(sd_ready, sync_btn)
                .on_hover_text(
                    "Writes startup_1..2.bmp and screen_saver_1..16.bmp into EXPORT/PROJECT/PROJECT_XX/PICTURE/",
                )
                .clicked()
                && let Some(ref sd) = state.detected_sd
            {
                let picture_dir = sd.project_picture_dir(&state.target_project_name);
                match export_oled_pack_to_dir(
                    &state.mono_frames,
                    &picture_dir,
                    state.export_target,
                ) {
                    Ok(files) => {
                        state.status_message = Some((
                            format!(
                                "Synced {} BMPs to {}/PICTURE/! Now re-import project on SP-404.",
                                files.len(),
                                state.target_project_name
                            ),
                            false,
                        ));
                        state.show_hardware_guide = true;
                    }
                    Err(e) => {
                        state.status_message = Some((format!("SD Sync error: {e}"), true));
                    }
                }
            }

            if ui
                .button(RichText::new("💾 Export Folder...").size(10.0))
                .on_hover_text("Export the 1-bit BMP files to any local directory on your PC")
                .clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder()
            {
                match export_oled_pack_to_dir(&state.mono_frames, &folder, state.export_target) {
                    Ok(files) => {
                        state.status_message = Some((
                            format!("Exported {} BMP files to {}", files.len(), folder.display()),
                            false,
                        ));
                    }
                    Err(e) => {
                        state.status_message = Some((format!("Export error: {e}"), true));
                    }
                }
            }

            if ui
                .selectable_label(
                    state.show_hardware_guide,
                    RichText::new("? Hardware Steps").size(9.5),
                )
                .clicked()
            {
                state.show_hardware_guide = !state.show_hardware_guide;
            }
        });

        if state.show_hardware_guide {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(RolandTheme::WELL_BG)
                .stroke(Stroke::new(1.0, RolandTheme::BORDER_SUBTLE))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(
                            "1. On SP-404MKII: SHIFT+PAD 14 → EXPORT → PROJECT to SD card.\n\
                             2. Click 'SYNC TO SD PROJECT' above to write the 1-bit BMPs.\n\
                             3. On SP-404MKII: SHIFT+PAD 13 (UTILITY) → SYSTEM → Screen Saver: Custom, \
                                then SHIFT+PAD 14 → IMPORT → PROJECT.",
                        )
                        .size(9.0)
                        .color(RolandTheme::ACCENT_AMBER),
                    );
                });
        }

        if let Some((ref msg, is_err)) = state.status_message {
            ui.add_space(3.0);
            ui.label(
                RichText::new(msg)
                    .size(9.5)
                    .color(if is_err {
                        RolandTheme::METER_RED
                    } else {
                        RolandTheme::METER_GREEN
                    }),
            );
        }
    });
}
