//! OLED Screen Studio UI view (`128 × 64` 1-bit Startup & Screensaver Animator).
//!
//! Lets Linux users drop animated `.gif`, `.png`, `.jpg`, or `.webp` files (or pick
//! built-in procedural presets), preview 1-bit dithering on an authentic `128 × 64`
//! dot-matrix simulator, and sync `startup_1..2.bmp` and `screen_saver_1..16.bmp`
//! directly to `EXPORT/PROJECT/PROJECT_XX/PICTURE/`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Slider, Stroke, Ui, Vec2};
use egui_extras::{Size, StripBuilder};
use strum::IntoEnumIterator;

use crate::oled_studio::{
    DemoPreset, DitherMode, GrayFrame, MAX_SCREENSAVER_FRAMES, MonoFrame, OledAdjustments,
    OledExportTarget, dither_frame, export_oled_pack_to_dir, generate_demo_preset,
    image_to_gray_frame, load_frames_from_path,
};
use crate::sample_crate::model::SdCardInfo;
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

        Self {
            source_label: format!("Preset: {preset}"),
            active_preset: preset,
            gray_frames,
            mono_frames,
            adjustments,
            export_target: OledExportTarget::Both,
            target_project_name: "PROJECT_01".to_string(),
            is_playing: true,
            current_frame: 0,
            fps: 8,
            last_frame_step: Instant::now(),
            amber_preview: false,
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

pub fn show_screen_studio_ui(
    ui: &mut Ui,
    state: &mut ScreenStudioState,
    detected_sd: Option<&SdCardInfo>,
) {
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

    // 2. Default target_project_name to first exported project on SD if available
    if let Some(card) = detected_sd
        && !card.exported_projects.is_empty()
        && !card.exported_projects.contains(&state.target_project_name)
        && state.target_project_name == "PROJECT_01"
        && let Some(first_proj) = card.exported_projects.first()
    {
        state.target_project_name = first_proj.clone();
    }

    // 3. Advance flipbook animation timer if playing
    if state.is_playing && !state.mono_frames.is_empty() {
        let interval = Duration::from_millis((1000 / state.fps.max(1)) as u64);
        if state.last_frame_step.elapsed() >= interval {
            state.last_frame_step = Instant::now();
            state.current_frame = (state.current_frame + 1) % state.mono_frames.len();
        }
    }

    // Dynamic bottom deck budget:
    // Base height: 20px header + 22px ingest + 22px dither + 22px thresh + 22px project + 24px action + paddings = 162px
    let mut bottom_deck_h = 162.0;
    if state.show_hardware_guide {
        bottom_deck_h += 48.0;
    }
    if state.status_message.is_some() {
        bottom_deck_h += 18.0;
    }

    // Universal 2-Deck Model: Upper OLED Stage (Hero) + Lower Ingest/Dither/Sync Deck
    StripBuilder::new(ui)
        .size(Size::remainder().at_least(140.0))
        .size(Size::exact(RolandTheme::CARD_GAP))
        .size(Size::exact(bottom_deck_h))
        .vertical(|mut strip| {
            // ================================================================
            // UPPER HERO STAGE (Remainder): Authentic 128×64 OLED Screen & Transport
            // ================================================================
            strip.cell(|ui| {
                RolandTheme::rack_card_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let mut amber = state.amber_preview;
                    RolandTheme::section_header(ui, "HARDWARE OLED SIMULATOR (128×64)", |ui| {
                        if ui
                            .checkbox(&mut amber, RichText::new("Amber Tint").size(9.5))
                            .on_hover_text("Toggle between Classic White OLED and Roland Amber preview")
                            .changed()
                        {
                            state.amber_preview = amber;
                        }
                        ui.separator();
                        ui.label(
                            RichText::new(&state.source_label)
                                .size(9.5)
                                .color(RolandTheme::TEXT_MUTED)
                                .monospace(),
                        );
                    });

                    // 1. Hero OLED Bezel: Reserving 28px for Transport Scrubber below it
                    let bezel_avail_h = (ui.available_height() - 28.0).max(72.0);
                    ui.allocate_ui(Vec2::new(ui.available_width(), bezel_avail_h), |ui| {
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
                    });

                    ui.add_space(2.0);

                    // 2. Docked Transport Bar: Full-Width Frame Scrubber & Playhead Controls
                    ui.horizontal(|ui| {
                        let play_label = if state.is_playing {
                            "Pause"
                        } else {
                            "Play"
                        };
                        let play_btn = egui::Button::new(
                            RichText::new(play_label)
                                .size(9.5)
                                .color(RolandTheme::TEXT_PRIMARY)
                                .strong(),
                        );
                        if ui
                            .add_sized(Vec2::new(54.0, 18.0), play_btn)
                            .clicked()
                        {
                            state.is_playing = !state.is_playing;
                        }

                        let max_idx = state.mono_frames.len().saturating_sub(1);
                        let mut frame_num = state.current_frame + 1;
                        ui.spacing_mut().slider_width = (ui.available_width() - 85.0).max(70.0);
                        if ui
                            .add(
                                Slider::new(&mut frame_num, 1..=(max_idx + 1).max(1))
                                    .prefix("Frame ")
                                    .trailing_fill(true),
                            )
                            .changed()
                        {
                            state.is_playing = false;
                            state.current_frame = (frame_num - 1).min(max_idx);
                        }
                    });
                });
            });

            strip.cell(|_ui| {});

            // ================================================================
            // LOWER CONTROL DECK: Ingestion, 1-Bit Dither & Project Sync
            // ================================================================
            strip.cell(|ui| {
                let mut changed = false;

                RolandTheme::rack_card_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    RolandTheme::section_header(ui, "INGESTION, DITHER & PROJECT SYNC", |ui| {
                        if ui
                            .small_button("Reset")
                            .on_hover_text("Reset dithering and contrast controls to defaults")
                            .clicked()
                        {
                            state.adjustments = OledAdjustments::default();
                            changed = true;
                        }
                    });

                    // Row 1: Source Ingestion & Preset Bar
                    StripBuilder::new(ui)
                        .size(Size::relative(0.36))
                        .size(Size::relative(0.24))
                        .size(Size::remainder())
                        .horizontal(|mut row| {
                            row.cell(|ui| {
                                let btn = egui::Button::new(
                                    RichText::new("Load Image")
                                        .size(9.5)
                                        .color(RolandTheme::ACCENT_ORANGE)
                                        .strong(),
                                )
                                .min_size(Vec2::new(ui.available_width(), 19.0));

                                if ui
                                    .add(btn)
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
                            });

                            row.cell(|ui| {
                                let btn = egui::Button::new(
                                    RichText::new("Paste").size(9.5).color(RolandTheme::TEXT_PRIMARY),
                                )
                                .min_size(Vec2::new(ui.available_width(), 19.0));

                                if ui
                                    .add(btn)
                                    .on_hover_text(
                                        "Paste an image or file path directly from your Linux clipboard",
                                    )
                                    .clicked()
                                {
                                    state.paste_from_clipboard();
                                }
                            });

                            row.cell(|ui| {
                                let mut chosen_preset = state.active_preset;
                                egui::ComboBox::from_id_salt("oled_demo_preset")
                                    .selected_text(
                                        RichText::new(format!("{chosen_preset}"))
                                            .size(9.5),
                                    )
                                    .width(ui.available_width())
                                    .show_ui(ui, |ui| {
                                        for p in DemoPreset::iter() {
                                            ui.selectable_value(
                                                &mut chosen_preset,
                                                p,
                                                p.to_string(),
                                            );
                                        }
                                    });
                                if chosen_preset != state.active_preset {
                                    state.load_preset(chosen_preset);
                                }
                            });
                        });

                    ui.add_space(3.0);

                    // Row 2: 2-Column Form Grid (Dither Mode, Threshold/Contrast, SD Project)
                    RolandTheme::form_grid(ui, "oled_dither_sync_grid", |ui| {
                        // Row A: Dither Mode + Invert
                        RolandTheme::grid_label(ui, "DITHER MODE");
                        ui.horizontal(|ui| {
                            let combo_w = (ui.available_width() - 95.0).max(120.0);
                            egui::ComboBox::from_id_salt("oled_dither_mode")
                                .selected_text(
                                    RichText::new(state.adjustments.dither_mode.to_string())
                                        .size(9.5),
                                )
                                .width(combo_w)
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
                                    RichText::new("Invert B/W").size(9.0),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();

                        // Row B: Threshold & Contrast Sliders
                        RolandTheme::grid_label(ui, "THRESH / CONT");
                        StripBuilder::new(ui)
                            .size(Size::relative(0.5))
                            .size(Size::remainder())
                            .horizontal(|mut s| {
                                s.cell(|ui| {
                                    ui.spacing_mut().slider_width =
                                        (ui.available_width() - 48.0).max(45.0);
                                    if ui
                                        .add(
                                            Slider::new(&mut state.adjustments.threshold, 8..=248)
                                                .show_value(true)
                                                .trailing_fill(true),
                                        )
                                        .on_hover_text("1-Bit Luminance Threshold (8..248)")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });
                                s.cell(|ui| {
                                    ui.spacing_mut().slider_width =
                                        (ui.available_width() - 52.0).max(45.0);
                                    if ui
                                        .add(
                                            Slider::new(
                                                &mut state.adjustments.contrast,
                                                -0.7..=0.9,
                                            )
                                            .prefix("C ")
                                            .fixed_decimals(2)
                                            .trailing_fill(true),
                                        )
                                        .on_hover_text("Pre-dither contrast curve (-0.7..+0.9)")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });
                            });
                        ui.end_row();

                        // Row C: SD Project Slot & Target Pack
                        RolandTheme::grid_label(ui, "SD PROJECT");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("oled_project_slot")
                                .selected_text(
                                    RichText::new(&state.target_project_name)
                                        .size(9.5)
                                        .monospace(),
                                )
                                .width(95.0)
                                .show_ui(ui, |ui| {
                                    for i in 1..=16 {
                                        let slot = format!("PROJECT_{i:02}");
                                        ui.selectable_value(
                                            &mut state.target_project_name,
                                            slot.clone(),
                                            slot,
                                        );
                                    }
                                });

                            let target_w = ui.available_width().max(110.0);
                            egui::ComboBox::from_id_salt("oled_export_target")
                                .selected_text(
                                    RichText::new(state.export_target.to_string()).size(9.5),
                                )
                                .width(target_w)
                                .show_ui(ui, |ui| {
                                    for t in OledExportTarget::iter() {
                                        ui.selectable_value(
                                            &mut state.export_target,
                                            t,
                                            t.to_string(),
                                        );
                                    }
                                });
                        });
                        ui.end_row();
                    });

                    if changed {
                        state.redither_all();
                    }

                    ui.add_space(3.0);

                    // Row 3: Primary Action Bar (Clean text, Roland color scheme, no emojis)
                    let sd_ready = detected_sd.is_some_and(|sd| !sd.is_read_only);
                    StripBuilder::new(ui)
                        .size(Size::relative(0.48))
                        .size(Size::relative(0.32))
                        .size(Size::remainder())
                        .horizontal(|mut bar| {
                            bar.cell(|ui| {
                                let sync_btn = egui::Button::new(
                                    RichText::new("SYNC TO SD PROJECT")
                                        .size(9.5)
                                        .monospace()
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
                                })
                                .min_size(Vec2::new(ui.available_width(), 22.0));

                                if ui
                                    .add_enabled(sd_ready, sync_btn)
                                    .on_hover_text(
                                        "Writes startup_1..2.bmp and screen_saver_1..16.bmp into EXPORT/PROJECT/PROJECT_XX/PICTURE/",
                                    )
                                    .clicked()
                                    && let Some(sd) = detected_sd
                                {
                                    let picture_dir =
                                        sd.project_picture_dir(&state.target_project_name);
                                    match export_oled_pack_to_dir(
                                        &state.mono_frames,
                                        &picture_dir,
                                        state.export_target,
                                    ) {
                                        Ok(files) => {
                                            state.status_message = Some((
                                                format!(
                                                    "Synced {} BMPs to {}/PICTURE/ (Re-import project on SP-404)",
                                                    files.len(),
                                                    state.target_project_name
                                                ),
                                                false,
                                            ));
                                            state.show_hardware_guide = true;
                                        }
                                        Err(e) => {
                                            state.status_message =
                                                Some((format!("SD Sync error: {e}"), true));
                                        }
                                    }
                                }
                            });

                            bar.cell(|ui| {
                                let btn = egui::Button::new(
                                    RichText::new("Export Folder")
                                        .size(9.5)
                                        .color(RolandTheme::TEXT_PRIMARY),
                                )
                                .min_size(Vec2::new(ui.available_width(), 22.0));

                                if ui
                                    .add(btn)
                                    .on_hover_text(
                                        "Export the 1-bit BMP files to any local directory on your PC",
                                    )
                                    .clicked()
                                    && let Some(folder) = rfd::FileDialog::new().pick_folder()
                                {
                                    match export_oled_pack_to_dir(
                                        &state.mono_frames,
                                        &folder,
                                        state.export_target,
                                    ) {
                                        Ok(files) => {
                                            state.status_message = Some((
                                                format!(
                                                    "Exported {} BMP files to {}",
                                                    files.len(),
                                                    folder.display()
                                                ),
                                                false,
                                            ));
                                        }
                                        Err(e) => {
                                            state.status_message =
                                                Some((format!("Export error: {e}"), true));
                                        }
                                    }
                                }
                            });

                            bar.cell(|ui| {
                                let btn = egui::Button::selectable(
                                    state.show_hardware_guide,
                                    RichText::new("Steps").size(9.5),
                                )
                                .min_size(Vec2::new(ui.available_width(), 22.0));

                                if ui.add(btn).clicked() {
                                    state.show_hardware_guide = !state.show_hardware_guide;
                                }
                            });
                        });

                    if state.show_hardware_guide {
                        ui.add_space(3.0);
                        egui::Frame::new()
                            .fill(RolandTheme::WELL_BG)
                            .stroke(Stroke::new(1.0, RolandTheme::BORDER_SUBTLE))
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(
                                        "1. On SP-404MKII: SHIFT+PAD 14 -> EXPORT -> PROJECT to SD card.\n\
                                         2. Click 'SYNC TO SD PROJECT' above to write the 1-bit BMPs.\n\
                                         3. On SP-404MKII: SHIFT+PAD 13 (UTILITY) -> SYSTEM -> Screen Saver: Custom, \
                                            then SHIFT+PAD 14 -> IMPORT -> PROJECT.",
                                    )
                                    .size(9.0)
                                    .color(RolandTheme::ACCENT_AMBER),
                                );
                            });
                    }

                    if let Some((ref msg, is_err)) = state.status_message {
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(msg)
                                .size(9.0)
                                .color(if is_err {
                                    RolandTheme::METER_RED
                                } else {
                                    RolandTheme::METER_GREEN
                                }),
                        );
                    }
                });
            });
        });
}
