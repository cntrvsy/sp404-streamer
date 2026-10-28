//! Main application frame and GUI lifecycle implementation for egui.
//!
//! Renders the compact desktop HUD, dispatches non-blocking commands via the command sender,
//! tracks smooth peak hold decay, and adjusts UI redraw rate adaptively based on signal activity.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Slider, Stroke, Vec2};

use strum::{Display, EnumIter, IntoEnumIterator, IntoStaticStr};

use crate::audio::command::AudioCommand;
use crate::audio::preview_player::PreviewPlayer;
use crate::audio::state::SharedAudioState;
use crate::ui::crate_view::{self, CrateState};
use crate::ui::theme::RolandTheme;
use crate::ui::widgets::vu_meter::VuMeterWidget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum AppTab {
    #[strum(to_string = "STREAM HUD")]
    StreamHud,
    #[strum(to_string = "SAMPLE CRATE")]
    SampleCrate,
}

pub struct Sp404StreamerApp {
    state: Arc<SharedAudioState>,
    command_sender: mpsc::Sender<AudioCommand>,
    preview_player: PreviewPlayer,
    crate_state: CrateState,
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
        // Ensure volume defaults to 100% unity gain (1.0) so audio always passes through by default
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
            // Decay by ~20 dB per second
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

        // Safe linear amplitude to dBFS conversion guarded against log10(0.0) == -inf
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

        // Check and handle clipping (> -0.1 dBFS)
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

        // Window interior margin
        ui.add_space(6.0);

        // ====================================================================
        // TOP DECK: Tab Switcher (Segmented Control Pill Container)
        // ====================================================================
        egui::Frame::new()
            .fill(Color32::from_rgb(14, 15, 20))
            .stroke(Stroke::new(1.0, RolandTheme::BORDER_SUBTLE))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(4, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    for tab in AppTab::iter() {
                        if RolandTheme::tab_button(ui, self.current_tab == tab, tab.into())
                            .clicked()
                        {
                            self.current_tab = tab;
                        }
                    }
                });
            });

        ui.add_space(8.0);

        match self.current_tab {
            AppTab::StreamHud => {
                // ====================================================================
                // RACK CARD 1: Master Device Header & Telemetry
                // ====================================================================
                RolandTheme::rack_card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("SP-404MKII")
                                .color(RolandTheme::ACCENT_ORANGE)
                                .strong()
                                .size(13.5),
                        );
                        ui.label(
                            RichText::new("STUDIO BRIDGE")
                                .color(RolandTheme::TEXT_SECONDARY)
                                .size(11.0)
                                .monospace(),
                        );

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if is_connected {
                                if ui
                                    .small_button("↻")
                                    .on_hover_text("Refresh PipeWire audio links to SP-404MKII")
                                    .clicked()
                                    && let Some(node_name) =
                                        self.state.sp404_node_name.read().clone()
                                {
                                    let _ = self.command_sender.send(AudioCommand::LinkCapture {
                                        sp404_node: node_name,
                                    });
                                }
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(10, 11, 15))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(32, 36, 48)))
                                    .corner_radius(CornerRadius::same(3))
                                    .inner_margin(egui::Margin::symmetric(6, 2))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("●")
                                                    .color(RolandTheme::METER_GREEN)
                                                    .size(9.0),
                                            );
                                            ui.label(
                                                RichText::new("48kHz • 24-BIT • PipeWire Native")
                                                    .color(RolandTheme::METER_GREEN)
                                                    .size(9.5)
                                                    .monospace()
                                                    .strong(),
                                            );
                                        });
                                    });
                            } else {
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(10, 11, 15))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(32, 36, 48)))
                                    .corner_radius(CornerRadius::same(3))
                                    .inner_margin(egui::Margin::symmetric(6, 2))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("○")
                                                    .color(RolandTheme::ACCENT_AMBER)
                                                    .size(9.0),
                                            );
                                            ui.label(
                                                RichText::new("SCANNING USB")
                                                    .color(RolandTheme::ACCENT_AMBER)
                                                    .size(9.5)
                                                    .monospace(),
                                            );
                                        });
                                    });
                            }
                        });
                    });

                    ui.add_space(3.0);
                    let device_desc = self.state.device_name.read().clone();
                    let status_str = if is_connected {
                        format!("DEVICE: {device_desc} | USB Mix Out Linked")
                    } else {
                        "USB Mix Out not detected. Connect SP-404MKII via USB-C (System: Generic)"
                            .to_string()
                    };

                    ui.label(
                        RichText::new(status_str)
                            .size(9.5)
                            .color(RolandTheme::TEXT_MUTED)
                            .monospace(),
                    );
                });

                ui.add_space(8.0);

                // ====================================================================
                // RACK CARD 2: Precision Master Metering Deck
                // ====================================================================
                RolandTheme::rack_card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("MASTER INPUT LEVEL")
                                .size(9.5)
                                .color(RolandTheme::TEXT_MUTED)
                                .monospace(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                            ui.label(
                                RichText::new(peak_str)
                                    .size(9.0)
                                    .color(peak_color)
                                    .monospace(),
                            );
                        });
                    });

                    ui.add_space(4.0);
                    let meter_w = ui.available_width();
                    VuMeterWidget::new("L", cur_l, self.peak_hold_l, is_clipped_l)
                        .ui(ui, meter_w, 18.0);
                    ui.add_space(3.0);
                    VuMeterWidget::new("R", cur_r, self.peak_hold_r, is_clipped_r)
                        .ui(ui, meter_w, 18.0);
                    ui.add_space(2.0);
                    VuMeterWidget::draw_scale(ui, meter_w, 14.0);
                });

                ui.add_space(8.0);

                // ====================================================================
                // RACK CARD 3: Output Routing & Monitor Console
                // ====================================================================
                RolandTheme::rack_card_frame().show(ui, |ui| {
                    // 1. MONITOR VOLUME HEADER ROW
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("MONITOR OUTPUT")
                                .size(10.5)
                                .monospace()
                                .strong()
                                .color(RolandTheme::TEXT_PRIMARY),
                        );

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            // Quick Reset / Snap to 100% Unity Button
                            if ui
                                .small_button("0 dB Unity")
                                .on_hover_text("Reset monitor volume to 100% (0 dB Unity Gain)")
                                .clicked()
                            {
                                self.volume_percent = 100.0;
                                self.state.set_monitor_gain(1.0);
                            }

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

                            ui.label(
                                RichText::new(format!("[ {chip_text} ]"))
                                    .size(10.0)
                                    .monospace()
                                    .strong()
                                    .color(chip_color),
                            );
                        });
                    });

                    ui.add_space(3.0);

                    // 2. UNMISTAKABLE FULL-WIDTH FADER SLIDER
                    let fader_width = ui.available_width();
                    let slider = Slider::new(&mut self.volume_percent, 0.0..=150.0)
                        .trailing_fill(true)
                        .show_value(false);

                    if ui.add_sized(Vec2::new(fader_width, 22.0), slider).changed() {
                        let gain = self.volume_percent / 100.0;
                        self.state.set_monitor_gain(gain);
                    }

                    ui.add_space(6.0);

                    // 3. AUDIO OUTPUT SINK SELECTOR
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("AUDIO SINK:")
                                .size(10.0)
                                .monospace()
                                .color(RolandTheme::TEXT_MUTED),
                        );
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

                        let combo_width = (ui.available_width() - 32.0).max(100.0);
                        egui::ComboBox::from_id_salt("output_sink_selector")
                            .selected_text(RichText::new(display_text).size(10.5))
                            .width(combo_width)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        current_sink.is_none(),
                                        "System Default (Follow OS)",
                                    )
                                    .clicked()
                                {
                                    let _ = self
                                        .command_sender
                                        .send(AudioCommand::RoutePlayback { sink_name: None });
                                }
                                for sink in &available {
                                    let is_selected = current_sink.as_deref() == Some(&sink.name);
                                    if ui
                                        .selectable_label(is_selected, &sink.description)
                                        .clicked()
                                    {
                                        let _ =
                                            self.command_sender.send(AudioCommand::RoutePlayback {
                                                sink_name: Some(sink.name.clone()),
                                            });
                                    }
                                }
                            });

                        if ui
                            .small_button("↻")
                            .on_hover_text("Refresh audio output list")
                            .clicked()
                        {
                            let _ = self.command_sender.send(AudioCommand::RefreshSinks);
                        }
                    });

                    ui.add_space(6.0);

                    // 4. UNAMBIGUOUS LOCAL PC MONITOR STATUS & ACTION
                    let is_unlinked = self.state.is_local_monitor_unlinked();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("PC OUTPUT:")
                                .size(10.0)
                                .monospace()
                                .color(RolandTheme::TEXT_MUTED),
                        );

                        let (status_text, status_color, btn_label, btn_stroke_color) =
                            if is_unlinked {
                                (
                                    "● MUTED",
                                    RolandTheme::ACCENT_AMBER,
                                    "🔊 Enable Pass-Through",
                                    Color32::from_rgb(34, 197, 94),
                                )
                            } else {
                                (
                                    "● LIVE (PASS-THROUGH)",
                                    RolandTheme::METER_GREEN,
                                    "🔇 Mute Local Speakers",
                                    Color32::from_rgb(239, 68, 68),
                                )
                            };

                        ui.label(
                            RichText::new(status_text)
                                .size(10.0)
                                .monospace()
                                .strong()
                                .color(status_color),
                        );

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let btn = egui::Button::new(
                                RichText::new(btn_label)
                                    .size(10.0)
                                    .monospace()
                                    .strong()
                                    .color(RolandTheme::TEXT_PRIMARY),
                            )
                            .stroke(Stroke::new(1.0, btn_stroke_color));

                            if ui.add(btn).clicked() {
                                let new_unlinked = !is_unlinked;
                                let _ = self
                                    .command_sender
                                    .send(AudioCommand::SetLocalMonitorMute(new_unlinked));
                            }
                        });
                    });
                });
            }
            AppTab::SampleCrate => {
                crate_view::render_crate_view(ui, &mut self.crate_state, &self.preview_player);
            }
        }

        ui.add_space(4.0);

        // ====================================================================
        // BOTTOM BANNER: Context Guidance + "Made with ♥ from frStudios"
        // ====================================================================
        egui::Frame::new()
            .fill(Color32::from_rgb(12, 13, 17))
            .stroke(Stroke::new(1.0, RolandTheme::BORDER_SUBTLE))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if self.current_tab == AppTab::StreamHud {
                        ui.label(
                            RichText::new("DISCORD / OBS:")
                                .size(9.0)
                                .color(RolandTheme::ACCENT_AMBER)
                                .monospace()
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Share window to stream audio")
                                .size(9.0)
                                .color(RolandTheme::TEXT_MUTED),
                        );
                    } else {
                        ui.label(
                            RichText::new("CONFORM TARGET:")
                                .size(9.0)
                                .color(RolandTheme::ACCENT_AMBER)
                                .monospace()
                                .strong(),
                        );
                        ui.label(
                            RichText::new("48kHz • 24-bit PCM")
                                .size(9.0)
                                .color(RolandTheme::TEXT_MUTED),
                        );
                    }

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

        // --- ADAPTIVE REPAINT RATE ---
        let ctx = ui.ctx().clone();
        if self.current_tab == AppTab::SampleCrate {
            if self.preview_player.is_playing() {
                ctx.request_repaint_after(Duration::from_millis(16));
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        } else if is_connected && (cur_l > -55.0 || cur_r > -55.0) {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}
