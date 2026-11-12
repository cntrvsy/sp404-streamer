//! Sample Crate UI panel for fast auditioning, staging, conforming, and SD sync.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Stroke, Ui, Vec2};
use egui_extras::{Column, Size, StripBuilder, TableBuilder};
use parking_lot::{Mutex, RwLock};
use tracing::{error, warn};

use crate::audio::PreviewPlayer;
use crate::sample_crate::conformer::{
    DecodedAudio, decode_audio_file, export_conformed_wav, stage_audio_file,
};
use crate::sample_crate::model::{ConformOptions, SdCardInfo, StagedSample, UnmountedCard};
use crate::sample_crate::sd_detector::{detect_sd_card_full, safe_eject};
use crate::sample_crate::slicer::{
    ChopMode, compute_slice_frames, export_chopped_slices, slice_frames_to_ratios,
};
use crate::ui::theme::RolandTheme;
use crate::ui::widgets::WaveformView;

static NEXT_SAMPLE_ID: AtomicU64 = AtomicU64::new(1);

/// Live progress report for background export operations.
#[derive(Debug, Clone)]
pub enum ExportProgress {
    Idle,
    Exporting {
        current: usize,
        total: usize,
        current_name: String,
    },
    Success(String),
    Error(String),
}

impl ExportProgress {
    pub const fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub const fn is_exporting(&self) -> bool {
        matches!(self, Self::Exporting { .. })
    }

    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// Exclusive row actions dispatched from egui table rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrateRowAction {
    None,
    Select(usize),
    MoveUp(usize),
    MoveDown(usize),
    Remove(usize),
}

/// State of the Sample Crate tab.
pub struct CrateState {
    pub staged_samples: Vec<StagedSample>,
    pub decoded_cache: Arc<RwLock<HashMap<PathBuf, DecodedAudio>>>,
    pub selected_index: Option<usize>,
    pub target_folder_name: String,
    pub options: ConformOptions,
    pub detected_sd: Option<SdCardInfo>,
    pub unmounted_cards: Vec<UnmountedCard>,
    pub last_sd_scan: Instant,
    pub status_message: Option<(String, bool)>, // (message, is_error)
    pub export_status: Arc<Mutex<ExportProgress>>,
    pub export_cancel_flag: Arc<AtomicBool>,
    pub auto_chop_enabled: bool,
    pub chop_slice_count: usize,
    pub chop_even_grid: bool,
    pub chop_sensitivity: f32,
    pub search_filter: String,
}

impl Default for CrateState {
    fn default() -> Self {
        let status = detect_sd_card_full();
        Self {
            staged_samples: Vec::new(),
            decoded_cache: Arc::new(RwLock::new(HashMap::new())),
            selected_index: None,
            target_folder_name: "Kit_01".to_string(),
            options: ConformOptions::default(),
            detected_sd: status.card,
            unmounted_cards: status.unmounted_candidates,
            last_sd_scan: Instant::now(),
            status_message: None,
            export_status: Arc::new(Mutex::new(ExportProgress::Idle)),
            export_cancel_flag: Arc::new(AtomicBool::new(false)),
            auto_chop_enabled: false,
            chop_slice_count: 16,
            chop_even_grid: false,
            chop_sensitivity: 0.65,
            search_filter: String::new(),
        }
    }
}

impl CrateState {
    /// Rescans for SD cards every 2 seconds or fast-drops disconnected cards
    pub fn refresh_sd_if_needed(&mut self) {
        if let Some(ref sd) = self.detected_sd
            && !sd.mount_path.exists()
        {
            self.detected_sd = None;
        }

        if self.last_sd_scan.elapsed() > Duration::from_secs(2) {
            self.force_refresh_sd();
        }
    }

    /// Immediate full hardware rescan and auto-mount
    pub fn force_refresh_sd(&mut self) {
        let status = detect_sd_card_full();
        self.detected_sd = status.card;
        self.unmounted_cards = status.unmounted_candidates;
        self.last_sd_scan = Instant::now();
    }

    /// Stages a selected audio file or all audio files in a selected directory (recursive).
    pub fn ingest_path(&mut self, path: &Path) {
        if path.is_dir() {
            let mut files = Vec::new();
            Self::collect_audio_files(path, &mut files);
            files.sort();

            if self.staged_samples.is_empty()
                && let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
            {
                self.target_folder_name = dir_name.to_string();
            }

            for f in files {
                self.stage_single_file(&f);
            }
        } else if is_supported_audio_extension(path) {
            self.stage_single_file(path);
        }
    }

    fn collect_audio_files(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    Self::collect_audio_files(&p, out);
                } else if is_supported_audio_extension(&p) {
                    out.push(p);
                }
            }
        }
    }

    fn stage_single_file(&mut self, path: &Path) {
        if self.staged_samples.iter().any(|s| s.source_path == path) {
            return;
        }

        let id = NEXT_SAMPLE_ID.fetch_add(1, Ordering::Relaxed);
        match stage_audio_file(id, path, self.options.max_name_len) {
            Ok(sample) => {
                if self.selected_index.is_none() {
                    self.selected_index = Some(self.staged_samples.len());
                }
                self.staged_samples.push(sample);
            }
            Err(e) => {
                warn!("Failed to stage {}: {e}", path.display());
            }
        }
    }
}

fn is_supported_audio_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_lowercase().as_str(),
        "wav" | "mp3" | "flac" | "aif" | "aiff" | "ogg" | "opus" | "m4a" | "aac"
    )
}

/// Renders the Crate view in the given egui UI.
pub fn render_crate_view(ui: &mut Ui, state: &mut CrateState, preview_player: &PreviewPlayer) {
    state.refresh_sd_if_needed();

    if preview_player.is_playing() {
        ui.ctx().request_repaint();
    }

    // 1. Handle Keyboard Navigation (Guarded against typing in TextEdits!)
    handle_keyboard_shortcuts(ui, state, preview_player);

    // Calculate dynamic bottom height for the Export deck
    let mut bottom_deck_h = 138.0;
    if state.status_message.is_some() || !state.export_status.lock().is_idle() {
        bottom_deck_h += 22.0;
    }
    if state
        .detected_sd
        .as_ref()
        .map(|sd| sd.is_read_only)
        .unwrap_or(false)
    {
        bottom_deck_h += 16.0;
    }

    StripBuilder::new(ui)
        .size(Size::remainder().at_least(180.0))
        .size(Size::exact(RolandTheme::CARD_GAP))
        .size(Size::exact(bottom_deck_h))
        .vertical(|mut strip| {
            // ================================================================
            // TOP CARD (Remainder): Staged Samples Table & Waveform Strip
            // ================================================================
            strip.cell(|ui| {
                render_staging_deck(ui, state, preview_player);
            });

            strip.cell(|_ui| {});

            // ================================================================
            // BOTTOM CARD: 2-Column Aligned Conform & SD Export Deck
            // ================================================================
            strip.cell(|ui| {
                render_export_deck(ui, state);
            });
        });
}

fn handle_keyboard_shortcuts(ui: &mut Ui, state: &mut CrateState, preview_player: &PreviewPlayer) {
    let esc_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if esc_pressed && !state.search_filter.is_empty() {
        state.search_filter.clear();
    }

    // Safeguard: Never trigger transport or delete actions when user is typing in a text field
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }

    let space_pressed = ui.input(|i| i.key_pressed(egui::Key::Space));
    let down_pressed = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
    let up_pressed = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
    let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
    let del_pressed =
        ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));

    let n_samples = state.staged_samples.len();

    if ctrl_held {
        if down_pressed
            && let Some(idx) = state.selected_index
            && idx + 1 < n_samples
        {
            state.staged_samples.swap(idx, idx + 1);
            state.selected_index = Some(idx + 1);
        }
        if up_pressed
            && let Some(idx) = state.selected_index
            && idx > 0
        {
            state.staged_samples.swap(idx, idx - 1);
            state.selected_index = Some(idx - 1);
        }
    } else {
        if down_pressed && n_samples > 0 {
            preview_player.stop();
            state.selected_index = Some(match state.selected_index {
                Some(idx) => (idx + 1).min(n_samples - 1),
                None => 0,
            });
        }

        if up_pressed && n_samples > 0 {
            preview_player.stop();
            state.selected_index = Some(match state.selected_index {
                Some(idx) => idx.saturating_sub(1),
                None => 0,
            });
        }
    }

    if del_pressed
        && let Some(idx) = state.selected_index
        && idx < state.staged_samples.len()
    {
        preview_player.stop();
        state.staged_samples.remove(idx);
        if state.staged_samples.is_empty() {
            state.selected_index = None;
        } else {
            state.selected_index = Some(idx.min(state.staged_samples.len() - 1));
        }
    }

    if space_pressed
        && let Some(idx) = state.selected_index
        && let Some(sample) = state.staged_samples.get(idx)
    {
        if preview_player.is_playing() {
            preview_player.stop();
        } else {
            let path = sample.source_path.clone();
            let cache = Arc::clone(&state.decoded_cache);

            let audio = {
                let read = cache.read();
                read.get(&path).cloned()
            };

            let audio_to_play = audio.unwrap_or_else(|| match decode_audio_file(&path) {
                Ok(decoded) => {
                    cache.write().insert(path, decoded.clone());
                    decoded
                }
                Err(e) => {
                    error!("Failed to decode for preview: {e}");
                    DecodedAudio {
                        channels: 1,
                        sample_rate: 48000,
                        samples: vec![Vec::new()],
                    }
                }
            });

            if !audio_to_play.samples.is_empty() && !audio_to_play.samples[0].is_empty() {
                preview_player.play(audio_to_play);
            }
        }
    }
}

pub fn render_global_sd_pill(ui: &mut Ui, state: &mut CrateState) {
    let mut eject_mount: Option<(PathBuf, Option<String>)> = None;
    let mut browse_folder: Option<PathBuf> = None;
    let mut mount_candidate_dev: Option<String> = None;
    let mut needs_refresh = false;
    let is_exporting = state.export_status.lock().is_exporting();

    ui.horizontal(|ui| {
        if let Some(ref sd) = state.detected_sd {
            ui.label(RichText::new("●").color(RolandTheme::METER_GREEN).size(9.0));
            ui.label(
                RichText::new(format!("SD: {}", sd.label))
                    .color(RolandTheme::TEXT_PRIMARY)
                    .strong()
                    .monospace()
                    .size(9.5),
            );
            ui.label(
                RichText::new(format!("({})", sd.free_space_formatted()))
                    .color(RolandTheme::TEXT_MUTED)
                    .size(9.0),
            );

            if sd.is_read_only {
                ui.label(
                    RichText::new("[LOCKED]")
                        .color(RolandTheme::METER_RED)
                        .strong()
                        .monospace()
                        .size(9.0),
                );
            }

            let eject_btn = ui.add_enabled(
                !is_exporting,
                egui::Button::new(
                    RichText::new("⏏ Eject")
                        .color(RolandTheme::TEXT_PRIMARY)
                        .size(9.5),
                )
                .small(),
            );
            if eject_btn.clicked() {
                eject_mount = Some((sd.mount_path.clone(), sd.device_node.clone()));
            }
        } else if let Some(unmounted) = state.unmounted_cards.first() {
            ui.label(
                RichText::new("○")
                    .color(RolandTheme::ACCENT_AMBER)
                    .size(9.0),
            );
            ui.label(
                RichText::new(format!("CARD: {}", unmounted.label))
                    .color(RolandTheme::ACCENT_AMBER)
                    .monospace()
                    .size(9.5),
            );

            if ui
                .small_button("Mount")
                .on_hover_text("Mount via udisks2")
                .clicked()
            {
                mount_candidate_dev = Some(unmounted.device_node.clone());
            }

            if ui.small_button("Browse").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                browse_folder = Some(path);
            }
        } else {
            ui.label(
                RichText::new("○ SD: None")
                    .color(RolandTheme::TEXT_MUTED)
                    .monospace()
                    .size(9.0),
            );

            if ui.small_button("Browse").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                browse_folder = Some(path);
            }
        }

        if ui
            .small_button("↻")
            .on_hover_text("Rescan SD devices")
            .clicked()
        {
            needs_refresh = true;
        }
    });

    if needs_refresh {
        state.force_refresh_sd();
    }

    if let Some(dev) = mount_candidate_dev {
        match crate::sample_crate::sd_detector::mount_block_device(&dev) {
            Ok(mount_path) => {
                let import_path =
                    crate::sample_crate::sd_detector::resolve_import_path(&mount_path);
                state.detected_sd = crate::sample_crate::sd_detector::build_sd_info(
                    &mount_path,
                    &import_path,
                    Some(&dev),
                );
                state.unmounted_cards.clear();
                state.status_message = Some(("✓ SD Card mounted successfully.".into(), false));
            }
            Err(e) => {
                state.status_message = Some((format!("Failed to mount {dev}: {e}"), true));
            }
        }
    }

    if let Some((mount, dev)) = eject_mount {
        state.export_cancel_flag.store(true, Ordering::Relaxed);
        match safe_eject(&mount, dev.as_deref()) {
            Ok(_) => {
                state.detected_sd = None;
                state.status_message = Some(("✓ SD Card safely unmounted.".into(), false));
                state.force_refresh_sd();
            }
            Err(err) => {
                state.status_message = Some((format!("Eject failed: {err}"), true));
            }
        }
    }

    if let Some(path) = browse_folder {
        let import_path = crate::sample_crate::sd_detector::resolve_import_path(&path);
        state.detected_sd =
            crate::sample_crate::sd_detector::build_sd_info(&path, &import_path, None);
    }
}

fn open_add_files_dialog(state: &mut CrateState) {
    let dialog = rfd::FileDialog::new()
        .set_title("Select Audio Samples (Hold Ctrl or Shift to select multiple)")
        .add_filter(
            "Audio Files (*.wav, *.mp3, *.flac, *.aiff, *.ogg, *.opus)",
            &[
                "wav", "mp3", "flac", "aif", "aiff", "ogg", "opus", "m4a", "aac",
            ],
        )
        .add_filter("All Files (*.*)", &["*"]);

    if let Some(files) = dialog.pick_files() {
        for f in files {
            state.ingest_path(&f);
        }
    }
}

fn open_add_folder_dialog(state: &mut CrateState) {
    let dialog = rfd::FileDialog::new()
        .set_title("Select Sample Pack Folder (Recursively imports all audio)");

    if let Some(folder) = dialog.pick_folder() {
        state.ingest_path(&folder);
    }
}

fn render_staging_deck(ui: &mut Ui, state: &mut CrateState, preview_player: &PreviewPlayer) {
    let count = state.staged_samples.len();
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        RolandTheme::section_header(ui, "STAGED SAMPLES & AUDITION", |ui| {
            if count > 0 {
                if ui
                    .button(RichText::new("Clear All").size(9.5).color(RolandTheme::METER_RED))
                    .clicked()
                {
                    preview_player.stop();
                    state.staged_samples.clear();
                    state.selected_index = None;
                }

                if ui
                    .button(RichText::new("+ Folder").size(9.5).color(RolandTheme::TEXT_PRIMARY))
                    .clicked()
                {
                    open_add_folder_dialog(state);
                }

                if ui
                    .button(RichText::new("+ Files").size(9.5).color(RolandTheme::TEXT_PRIMARY))
                    .clicked()
                {
                    open_add_files_dialog(state);
                }

                ui.add(
                    egui::TextEdit::singleline(&mut state.search_filter)
                        .hint_text("Filter...")
                        .desired_width(75.0),
                );

                ui.label(
                    RichText::new(format!("({count})"))
                        .color(RolandTheme::TEXT_MUTED)
                        .monospace()
                        .size(9.5),
                );
            }
        });
            if state.staged_samples.is_empty() {
                // Ingestion Bay Empty State
                egui::Frame::new()
                    .fill(Color32::from_rgb(10, 11, 15))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(35, 38, 50)))
                    .corner_radius(4.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("STAGE SAMPLES FOR SP-404 CONFORM & EXPORT")
                                    .color(RolandTheme::TEXT_PRIMARY)
                                    .strong()
                                    .size(11.5),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(
                                    "Import audio files or folders to audition, normalize, and write to SD",
                                )
                                .color(RolandTheme::TEXT_SECONDARY)
                                .size(10.0),
                            );
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                let available = ui.available_width();
                                ui.add_space(((available - 240.0) / 2.0).max(0.0));
                                if ui
                                    .button(
                                        RichText::new("+ Select Audio Files...")
                                            .strong()
                                            .size(10.5)
                                            .color(RolandTheme::TEXT_PRIMARY),
                                    )
                                    .clicked()
                                {
                                    open_add_files_dialog(state);
                                }
                                if ui
                                    .button(
                                        RichText::new("+ Select Folder...")
                                            .strong()
                                            .size(10.5)
                                            .color(RolandTheme::TEXT_PRIMARY),
                                    )
                                    .clicked()
                                {
                                    open_add_folder_dialog(state);
                                }
                            });
                        });
                    });
            } else {
                let filter_lower = state.search_filter.trim().to_lowercase();
                let is_filtering = !filter_lower.is_empty();
                let n_samples = state.staged_samples.len();

                let mut action = CrateRowAction::None;
                let selected = state.selected_index;

                // Dynamically budget table scroll height based on whether a waveform/chop bar is docked
                let reserved_bottom = if selected.is_some() {
                    if state.auto_chop_enabled { 110.0 } else { 82.0 }
                } else {
                    0.0
                };
                let table_h = (ui.available_height() - reserved_bottom).max(80.0);

                ui.scope(|ui| {
                    ui.visuals_mut().selection.bg_fill = Color32::from_rgb(26, 32, 46);
                    ui.visuals_mut().selection.stroke =
                        Stroke::new(1.0, Color32::from_rgb(52, 64, 88));

                    TableBuilder::new(ui)
                        .striped(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .max_scroll_height(table_h)
                        .sense(egui::Sense::click())
                        .column(Column::exact(24.0))            // # (Pad index)
                        .column(Column::exact(20.0))            // Reorder (↕)
                        .column(Column::remainder().clip(true)) // Sample name & play icon
                        .column(Column::exact(52.0))            // Format (MONO/STEREO)
                        .column(Column::exact(44.0))            // Duration (0:02)
                        .column(Column::exact(20.0))            // Remove (×)
                        .header(18.0, |mut header| {
                            header.col(|ui| {
                                ui.label(
                                    RichText::new("#")
                                        .color(RolandTheme::TEXT_MUTED)
                                        .size(9.0)
                                        .monospace(),
                                );
                            });
                            header.col(|ui| {
                                ui.label(
                                    RichText::new("↕")
                                        .color(RolandTheme::TEXT_MUTED)
                                        .size(9.0)
                                        .monospace(),
                                );
                            });
                            header.col(|ui| {
                                ui.label(
                                    RichText::new("SAMPLE NAME")
                                        .color(RolandTheme::TEXT_MUTED)
                                        .size(9.0)
                                        .strong(),
                                );
                            });
                            header.col(|ui| {
                                ui.label(
                                    RichText::new("FORMAT")
                                        .color(RolandTheme::TEXT_MUTED)
                                        .size(9.0)
                                        .monospace(),
                                );
                            });
                            header.col(|ui| {
                                ui.label(
                                    RichText::new("LENGTH")
                                        .color(RolandTheme::TEXT_MUTED)
                                        .size(9.0)
                                        .monospace(),
                                );
                            });
                            header.col(|_ui| {});
                        })
                        .body(|mut body| {
                            for (idx, sample) in state.staged_samples.iter().enumerate() {
                                if is_filtering
                                    && !sample.clean_name.to_lowercase().contains(&filter_lower)
                                {
                                    continue;
                                }

                                let is_curr = selected == Some(idx);
                                body.row(20.0, |mut row| {
                                    row.set_selected(is_curr);

                                    row.col(|ui| {
                                        ui.label(
                                            RichText::new(format!("{:02}", idx + 1))
                                                .color(if is_curr {
                                                    RolandTheme::ACCENT_ORANGE
                                                } else {
                                                    RolandTheme::TEXT_MUTED
                                                })
                                                .monospace()
                                                .size(9.5),
                                        );
                                    });

                                    row.col(|ui| {
                                        let reorder_btn = egui::Button::new(
                                            RichText::new("↕").size(9.0),
                                        )
                                        .small();
                                        let resp = ui.add_enabled(n_samples > 1, reorder_btn);
                                        if resp.clicked() && idx > 0 {
                                            action = CrateRowAction::MoveUp(idx);
                                        } else if resp.secondary_clicked() && idx + 1 < n_samples {
                                            action = CrateRowAction::MoveDown(idx);
                                        }
                                    });

                                    row.col(|ui| {
                                        if is_curr && preview_player.is_playing() {
                                            ui.label(
                                                RichText::new("▶")
                                                    .color(RolandTheme::ACCENT_ORANGE)
                                                    .size(9.5),
                                            );
                                        }
                                        ui.label(
                                            RichText::new(&sample.clean_name)
                                                .color(if is_curr {
                                                    Color32::WHITE
                                                } else {
                                                    RolandTheme::TEXT_PRIMARY
                                                })
                                                .size(10.5),
                                        );
                                    });

                                    row.col(|ui| {
                                        let (chan_text, chan_color) = if sample.is_mono {
                                            ("MONO", RolandTheme::METER_GREEN)
                                        } else {
                                            ("STEREO", Color32::from_rgb(0, 180, 216))
                                        };
                                        ui.label(
                                            RichText::new(chan_text)
                                                .color(chan_color)
                                                .size(8.5)
                                                .monospace(),
                                        );
                                    });

                                    row.col(|ui| {
                                        ui.label(
                                            RichText::new(sample.duration_formatted())
                                                .color(RolandTheme::TEXT_SECONDARY)
                                                .size(9.5),
                                        );
                                    });

                                    row.col(|ui| {
                                        if ui
                                            .button(
                                                RichText::new("×")
                                                    .color(RolandTheme::TEXT_MUTED)
                                                    .size(10.0),
                                            )
                                            .clicked()
                                        {
                                            action = CrateRowAction::Remove(idx);
                                        }
                                    });

                                    if row.response().clicked() && state.selected_index != Some(idx)
                                    {
                                        action = CrateRowAction::Select(idx);
                                    }
                                });
                            }
                        });
                });

                match action {
                    CrateRowAction::Select(idx) => {
                        preview_player.stop();
                        state.selected_index = Some(idx);
                    }
                    CrateRowAction::MoveUp(idx) if idx > 0 => {
                        state.staged_samples.swap(idx, idx - 1);
                        state.selected_index = Some(idx - 1);
                    }
                    CrateRowAction::MoveDown(idx) if idx + 1 < state.staged_samples.len() => {
                        state.staged_samples.swap(idx, idx + 1);
                        state.selected_index = Some(idx + 1);
                    }
                    CrateRowAction::Remove(idx) => {
                        preview_player.stop();
                        state.staged_samples.remove(idx);
                        if state.staged_samples.is_empty() {
                            state.selected_index = None;
                        } else if let Some(curr) = state.selected_index {
                            state.selected_index = Some(curr.min(state.staged_samples.len() - 1));
                        }
                    }
                    _ => {}
                }

                // Docked Waveform Strip & Auto-Chop
                if let Some(idx) = state.selected_index
                    && let Some(sample) = state.staged_samples.get(idx).cloned()
                {
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("SELECTED: {}.wav", sample.clean_name))
                                .color(RolandTheme::TEXT_PRIMARY)
                                .size(9.5)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} | {}",
                                sample.peak_db_formatted(),
                                sample.duration_formatted()
                            ))
                            .color(RolandTheme::TEXT_MUTED)
                            .size(9.0),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .selectable_label(
                                    state.auto_chop_enabled,
                                    RichText::new("✂ Auto-Chop")
                                        .size(9.0)
                                        .color(if state.auto_chop_enabled {
                                            RolandTheme::TEXT_ON_ACCENT
                                        } else {
                                            RolandTheme::ACCENT_CYAN
                                        })
                                        .strong(),
                                )
                                .on_hover_text(
                                    "Slice loop into numbered WAV files (01_..16_) for SP-404 pads",
                                )
                                .clicked()
                            {
                                state.auto_chop_enabled = !state.auto_chop_enabled;
                            }
                        });
                    });

                    let ensure_decoded = |path: &Path,
                                          cache: &Arc<RwLock<HashMap<PathBuf, DecodedAudio>>>| {
                        if let Some(cached) = cache.read().get(path).cloned() {
                            return Some(cached);
                        }
                        match decode_audio_file(path) {
                            Ok(decoded) => {
                                cache.write().insert(path.to_path_buf(), decoded.clone());
                                Some(decoded)
                            }
                            Err(e) => {
                                error!("Failed to decode sample: {e}");
                                None
                            }
                        }
                    };

                    let (slice_frames, slice_ratios) = if state.auto_chop_enabled
                        && let Some(decoded) =
                            ensure_decoded(&sample.source_path, &state.decoded_cache)
                    {
                        let mode = if state.chop_even_grid {
                            ChopMode::EvenGrid
                        } else {
                            ChopMode::Transient {
                                sensitivity: state.chop_sensitivity,
                            }
                        };
                        let frames = compute_slice_frames(&decoded, state.chop_slice_count, mode);
                        let ratios = slice_frames_to_ratios(&decoded, &frames);
                        (frames, ratios)
                    } else {
                        (Vec::new(), Vec::new())
                    };

                    let is_playing = preview_player.is_playing();
                    let progress = preview_player.progress();
                    let (_resp, seek_ratio) = WaveformView::new(
                        &sample.waveform_overview,
                        is_playing,
                        progress,
                    )
                    .slice_markers(&slice_ratios)
                    .show(ui);

                    if let Some(ratio) = seek_ratio
                        && let Some(audio_to_play) =
                            ensure_decoded(&sample.source_path, &state.decoded_cache)
                        && !audio_to_play.samples.is_empty()
                        && !audio_to_play.samples[0].is_empty()
                    {
                        preview_player.play_at(audio_to_play, ratio);
                    }

                    if state.auto_chop_enabled {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("SLICES:")
                                    .size(9.0)
                                    .color(RolandTheme::ACCENT_CYAN)
                                    .monospace()
                                    .strong(),
                            );
                            for &cnt in &[4usize, 8, 12, 16] {
                                if ui
                                    .selectable_label(
                                        state.chop_slice_count == cnt,
                                        RichText::new(format!("{cnt}")).size(9.0).monospace(),
                                    )
                                    .clicked()
                                {
                                    state.chop_slice_count = cnt;
                                }
                            }

                            ui.checkbox(
                                &mut state.chop_even_grid,
                                RichText::new("Even Grid").size(9.0),
                            );

                            if !state.chop_even_grid {
                                ui.spacing_mut().slider_width = 50.0;
                                ui.add(
                                    egui::Slider::new(&mut state.chop_sensitivity, 0.1..=0.95)
                                        .show_value(false)
                                        .trailing_fill(true),
                                );
                            }

                            let can_chop_sd = state
                                .detected_sd
                                .as_ref()
                                .is_some_and(|sd| !sd.is_read_only);
                            if ui
                                .add_enabled(
                                    can_chop_sd,
                                    egui::Button::new(
                                        RichText::new(format!(
                                            "⚡ Export {} Slices",
                                            slice_frames.len()
                                        ))
                                        .size(9.0)
                                        .color(if can_chop_sd {
                                            RolandTheme::ACCENT_ORANGE
                                        } else {
                                            RolandTheme::TEXT_MUTED
                                        })
                                        .strong(),
                                    )
                                    .small(),
                                )
                                .clicked()
                                && let Some(ref sd) = state.detected_sd
                                && let Some(decoded) =
                                    ensure_decoded(&sample.source_path, &state.decoded_cache)
                            {
                                let dest_dir = sd.import_path.join(&state.target_folder_name);
                                match export_chopped_slices(
                                    &decoded,
                                    &slice_frames,
                                    &sample.clean_name,
                                    &dest_dir,
                                    &state.options,
                                ) {
                                    Ok(written) => {
                                        state.status_message = Some((
                                            format!(
                                                "✓ Exported {} slices to {}/",
                                                written.len(),
                                                state.target_folder_name
                                            ),
                                            false,
                                        ));
                                    }
                                    Err(e) => {
                                        state.status_message =
                                            Some((format!("Auto-chop export error: {e}"), true));
                                    }
                                }
                            }
                        });
                    }
                }
            }
        });
}

fn render_export_deck(ui: &mut Ui, state: &mut CrateState) {
    let export_progress = state.export_status.lock().clone();
    let is_exporting = export_progress.is_exporting();

    let (has_sd, is_ro, free_bytes, target_prefix_label) = if let Some(ref sd) = state.detected_sd {
        (
            true,
            sd.is_read_only,
            sd.free_bytes,
            format!("IMPORT/{}/", state.target_folder_name),
        )
    } else {
        (
            false,
            false,
            0,
            format!("IMPORT/{}/", state.target_folder_name),
        )
    };

    let total_duration: f32 = state.staged_samples.iter().map(|s| s.duration_secs).sum();
    let req_bytes = (total_duration * 192_000.0 * 1.05) as u64;
    let is_space_insufficient =
        has_sd && !state.staged_samples.is_empty() && free_bytes < req_bytes;

    let clean_folder = state.target_folder_name.trim().to_string();
    let is_folder_valid = !clean_folder.is_empty()
        && clean_folder.len() <= 16
        && !clean_folder
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));

    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        RolandTheme::section_header(ui, "CONFORM & SD EXPORT (16-BIT / 48kHz WAV)", |ui| {
            ui.label(
                RichText::new(&target_prefix_label)
                    .size(9.5)
                    .color(RolandTheme::TEXT_MUTED)
                    .monospace(),
            );
        });

        RolandTheme::form_grid(ui, "crate_export_grid", |ui| {
            // Row 1: Target Kit
            RolandTheme::grid_label(ui, "TARGET KIT");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !is_exporting,
                    egui::TextEdit::singleline(&mut state.target_folder_name).desired_width(120.0),
                );

                let is_duplicate = state
                    .detected_sd
                    .as_ref()
                    .map(|sd| sd.existing_kits.contains(&state.target_folder_name))
                    .unwrap_or(false);

                if is_duplicate {
                    ui.label(
                        RichText::new("[Folder Exists]")
                            .color(RolandTheme::ACCENT_AMBER)
                            .monospace()
                            .size(9.5),
                    );
                }
            });
            ui.end_row();

            // Row 2: DSP Options
            RolandTheme::grid_label(ui, "AUDIO DSP");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.normalize_peak, "Normalize (-0.5dB)"),
                );
                ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.trim_start_silence, "Trim Silence"),
                );
                ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.micro_fades, "De-Click"),
                );
                ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.tpdf_dither, "TPDF Dither"),
                );
            });
            ui.end_row();

            // Row 3: Pad Order
            RolandTheme::grid_label(ui, "PAD ORDER");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.pad_numbered, "Prefix 01–16"),
                );
                let hint = if state.options.pad_numbered {
                    "Maps files sequentially onto Pads 1-16 on import"
                } else {
                    "Preserves original filenames for general browsing"
                };
                ui.label(RichText::new(hint).color(RolandTheme::TEXT_MUTED).size(9.0));
            });
            ui.end_row();
        });

        ui.add_space(4.0);

        // Preflight diagnostic warnings
        if is_ro {
            ui.label(
                RichText::new("● CANNOT WRITE: SD CARD IS READ-ONLY (Check LOCK switch)")
                    .color(RolandTheme::METER_RED)
                    .monospace()
                    .size(9.5),
            );
        } else if is_space_insufficient {
            ui.label(
                RichText::new(format!(
                    "● INSUFFICIENT CARD SPACE: Requires ~{:.1} MB",
                    req_bytes as f64 / 1_048_576.0
                ))
                .color(RolandTheme::METER_RED)
                .monospace()
                .size(9.5),
            );
        } else if !is_folder_valid && !clean_folder.is_empty() {
            ui.label(
                RichText::new("● Kit name must be 1-16 chars without special symbols")
                    .color(RolandTheme::METER_RED)
                    .monospace()
                    .size(9.5),
            );
        }

        // Primary Action Button
        let can_export = !is_exporting
            && !state.staged_samples.is_empty()
            && is_folder_valid
            && has_sd
            && !is_ro
            && !is_space_insufficient;

        match export_progress {
            ExportProgress::Exporting {
                current,
                total,
                current_name,
            } => {
                ui.ctx().request_repaint();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Spinner::new()
                            .size(12.0)
                            .color(RolandTheme::ACCENT_ORANGE),
                    );
                    ui.label(
                        RichText::new(format!("Exporting {current}/{total} conformed WAVs..."))
                            .strong()
                            .color(RolandTheme::ACCENT_ORANGE)
                            .size(10.0),
                    );
                });
                ui.add_space(2.0);
                let progress = if total > 0 {
                    current as f32 / total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(progress)
                        .text(format!("{current}/{total} ({current_name})"))
                        .animate(true),
                );
            }
            ExportProgress::Success(ref msg) => {
                ui.horizontal(|ui| {
                    render_export_button(ui, can_export, state);
                    ui.label(
                        RichText::new(msg)
                            .color(RolandTheme::METER_GREEN)
                            .size(10.0),
                    );
                });
            }
            ExportProgress::Error(ref err) => {
                ui.horizontal(|ui| {
                    render_export_button(ui, can_export, state);
                    ui.label(
                        RichText::new(format!("Error: {err}"))
                            .color(RolandTheme::METER_RED)
                            .size(10.0),
                    );
                });
            }
            ExportProgress::Idle => {
                ui.horizontal(|ui| {
                    render_export_button(ui, can_export, state);
                    if let Some((ref msg, is_err)) = state.status_message {
                        let col = if is_err {
                            RolandTheme::METER_RED
                        } else {
                            RolandTheme::METER_GREEN
                        };
                        ui.label(RichText::new(msg).color(col).size(10.0));
                    }
                });
            }
        }
    });
}

fn render_export_button(ui: &mut Ui, can_export: bool, state: &mut CrateState) {
    let is_ro = state
        .detected_sd
        .as_ref()
        .map(|sd| sd.is_read_only)
        .unwrap_or(false);
    let btn_text = if state.detected_sd.is_none() {
        "No SD Card Mounted"
    } else if is_ro {
        "SD Card is Read-Only"
    } else if state.staged_samples.is_empty() {
        "Stage Samples to Export"
    } else {
        "CONFORM & WRITE TO SD (16-BIT / 48kHz)"
    };

    let btn_width = ui.available_width();
    let export_btn = egui::Button::new(
        RichText::new(btn_text)
            .strong()
            .monospace()
            .size(10.5)
            .color(if can_export {
                RolandTheme::TEXT_ON_ACCENT
            } else {
                RolandTheme::TEXT_MUTED
            }),
    )
    .fill(if can_export {
        RolandTheme::ACCENT_ORANGE
    } else {
        Color32::from_rgb(24, 26, 34)
    })
    .min_size(Vec2::new(btn_width, 24.0));

    if ui.add_enabled(can_export, export_btn).clicked() {
        execute_export(state);
    }
}

fn execute_export(state: &mut CrateState) {
    let Some(ref sd) = state.detected_sd else {
        return;
    };

    let target_dir = sd.import_path.join(&state.target_folder_name);
    let options = state.options.clone();
    let samples = state.staged_samples.clone();
    let cache = Arc::clone(&state.decoded_cache);
    let export_status = Arc::clone(&state.export_status);
    let target_folder_name = state.target_folder_name.clone();
    let rel_import_path = sd.relative_import_path();

    state.export_cancel_flag.store(false, Ordering::Relaxed);
    let cancel_flag = Arc::clone(&state.export_cancel_flag);

    *export_status.lock() = ExportProgress::Exporting {
        current: 0,
        total: samples.len(),
        current_name: "Starting export...".to_string(),
    };

    std::thread::spawn(move || {
        let total = samples.len();
        let mut success_count = 0;

        for (idx, sample) in samples.iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                *export_status.lock() = ExportProgress::Idle;
                return;
            }

            *export_status.lock() = ExportProgress::Exporting {
                current: idx + 1,
                total,
                current_name: sample.clean_name.clone(),
            };

            let audio_res = {
                let cache_read = cache.read();
                cache_read.get(&sample.source_path).cloned()
            };

            let audio = match audio_res {
                Some(a) => a,
                None => match decode_audio_file(&sample.source_path) {
                    Ok(a) => {
                        cache.write().insert(sample.source_path.clone(), a.clone());
                        a
                    }
                    Err(e) => {
                        *export_status.lock() = ExportProgress::Error(format!(
                            "Failed to decode {}: {e}",
                            sample.clean_name
                        ));
                        return;
                    }
                },
            };

            let file_name = if options.pad_numbered {
                format!("{:02}_{}.wav", idx + 1, sample.clean_name)
            } else {
                format!("{}.wav", sample.clean_name)
            };

            let dest_file = target_dir.join(file_name);
            if let Err(e) = export_conformed_wav(&audio, &dest_file, &options) {
                *export_status.lock() =
                    ExportProgress::Error(format!("Failed to export {}: {e}", sample.clean_name));
                return;
            }

            success_count += 1;
        }

        *export_status.lock() = ExportProgress::Success(format!(
            "✓ Exported {success_count} WAVs to {rel_import_path}/{target_folder_name}!"
        ));
    });
}
