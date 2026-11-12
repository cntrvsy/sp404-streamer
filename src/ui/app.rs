//! Main application frame and GUI lifecycle implementation for egui.
//!
//! Renders the compact desktop HUD, dispatches non-blocking commands via the command sender,
//! tracks smooth peak hold decay, and adjusts UI redraw rate adaptively based on signal activity.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Slider, Stroke, Vec2};
use egui_extras::{Size, StripBuilder};
use strum::{Display, EnumIter, IntoEnumIterator, IntoStaticStr};

use crate::audio::command::AudioCommand;
use crate::audio::preview_player::PreviewPlayer;
use crate::audio::state::SharedAudioState;
use crate::ui::crate_view::{self, CrateState};
use crate::ui::screen_view::{self, ScreenStudioState};
use crate::ui::theme::RolandTheme;
use crate::ui::widgets::vu_meter::VuMeterWidget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum AppTab {
    #[strum(to_string = "STREAM HUD")]
    StreamHud,
    #[strum(to_string = "SAMPLE CRATE")]
    SampleCrate,
    #[strum(to_string = "OLED STUDIO")]
    OledStudio,
}

pub struct Sp404StreamerApp {
    state: Arc<SharedAudioState>,
    command_sender: mpsc::Sender<AudioCommand>,
    preview_player: PreviewPlayer,
    crate_state: CrateState,
    screen_state: ScreenStudioState,
    current_tab: AppTab,

    // Smooth peak hold
    peak_hold_l: f32,
    peak_hold_r: f32,
    last_peak_update: Instant,

    // Clip hold timer
    clip_l_time: Option<Instant>,
    clip_r_time: Option<Instant>,

    // Local monitor UI cache
    volume_percent: f32,
}

impl Sp404StreamerApp {
    pub fn new(
        state: Arc<SharedAudioState>,
        command_sender: mpsc::Sender<AudioCommand>,
        preview_player: PreviewPlayer,
    ) -> Self {
        let initial_gain = state.get_monitor_gain();
        let volume_percent = if initial_gain <= 0.01 {
            100.0
        } else {
            initial_gain * 100.0
        };
        state.set_monitor_gain(volume_percent / 100.0);

        // Initial scan of available audio playback sinks via command queue
        let _ = command_sender.send(AudioCommand::RefreshSinks);

        Self {
            state,
            command_sender,
            preview_player,
            crate_state: CrateState::default(),
            screen_state: ScreenStudioState::default(),
            current_tab: AppTab::StreamHud,
            peak_hold_l: -60.0,
            peak_hold_r: -60.0,
            last_peak_update: Instant::now(),
            clip_l_time: None,
            clip_r_time: None,
            volume_percent,
        }
    }

    fn update_peak_hold(&mut self, cur_l: f32, cur_r: f32) {
        let elapsed = self.last_peak_update.elapsed().as_secs_f32();
        self.last_peak_update = Instant::now();

        // Update Left
        if cur_l > self.peak_hold_l {
            self.peak_hold_l = cur_l;
        } else {
            self.peak_hold_l = (self.peak_hold_l - elapsed * 20.0).max(-60.0);
        }

        // Update Right
        if cur_r > self.peak_hold_r {
            self.peak_hold_r = cur_r;
        } else {
            self.peak_hold_r = (self.peak_hold_r - elapsed * 20.0).max(-60.0);
        }
    }
}

impl eframe::App for Sp404StreamerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Read current atomic levels in a single synchronized transaction
        let is_connected = self.state.is_connected();
        let (linear_l, linear_r) = self.state.get_peaks();

        let cur_l = if linear_l > 0.0001 {
            (20.0 * linear_l.log10()).clamp(-60.0, 6.0)
        } else {
            -60.0
        };

        let cur_r = if linear_r > 0.0001 {
            (20.0 * linear_r.log10()).clamp(-60.0, 6.0)
        } else {
            -60.0
        };

        let now = Instant::now();
        if cur_l >= -0.1 {
            self.clip_l_time = Some(now);
        }
        if cur_r >= -0.1 {
            self.clip_r_time = Some(now);
        }

        let is_clipped_l = self
            .clip_l_time
            .map(|t| now.duration_since(t) < Duration::from_millis(1500))
            .unwrap_or(false);

        let is_clipped_r = self
            .clip_r_time
            .map(|t| now.duration_since(t) < Duration::from_millis(1500))
            .unwrap_or(false);

        self.update_peak_hold(cur_l, cur_r);

        // Always keep SD card state freshly synced for global header and tabs
        self.crate_state.refresh_sd_if_needed();

        // Window interior margin
        ui.add_space(4.0);

        // ====================================================================
        // ROOT 3-ZONE VERTICAL STRIPBUILDER
        // Zone 1: Pinned Hardware Header & Tab Switcher (exact 64.0)
        // Zone 2: Fluid Active Tab Body (remainder)
        // Zone 3: Pinned Bottom Guidance & frStudios Footer (exact 24.0)
        // ====================================================================
        StripBuilder::new(ui)
            .size(Size::exact(64.0))
            .size(Size::exact(6.0))
            .size(Size::remainder())
            .size(Size::exact(4.0))
            .size(Size::exact(24.0))
            .vertical(|mut root_strip| {
                // ZONE 1: PINNED GLOBAL HEADER & TAB BAR
                root_strip.cell(|ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(14, 15, 20))
                        .stroke(Stroke::new(1.0, RolandTheme::BORDER_SUBTLE))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            // Row 1: Hardware Telemetry Pills (USB Audio + SD Card)
                            ui.horizontal(|ui| {
                                // Left: USB Audio Pill
                                if is_connected {
                                    ui.label(
                                        RichText::new("●")
                                            .color(RolandTheme::METER_GREEN)
                                            .size(9.0),
                                    );
                                    ui.label(
                                        RichText::new("USB 48kHz")
                                            .color(RolandTheme::METER_GREEN)
                                            .size(9.5)
                                            .monospace()
                                            .strong(),
                                    );
                                    if ui
                                        .small_button("↻")
                                        .on_hover_text("Relink PipeWire stream")
                                        .clicked()
                                        && let Some(node_name) =
                                            self.state.sp404_node_name.read().clone()
                                    {
                                        let _ =
                                            self.command_sender.send(AudioCommand::LinkCapture {
                                                sp404_node: node_name,
                                            });
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("○")
                                            .color(RolandTheme::ACCENT_AMBER)
                                            .size(9.0),
                                    );
                                    ui.label(
                                        RichText::new("USB SCANNING")
                                            .color(RolandTheme::ACCENT_AMBER)
                                            .size(9.5)
                                            .monospace(),
                                    );
                                }

                                ui.separator();

                                // Right: SD Card Pill (Shared source of truth across all tabs)
                                crate_view::render_global_sd_pill(ui, &mut self.crate_state);
                            });

                            ui.add_space(4.0);

                            // Row 2: Symmetrical 3-Column Tab Bar (1/3 + 1/3 + 1/3)
                            StripBuilder::new(ui)
                                .sizes(Size::remainder(), 3)
                                .horizontal(|mut tab_strip| {
                                    for tab in AppTab::iter() {
                                        tab_strip.cell(|ui| {
                                            if RolandTheme::tab_button(
                                                ui,
                                                self.current_tab == tab,
                                                tab.into(),
                                            )
                                            .clicked()
                                            {
                                                self.current_tab = tab;
                                            }
                                        });
                                    }
                                });
                        });
                });

                root_strip.cell(|_ui| {});

                // ZONE 2: FLUID ACTIVE TAB BODY
                root_strip.cell(|ui| match self.current_tab {
                    AppTab::StreamHud => {
                        self.render_stream_hud_tab(
                            ui,
                            is_connected,
                            cur_l,
                            cur_r,
                            is_clipped_l,
                            is_clipped_r,
                        );
                    }
                    AppTab::SampleCrate => {
                        crate_view::render_crate_view(
                            ui,
                            &mut self.crate_state,
                            &self.preview_player,
                        );
                    }
                    AppTab::OledStudio => {
                        screen_view::show_screen_studio_ui(
                            ui,
                            &mut self.screen_state,
                            self.crate_state.detected_sd.as_ref(),
                        );
                    }
                });

                root_strip.cell(|_ui| {});

                // ZONE 3: PINNED FOOTER
                root_strip.cell(|ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(12, 13, 17))
                        .stroke(Stroke::new(1.0, RolandTheme::BORDER_SUBTLE))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (badge_label, detail_label) = match self.current_tab {
                                    AppTab::StreamHud => {
                                        ("DISCORD / OBS:", "Share window to stream PipeWire audio")
                                    }
                                    AppTab::SampleCrate => {
                                        ("CONFORM TARGET:", "48kHz • 16-bit PCM (SD Native)")
                                    }
                                    AppTab::OledStudio => {
                                        ("OLED TARGET:", "128×64 • 1-bit BMP (PROJECT_XX/PICTURE)")
                                    }
                                };
                                ui.label(
                                    RichText::new(badge_label)
                                        .size(9.0)
                                        .color(RolandTheme::ACCENT_AMBER)
                                        .monospace()
                                        .strong(),
                                );
                                ui.label(
                                    RichText::new(detail_label)
                                        .size(9.0)
                                        .color(RolandTheme::TEXT_MUTED),
                                );

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.hyperlink_to(
                                        RichText::new("frStudios")
                                            .size(9.5)
                                            .color(RolandTheme::ACCENT_ORANGE)
                                            .strong(),
                                        "https://frstudios.co.ke/",
                                    );
                                    ui.label(
                                        RichText::new("from")
                                            .size(9.5)
                                            .color(RolandTheme::TEXT_MUTED),
                                    );
                                    ui.label(
                                        RichText::new("♥")
                                            .size(10.0)
                                            .color(Color32::from_rgb(239, 68, 68)),
                                    );
                                    ui.label(
                                        RichText::new("Made with")
                                            .size(9.5)
                                            .color(RolandTheme::TEXT_MUTED),
                                    );
                                });
                            });
                        });
                });
            });

        // --- ADAPTIVE REPAINT RATE ---
        let ctx = ui.ctx().clone();
        match self.current_tab {
            AppTab::SampleCrate => {
                if self.preview_player.is_playing() {
                    ctx.request_repaint_after(Duration::from_millis(16));
                } else {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
            AppTab::OledStudio => {
                if self.screen_state.is_playing {
                    let frame_ms = (1000 / self.screen_state.fps.max(1)).clamp(30, 250) as u64;
                    ctx.request_repaint_after(Duration::from_millis(frame_ms));
                } else {
                    ctx.request_repaint_after(Duration::from_millis(120));
                }
            }
            AppTab::StreamHud => {
                if is_connected && (cur_l > -55.0 || cur_r > -55.0) {
                    ctx.request_repaint_after(Duration::from_millis(16));
                } else {
                    ctx.request_repaint_after(Duration::from_millis(120));
                }
            }
        }
    }
}

impl Sp404StreamerApp {
    fn render_stream_hud_tab(
        &mut self,
        ui: &mut egui::Ui,
        is_connected: bool,
        cur_l: f32,
        cur_r: f32,
        is_clipped_l: bool,
        is_clipped_r: bool,
    ) {
        StripBuilder::new(ui)
            .size(Size::exact(92.0))
            .size(Size::exact(RolandTheme::CARD_GAP))
            .size(Size::exact(132.0))
            .size(Size::exact(RolandTheme::CARD_GAP))
            .size(Size::remainder())
            .vertical(|mut strip| {
                // CARD 1: SIGNAL METERING
                strip.cell(|ui| {
                    let peak_max = cur_l.max(cur_r);
                    let peak_color = if peak_max >= -0.1 {
                        RolandTheme::METER_RED
                    } else if peak_max >= -12.0 {
                        RolandTheme::METER_AMBER
                    } else {
                        RolandTheme::TEXT_MUTED
                    };
                    let peak_str = if peak_max <= -59.5 {
                        "MAX PEAK: -∞ dB".to_string()
                    } else {
                        format!("MAX PEAK: {peak_max:.1} dBFS")
                    };

                    RolandTheme::rack_card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        RolandTheme::section_header(ui, "MASTER INPUT LEVEL", |ui| {
                            ui.label(
                                RichText::new(peak_str)
                                    .size(9.0)
                                    .color(peak_color)
                                    .monospace()
                                    .strong(),
                            );
                        });

                        let meter_w = ui.available_width();
                        VuMeterWidget::new("L", cur_l, self.peak_hold_l, is_clipped_l)
                            .ui(ui, meter_w, 16.0);
                        ui.add_space(2.0);
                        VuMeterWidget::new("R", cur_r, self.peak_hold_r, is_clipped_r)
                            .ui(ui, meter_w, 16.0);
                        ui.add_space(2.0);
                        VuMeterWidget::draw_scale(ui, meter_w, 12.0);
                    });
                });

                strip.cell(|_ui| {});

                // CARD 2: MONITOR & ROUTING
                strip.cell(|ui| {
                    RolandTheme::rack_card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let (chip_color, chip_text) = if self.volume_percent <= 0.01 {
                            (RolandTheme::METER_RED, "MUTED (0%)".to_string())
                        } else if (self.volume_percent - 100.0).abs() < 0.5 {
                            (RolandTheme::ACCENT_AMBER, "100% (0.0 dB)".to_string())
                        } else {
                            let db = 20.0 * (self.volume_percent / 100.0).log10();
                            (
                                RolandTheme::TEXT_PRIMARY,
                                format!("{:.0}% ({:+0.1} dB)", self.volume_percent, db),
                            )
                        };

                        let mut reset_clicked = false;
                        RolandTheme::section_header(ui, "MONITOR & OUTPUT ROUTING", |ui| {
                            if ui
                                .small_button("0 dB Unity")
                                .on_hover_text("Reset monitor volume to 100% (0 dB Unity Gain)")
                                .clicked()
                            {
                                reset_clicked = true;
                            }

                            ui.label(
                                RichText::new(format!("[ {chip_text} ]"))
                                    .size(9.5)
                                    .monospace()
                                    .strong()
                                    .color(chip_color),
                            );
                        });

                        if reset_clicked {
                            self.volume_percent = 100.0;
                            self.state.set_monitor_gain(1.0);
                        }

                        RolandTheme::form_grid(ui, "monitor_routing_grid", |ui| {
                            // Row 1: Monitor Gain Slider
                            RolandTheme::grid_label(ui, "MONITOR GAIN");
                            ui.horizontal(|ui| {
                                let slider = Slider::new(&mut self.volume_percent, 0.0..=150.0)
                                    .trailing_fill(true)
                                    .show_value(false);
                                let fader_w = ui.available_width();
                                if ui.add_sized(Vec2::new(fader_w, 20.0), slider).changed() {
                                    let gain = self.volume_percent / 100.0;
                                    self.state.set_monitor_gain(gain);
                                }
                            });
                            ui.end_row();

                            // Row 2: Audio Sink Selector
                            RolandTheme::grid_label(ui, "AUDIO SINK");
                            ui.horizontal(|ui| {
                                let current_sink = self.state.selected_sink_name.read().clone();
                                let available = self.state.available_sinks.read().clone();

                                let display_text = match &current_sink {
                                    None => "System Default (Follow OS)".to_string(),
                                    Some(name) => available
                                        .iter()
                                        .find(|s| &s.name == name)
                                        .map(|s| s.description.clone())
                                        .unwrap_or_else(|| name.clone()),
                                };

                                let combo_w = (ui.available_width() - 26.0).max(100.0);
                                egui::ComboBox::from_id_salt("output_sink_selector")
                                    .selected_text(RichText::new(display_text).size(10.0))
                                    .width(combo_w)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                current_sink.is_none(),
                                                "System Default (Follow OS)",
                                            )
                                            .clicked()
                                        {
                                            let _ = self.command_sender.send(
                                                AudioCommand::RoutePlayback { sink_name: None },
                                            );
                                        }
                                        for sink in &available {
                                            let is_selected =
                                                current_sink.as_deref() == Some(&sink.name);
                                            if ui
                                                .selectable_label(is_selected, &sink.description)
                                                .clicked()
                                            {
                                                let _ = self.command_sender.send(
                                                    AudioCommand::RoutePlayback {
                                                        sink_name: Some(sink.name.clone()),
                                                    },
                                                );
                                            }
                                        }
                                    });

                                if ui
                                    .small_button("↻")
                                    .on_hover_text("Refresh sinks")
                                    .clicked()
                                {
                                    let _ = self.command_sender.send(AudioCommand::RefreshSinks);
                                }
                            });
                            ui.end_row();

                            // Row 3: PC Speakers Pass-Through
                            RolandTheme::grid_label(ui, "PC SPEAKERS");
                            ui.horizontal(|ui| {
                                let is_unlinked = self.state.is_local_monitor_unlinked();
                                let (status_text, status_color, btn_label) = if is_unlinked {
                                    (
                                        "● MUTED",
                                        RolandTheme::ACCENT_AMBER,
                                        "🔊 Enable Pass-Through",
                                    )
                                } else {
                                    ("● LIVE", RolandTheme::METER_GREEN, "🔇 Mute Speakers")
                                };

                                ui.label(
                                    RichText::new(status_text)
                                        .size(9.5)
                                        .monospace()
                                        .strong()
                                        .color(status_color),
                                );

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let btn = egui::Button::new(
                                        RichText::new(btn_label)
                                            .size(9.5)
                                            .monospace()
                                            .strong()
                                            .color(RolandTheme::TEXT_PRIMARY),
                                    );
                                    if ui.add(btn).clicked() {
                                        let new_unlinked = !is_unlinked;
                                        let _ = self
                                            .command_sender
                                            .send(AudioCommand::SetLocalMonitorMute(new_unlinked));
                                    }
                                });
                            });
                            ui.end_row();
                        });
                    });
                });

                strip.cell(|_ui| {});

                // CARD 3: PIPEWIRE STREAM TELEMETRY
                strip.cell(|ui| {
                    RolandTheme::rack_card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let device_desc = self.state.device_name.read().clone();
                        RolandTheme::section_header(ui, "PIPEWIRE STREAM TELEMETRY", |ui| {
                            ui.label(
                                RichText::new(device_desc)
                                    .size(9.0)
                                    .color(RolandTheme::TEXT_MUTED)
                                    .monospace(),
                            );
                        });

                        RolandTheme::form_grid(ui, "stream_telemetry_grid", |ui| {
                            RolandTheme::grid_label(ui, "USB STATUS");
                            ui.label(
                                RichText::new(if is_connected {
                                    "Connected (SP-404MKII Mix Out)"
                                } else {
                                    "Waiting for SP-404MKII USB-C audio..."
                                })
                                .size(9.5)
                                .color(if is_connected {
                                    RolandTheme::METER_GREEN
                                } else {
                                    RolandTheme::ACCENT_AMBER
                                }),
                            );
                            ui.end_row();

                            RolandTheme::grid_label(ui, "FORMAT");
                            ui.label(
                                RichText::new(
                                    "48000 Hz • Stereo • Linear Float32 (PipeWire Native)",
                                )
                                .size(9.5)
                                .color(RolandTheme::TEXT_SECONDARY)
                                .monospace(),
                            );
                            ui.end_row();

                            RolandTheme::grid_label(ui, "DISCORD SETUP");
                            ui.label(
                                RichText::new(
                                    "Share this window in Discord/Vesktop to stream 48kHz audio",
                                )
                                .size(9.5)
                                .color(RolandTheme::TEXT_MUTED),
                            );
                            ui.end_row();
                        });
                    });
                });
            });
    }
}
