//! Sample Crate UI panel for fast auditioning, staging, conforming, and SD sync.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Stroke, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
use parking_lot::{Mutex, RwLock};
use tracing::{error, warn};

use crate::audio::PreviewPlayer;
use crate::sample_crate::conformer::{
    DecodedAudio, decode_audio_file, export_conformed_wav, stage_audio_file,
};
use crate::sample_crate::model::{ConformOptions, SdCardInfo, StagedSample, UnmountedCard};
use crate::sample_crate::sd_detector::{detect_sd_card_full, safe_eject};
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
            search_filter: String::new(),
        }
    }
}

impl CrateState {
    /// Rescans for SD cards every 2 seconds or fast-drops disconnected cards
    pub fn refresh_sd_if_needed(&mut self) {
        // Fast drop if the mount directory vanished (e.g. card physically pulled)
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
        // Skip if already staged
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

    // Request continuous 60 FPS repaint while audio preview is playing so waveform needle glides smoothly
    if preview_player.is_playing() {
        ui.ctx().request_repaint();
    }

    // 1. Handle Keyboard Navigation (Spacebar = Audition, Arrows = Select, Ctrl+Arrows = Reorder, Del = Remove, Esc = Clear Filter)
    handle_keyboard_shortcuts(ui, state, preview_player);

    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);

    // 2. Top Deck: Hardware SD Card Status & Eject
    render_sd_card_deck(ui, state);

    // 3. Middle Deck: Staged Samples Table & Waveform
    render_staging_deck(ui, state, preview_player);

    // 4. Bottom Deck: Export & Conforming Controls
    render_export_deck(ui, state);
}

fn handle_keyboard_shortcuts(ui: &mut Ui, state: &mut CrateState, preview_player: &PreviewPlayer) {
    let space_pressed = ui.input(|i| i.key_pressed(egui::Key::Space));
    let down_pressed = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
    let up_pressed = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
    let esc_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
    let del_pressed =
        ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));

    if esc_pressed && !state.search_filter.is_empty() {
        state.search_filter.clear();
    }

    let n_samples = state.staged_samples.len();

    if ctrl_held {
        // Reordering samples in kit sequence via Ctrl+Up / Ctrl+Down
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
        // Normal cursor navigation
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

            // Check cache or decode
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

fn render_sd_card_deck(ui: &mut Ui, state: &mut CrateState) {
    let mut eject_mount: Option<(PathBuf, Option<String>)> = None;
    let mut browse_folder: Option<PathBuf> = None;
    let mut mount_candidate_dev: Option<String> = None;
    let mut needs_refresh = false;
    let is_exporting = state.export_status.lock().is_exporting();

    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some(ref sd) = state.detected_sd {
                ui.label(
                    RichText::new("●")
                        .color(RolandTheme::METER_GREEN)
                        .size(12.0),
                );
                ui.label(
                    RichText::new(format!("SP-404 SD: {}", sd.label))
                        .color(RolandTheme::TEXT_PRIMARY)
                        .strong()
                        .size(11.0),
                );
                ui.label(
                    RichText::new(format!("({})", sd.free_space_formatted()))
                        .color(RolandTheme::TEXT_SECONDARY)
                        .size(11.0),
                );

                if sd.is_read_only {
                    ui.label(
                        RichText::new("[READ-ONLY: CHECK LOCK SWITCH]")
                            .color(RolandTheme::METER_RED)
                            .strong()
                            .monospace()
                            .size(10.0),
                    );
                }

                if ui
                    .small_button("↻")
                    .on_hover_text("Rescan SD card status & capacity")
                    .clicked()
                {
                    needs_refresh = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let eject_btn = ui.add_enabled(
                        !is_exporting,
                        egui::Button::new(
                            RichText::new("Eject Card")
                                .color(RolandTheme::TEXT_PRIMARY)
                                .size(11.0),
                        ),
                    );
                    if is_exporting {
                        eject_btn.on_hover_text("Cannot eject while export is in progress");
                    } else if eject_btn.clicked() {
                        eject_mount = Some((sd.mount_path.clone(), sd.device_node.clone()));
                    }
                });
            } else if let Some(unmounted) = state.unmounted_cards.first() {
                ui.label(
                    RichText::new("○")
                        .color(RolandTheme::ACCENT_AMBER)
                        .size(12.0),
                );
                ui.label(
                    RichText::new(format!(
                        "CARD DETECTED: {} ({})",
                        unmounted.device_node, unmounted.label
                    ))
                    .color(RolandTheme::ACCENT_AMBER)
                    .monospace()
                    .size(10.5),
                );

                if ui
                    .small_button("↻")
                    .on_hover_text("Rescan devices")
                    .clicked()
                {
                    needs_refresh = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new("Browse Directory...").size(11.0))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        browse_folder = Some(path);
                    }

                    if ui
                        .button(
                            RichText::new("Mount Card")
                                .color(RolandTheme::ACCENT_ORANGE)
                                .strong()
                                .size(11.0),
                        )
                        .on_hover_text("Mount this card partition into the system via udisks2")
                        .clicked()
                    {
                        mount_candidate_dev = Some(unmounted.device_node.clone());
                    }
                });
            } else {
                ui.label(
                    RichText::new("○")
                        .color(Color32::from_rgb(120, 125, 140))
                        .size(12.0),
                );
                ui.label(
                    RichText::new("NO SD CARD MOUNTED (Insert card or select directory)")
                        .color(RolandTheme::TEXT_MUTED)
                        .size(11.0),
                );

                if ui
                    .small_button("↻")
                    .on_hover_text("Scan for newly plugged-in SD cards")
                    .clicked()
                {
                    needs_refresh = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new("Browse Directory...").size(11.0))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        browse_folder = Some(path);
                    }
                });
            }
        });
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
                state.status_message = Some((
                    "✓ SD Card safely unmounted. You can unplug it now.".into(),
                    false,
                ));
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
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if state.staged_samples.is_empty() {
            // Header for empty state: minimal and clean
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("STAGED SAMPLES")
                        .color(RolandTheme::ACCENT_ORANGE)
                        .strong()
                        .size(11.0),
                );
                ui.label(
                    RichText::new("(Empty)")
                        .color(RolandTheme::TEXT_MUTED)
                        .size(11.0),
                );
            });

            ui.add_space(6.0);

            // Ingestion Bay: The single, dedicated surface for adding samples when empty
            egui::Frame::new()
                .fill(Color32::from_rgb(10, 11, 15))
                .stroke(Stroke::new(1.0, Color32::from_rgb(35, 38, 50)))
                .corner_radius(4.0)
                .inner_margin(22.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("STAGE SAMPLES FOR SP-404 CONFORM & EXPORT")
                                .color(RolandTheme::TEXT_PRIMARY)
                                .strong()
                                .size(12.0),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("Import audio files or folders to audition, normalize, and write to SD")
                                .color(RolandTheme::TEXT_SECONDARY)
                                .size(10.5),
                        );
                        ui.add_space(6.0);

                        // Supported audio format badges
                        ui.horizontal(|ui| {
                            let available = ui.available_width();
                            ui.add_space(((available - 270.0) / 2.0).max(0.0));
                            for fmt in &["WAV", "AIFF", "FLAC", "MP3", "OGG", "OPUS"] {
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(17, 18, 23))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(36, 40, 52)))
                                    .corner_radius(3.0)
                                    .inner_margin(egui::Margin::symmetric(5, 2))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(*fmt).color(RolandTheme::TEXT_MUTED).size(9.0).monospace());
                                    });
                            }
                        });

                        ui.add_space(14.0);

                        // Exclusive Ingestion Actions: The only place import buttons appear when empty
                        ui.horizontal(|ui| {
                            let available = ui.available_width();
                            ui.add_space(((available - 240.0) / 2.0).max(0.0));
                            if ui
                                .button(RichText::new("+ Select Audio Files...").strong().size(11.0).color(RolandTheme::TEXT_PRIMARY))
                                .on_hover_text("Select audio files (Hold Ctrl or Shift to select multiple)")
                                .clicked()
                            {
                                open_add_files_dialog(state);
                            }
                            if ui
                                .button(RichText::new("+ Select Folder...").strong().size(11.0).color(RolandTheme::TEXT_PRIMARY))
                                .on_hover_text("Import all audio files from a sample pack directory recursively")
                                .clicked()
                            {
                                open_add_folder_dialog(state);
                            }
                        });

                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Hold Ctrl or Shift in the file chooser to select multiple files at once.")
                                .color(RolandTheme::TEXT_MUTED)
                                .size(9.5),
                        );
                    });
                });
        } else {
            // Header / Toolbar: Only shown when samples are staged
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("STAGED SAMPLES")
                        .color(RolandTheme::ACCENT_ORANGE)
                        .strong()
                        .size(11.0),
                );
                ui.label(
                    RichText::new(format!("({} items)", state.staged_samples.len()))
                        .color(RolandTheme::TEXT_SECONDARY)
                        .size(11.0),
                );

                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut state.search_filter)
                        .hint_text("Filter samples...")
                        .desired_width(110.0),
                );
                if !state.search_filter.is_empty()
                    && ui
                        .button(RichText::new("×").size(10.0))
                        .on_hover_text("Clear filter (Esc)")
                        .clicked()
                {
                    state.search_filter.clear();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new("Clear All").size(10.0).color(RolandTheme::METER_RED))
                        .clicked()
                    {
                        preview_player.stop();
                        state.staged_samples.clear();
                        state.selected_index = None;
                    }

                    if ui
                        .button(RichText::new("+ Folder").size(10.0).color(RolandTheme::TEXT_PRIMARY))
                        .on_hover_text("Import all audio files from a directory (recursively)")
                        .clicked()
                    {
                        open_add_folder_dialog(state);
                    }

                    if ui
                        .button(RichText::new("+ Files").size(10.0).color(RolandTheme::TEXT_PRIMARY))
                        .on_hover_text("Select multiple audio files (Ctrl/Shift+Click supported)")
                        .clicked()
                    {
                        open_add_files_dialog(state);
                    }

                    ui.label(
                        RichText::new("Space: Audition | Del: Remove")
                            .color(RolandTheme::TEXT_MUTED)
                            .size(10.0),
                    );
                });
            });

            ui.add_space(4.0);
            let filter_lower = state.search_filter.trim().to_lowercase();
            let is_filtering = !filter_lower.is_empty();
            let n_samples = state.staged_samples.len();

            let mut action = CrateRowAction::None;
            let selected = state.selected_index;

            ui.scope(|ui| {
                ui.visuals_mut().selection.bg_fill = Color32::from_rgb(26, 32, 46);
                ui.visuals_mut().selection.stroke = Stroke::new(1.0, Color32::from_rgb(52, 64, 88));

                TableBuilder::new(ui)
                    .striped(false)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .max_scroll_height(200.0)
                    .sense(egui::Sense::click())
                    .column(Column::exact(26.0))            // # (Pad index)
                    .column(Column::exact(22.0))            // Reorder (↕)
                    .column(Column::remainder().clip(true)) // Sample name & play icon
                    .column(Column::exact(56.0))            // Format (MONO/STEREO)
                    .column(Column::exact(46.0))            // Duration (0:02)
                    .column(Column::exact(22.0))            // Remove (×)
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.label(RichText::new("#").color(RolandTheme::TEXT_MUTED).size(9.5).monospace());
                        });
                        header.col(|ui| {
                            ui.label(RichText::new("↕").color(RolandTheme::TEXT_MUTED).size(9.5).monospace())
                                .on_hover_text("Reorder pad position (Left-click: Up ▲ | Right-click: Down ▼)");
                        });
                        header.col(|ui| {
                            ui.label(RichText::new("SAMPLE NAME").color(RolandTheme::TEXT_MUTED).size(9.5).strong());
                            if is_filtering {
                                let matched_count = state
                                    .staged_samples
                                    .iter()
                                    .filter(|s| s.clean_name.to_lowercase().contains(&filter_lower))
                                    .count();
                                ui.label(
                                    RichText::new(format!("({matched_count} matched)"))
                                        .color(RolandTheme::ACCENT_AMBER)
                                        .size(9.5),
                                );
                            }
                        });
                        header.col(|ui| {
                            ui.label(RichText::new("FORMAT").color(RolandTheme::TEXT_MUTED).size(9.5).monospace());
                        });
                        header.col(|ui| {
                            ui.label(RichText::new("LENGTH").color(RolandTheme::TEXT_MUTED).size(9.5).monospace());
                        });
                        header.col(|_ui| {});
                    })
                    .body(|mut body| {
                        for (idx, sample) in state.staged_samples.iter().enumerate() {
                            if is_filtering && !sample.clean_name.to_lowercase().contains(&filter_lower) {
                                continue;
                            }

                            let is_curr = selected == Some(idx);
                            body.row(22.0, |mut row| {
                                row.set_selected(is_curr);

                                // Col 1: Index
                                row.col(|ui| {
                                    ui.label(
                                        RichText::new(format!("{:02}", idx + 1))
                                            .color(if is_curr { RolandTheme::ACCENT_ORANGE } else { RolandTheme::TEXT_MUTED })
                                            .monospace()
                                            .size(10.0),
                                    );
                                });

                                // Col 2: Single Reorder button (Left-click: Up, Right-click: Down)
                                row.col(|ui| {
                                    let reorder_btn = egui::Button::new(RichText::new("↕").size(9.5)).small();
                                    let resp = ui
                                        .add_enabled(n_samples > 1, reorder_btn)
                                        .on_hover_text(format!(
                                            "Pad #{:02} Order:\n• Left-click: Move Up ▲ (Ctrl+Up)\n• Right-click: Move Down ▼ (Ctrl+Down)",
                                            idx + 1
                                        ));

                                    if resp.clicked() {
                                        if idx > 0 {
                                            action = CrateRowAction::MoveUp(idx);
                                        }
                                    } else if resp.secondary_clicked() && idx + 1 < n_samples {
                                        action = CrateRowAction::MoveDown(idx);
                                    }
                                });

                                // Col 3: Sample Name (with play indicator)
                                row.col(|ui| {
                                    if is_curr && preview_player.is_playing() {
                                        ui.label(RichText::new("▶").color(RolandTheme::ACCENT_ORANGE).size(10.0));
                                    }
                                    ui.label(
                                        RichText::new(&sample.clean_name)
                                            .color(if is_curr { Color32::WHITE } else { RolandTheme::TEXT_PRIMARY })
                                            .size(11.0),
                                    );
                                });

                                // Col 4: Format
                                row.col(|ui| {
                                    let (chan_text, chan_color) = if sample.is_mono {
                                        ("MONO", RolandTheme::METER_GREEN)
                                    } else {
                                        ("STEREO", Color32::from_rgb(0, 180, 216))
                                    };
                                    ui.label(
                                        RichText::new(chan_text)
                                            .color(chan_color)
                                            .size(9.0)
                                            .monospace(),
                                    );
                                });

                                // Col 5: Duration
                                row.col(|ui| {
                                    ui.label(
                                        RichText::new(sample.duration_formatted())
                                            .color(RolandTheme::TEXT_SECONDARY)
                                            .size(10.0),
                                    );
                                });

                                // Col 6: Remove
                                row.col(|ui| {
                                    if ui
                                        .button(RichText::new("×").color(RolandTheme::TEXT_MUTED).size(11.0))
                                        .on_hover_text("Remove from crate")
                                        .clicked()
                                    {
                                        action = CrateRowAction::Remove(idx);
                                    }
                                });

                                if row.response().clicked() && state.selected_index != Some(idx) {
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
        }

        ui.add_space(4.0);

        // Active Sample Waveform Strip with live playhead and scrubbing
        if let Some(idx) = state.selected_index
            && let Some(sample) = state.staged_samples.get(idx) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("SELECTED: {}.wav", sample.clean_name))
                            .color(RolandTheme::TEXT_PRIMARY)
                            .size(10.0)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Peak: {} | Duration: {}",
                            sample.peak_db_formatted(),
                            sample.duration_formatted()
                        ))
                        .color(RolandTheme::TEXT_SECONDARY)
                        .size(10.0),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Click or drag waveform to scrub")
                                .color(RolandTheme::TEXT_MUTED)
                                .size(9.5),
                        );
                    });
                });

                let is_playing = preview_player.is_playing();
                let progress = preview_player.progress();
                let (_resp, seek_ratio) = WaveformView::new(
                    &sample.waveform_overview,
                    is_playing,
                    progress,
                )
                .show(ui);

                if let Some(ratio) = seek_ratio {
                    let path = sample.source_path.clone();
                    let cache = Arc::clone(&state.decoded_cache);

                    let audio = {
                        let read = cache.read();
                        read.get(&path).cloned()
                    };

                    let audio_to_play = audio.unwrap_or_else(|| {
                        match decode_audio_file(&path) {
                            Ok(decoded) => {
                                cache.write().insert(path, decoded.clone());
                                decoded
                            }
                            Err(e) => {
                                error!("Failed to decode for scrub preview: {e}");
                                DecodedAudio {
                                    channels: 1,
                                    sample_rate: 48000,
                                    samples: vec![Vec::new()],
                                }
                            }
                        }
                    });

                    if !audio_to_play.samples.is_empty() && !audio_to_play.samples[0].is_empty() {
                        preview_player.play_at(audio_to_play, ratio);
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
            format!("Target: {}/", sd.relative_import_path()),
        )
    } else {
        (false, false, 0, "Target Folder: IMPORT/".to_string())
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
        // Target folder row & Duplicate warning
        let is_duplicate = state
            .detected_sd
            .as_ref()
            .map(|sd| sd.existing_kits.contains(&state.target_folder_name))
            .unwrap_or(false);

        ui.horizontal(|ui| {
            ui.label(RichText::new(&target_prefix_label).color(RolandTheme::TEXT_PRIMARY).size(11.0));
            let avail_for_edit = if is_duplicate {
                (ui.available_width() - 175.0).max(120.0)
            } else {
                ui.available_width().max(140.0)
            };
            ui.add_enabled(
                !is_exporting,
                egui::TextEdit::singleline(&mut state.target_folder_name)
                    .desired_width(avail_for_edit),
            );

            if is_duplicate {
                ui.label(
                    RichText::new("[Directory exists on card]")
                        .color(RolandTheme::ACCENT_AMBER)
                        .monospace()
                        .size(10.0),
                );
            }
        });

        ui.add_space(2.0);

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.normalize_peak, "Peak Normalize (-0.5 dBFS)"),
                )
                .on_hover_text("Brings the loudest peak in each sample to -0.5 dBFS so kits have consistent volume.");

                let pad_cb = ui.add_enabled(
                    !is_exporting,
                    egui::Checkbox::new(&mut state.options.pad_numbered, "Prefix 01-16 (Pad order)"),
                );
                pad_cb.on_hover_ui(|ui| {
                    ui.set_max_width(300.0);
                    ui.label(
                        RichText::new("Prefix 01-16 (Pad Order)")
                            .strong()
                            .color(RolandTheme::ACCENT_ORANGE),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("• ON (Recommended for Kits):\n  Names files 01_Kick, 02_Snare... Since the SP-404 sorts alphabetically, this loads files straight onto Pad 1 through Pad 16 in exact order.\n\n• OFF (Sample Libraries):\n  Preserves original sample names without numbers for general library browsing.")
                            .size(10.5),
                    );
                });
            });

            // Live explanatory microcopy below checkboxes
            let hint_text = if state.options.pad_numbered {
                "Prefix 01-16 enabled: Files will be numbered to map sequentially to Pads 1-16 on import."
            } else {
                "Prefix 01-16 disabled: Original filenames preserved for general library browsing."
            };
            ui.label(RichText::new(hint_text).color(RolandTheme::TEXT_MUTED).size(9.5));
        });

        ui.add_space(4.0);

        // Preflight diagnostic warnings
        if is_ro {
            ui.label(
                RichText::new("● CANNOT WRITE: SD CARD IS MOUNTED READ-ONLY (Check physical LOCK switch)")
                    .color(RolandTheme::METER_RED)
                    .monospace()
                    .size(10.0),
            );
        } else if is_space_insufficient {
            ui.label(
                RichText::new(format!(
                    "● INSUFFICIENT CARD SPACE: Requires ~{:.1} MB, but card only has {:.1} MB free",
                    req_bytes as f64 / 1_048_576.0,
                    free_bytes as f64 / 1_048_576.0
                ))
                .color(RolandTheme::METER_RED)
                .monospace()
                .size(10.0),
            );
        } else if !is_folder_valid && !clean_folder.is_empty() {
            ui.label(
                RichText::new("● Folder name must be 1-16 chars without / \\ : * ? \" < > |")
                    .color(RolandTheme::METER_RED)
                    .monospace()
                    .size(10.0),
            );
        }

        // Action Button & Animated Progress
        let can_export = !is_exporting
            && !state.staged_samples.is_empty()
            && is_folder_valid
            && has_sd
            && !is_ro
            && !is_space_insufficient;

        match export_progress {
            ExportProgress::Exporting { current, total, current_name } => {
                ui.ctx().request_repaint();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(RolandTheme::ACCENT_ORANGE));
                        ui.label(
                            RichText::new(format!("Exporting {current}/{total} conformed WAVs..."))
                                .strong()
                                .color(RolandTheme::ACCENT_ORANGE)
                                .size(11.0),
                        );
                    });
                    ui.add_space(2.0);
                    let progress = if total > 0 { current as f32 / total as f32 } else { 0.0 };
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .text(format!("{current}/{total} ({current_name})"))
                            .animate(true),
                    );
                });
            }
            ExportProgress::Success(ref msg) => {
                ui.horizontal(|ui| {
                    render_export_button(ui, can_export, state);
                    ui.label(RichText::new(msg).color(RolandTheme::METER_GREEN).size(11.0));
                });
            }
            ExportProgress::Error(ref err) => {
                ui.horizontal(|ui| {
                    render_export_button(ui, can_export, state);
                    ui.label(RichText::new(format!("Error: {err}")).color(RolandTheme::METER_RED).size(11.0));
                });
            }
            ExportProgress::Idle => {
                ui.horizontal(|ui| {
                    render_export_button(ui, can_export, state);
                    if let Some((ref msg, is_err)) = state.status_message {
                        let col = if is_err { RolandTheme::METER_RED } else { RolandTheme::METER_GREEN };
                        ui.label(RichText::new(msg).color(col).size(11.0));
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
        "CONFORM & WRITE TO SD"
    };

    let btn_width = ui.available_width();
    let export_btn = egui::Button::new(RichText::new(btn_text).strong().size(11.5).color(
        if can_export {
            RolandTheme::TEXT_ON_ACCENT
        } else {
            RolandTheme::TEXT_MUTED
        },
    ))
    .min_size(Vec2::new(btn_width, 28.0));

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

            // Decode
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

            // Determine destination filename
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
            "✓ Successfully exported {success_count} conformed WAVs to {rel_import_path}/{target_folder_name}!"
        ));
    });
}
