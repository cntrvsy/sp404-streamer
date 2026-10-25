use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, Slider, Vec2};

use crate::audio::port_manager::PortManager;
use crate::audio::state::SharedAudioState;
use crate::ui::theme::RolandTheme;
use crate::ui::widgets::vu_meter::VuMeterWidget;

pub struct Sp404StreamerApp {
    state: Arc<SharedAudioState>,

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
    pub fn new(state: Arc<SharedAudioState>) -> Self {
        let initial_gain = state.get_monitor_gain();
        Self {
            state,
            peak_hold_l: -60.0,
            peak_hold_r: -60.0,
            last_peak_update: Instant::now(),
            clip_l_time: None,
            clip_r_time: None,
            volume_percent: initial_gain * 100.0,
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
        // Read current atomic levels
        let is_connected = self.state.is_connected();
        let cur_l = self.state.get_peak_l();
        let cur_r = self.state.get_peak_r();

        // Check and handle clipping
        let now = Instant::now();
        if self.state.is_clipped_l.swap(false, Ordering::Relaxed) {
            self.clip_l_time = Some(now);
        }
        if self.state.is_clipped_r.swap(false, Ordering::Relaxed) {
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
        ui.add_space(8.0);

        // --- HEADER: Hardware Title & Status Dot ---
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("SP-404MKII")
                    .color(RolandTheme::ACCENT_ORANGE)
                    .strong()
                    .size(15.0),
            );
            ui.label(
                RichText::new("STREAMER")
                    .color(Color32::from_white_alpha(180))
                    .size(13.0),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_connected {
                    ui.label(
                        RichText::new("● CONNECTED")
                            .color(RolandTheme::METER_GREEN)
                            .size(11.0)
                            .strong(),
                    );
                } else {
                    ui.label(
                        RichText::new("○ WAITING FOR USB")
                            .color(RolandTheme::ACCENT_AMBER)
                            .size(11.0),
                    );
                }
            });
        });

        // Subtitle / Device status string
        let device_desc = self.state.device_name.read().clone();
        ui.add_space(2.0);
        if is_connected {
            ui.label(
                RichText::new(format!("Input: {} (48kHz Stereo)", device_desc))
                    .size(10.5)
                    .color(RolandTheme::TEXT_MUTED),
            );
        } else {
            ui.label(
                RichText::new("Tip: Ensure SP-404MKII Utility > System > USB Driver is GENERIC")
                    .size(10.0)
                    .color(Color32::from_rgb(180, 140, 80)),
            );
        }

        ui.add_space(10.0);

        // --- STEREO METERS ---
        let meter_width = ui.available_width();
        let meter_height = 20.0;

        VuMeterWidget::new("L", cur_l, self.peak_hold_l, is_clipped_l).ui(
            ui,
            meter_width,
            meter_height,
        );
        ui.add_space(4.0);
        VuMeterWidget::new("R", cur_r, self.peak_hold_r, is_clipped_r).ui(
            ui,
            meter_width,
            meter_height,
        );

        // dB scale markers
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("-60").size(9.0).color(RolandTheme::TEXT_MUTED));
            ui.add_space(meter_width * 0.18);
            ui.label(RichText::new("-24").size(9.0).color(RolandTheme::TEXT_MUTED));
            ui.add_space(meter_width * 0.18);
            ui.label(RichText::new("-12").size(9.0).color(RolandTheme::TEXT_MUTED));
            ui.add_space(meter_width * 0.18);
            ui.label(RichText::new("-6").size(9.0).color(RolandTheme::TEXT_MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let clip_text = if is_clipped_l || is_clipped_r {
                    RichText::new("CLIP").color(RolandTheme::METER_RED).strong().size(9.5)
                } else {
                    RichText::new("0 dB").size(9.0).color(RolandTheme::TEXT_MUTED)
                };
                ui.label(clip_text);
            });
        });

        ui.add_space(10.0);

        // --- CONTROLS: Volume & Local Monitor Mute ---
        ui.horizontal(|ui| {
            ui.label(RichText::new("Monitor Vol:").size(12.0));
            let slider = Slider::new(&mut self.volume_percent, 0.0..=150.0)
                .show_value(true)
                .suffix("%");

            if ui.add(slider).changed() {
                let gain = self.volume_percent / 100.0;
                self.state.set_monitor_gain(gain);
            }
        });

        ui.add_space(6.0);

        // Local Monitor Mute toggle button
        let is_unlinked = self.state.is_local_monitor_unlinked();
        let button_text = if is_unlinked {
            RichText::new("🔇 Local PC Monitor: MUTED (Discord Streaming)")
                .color(RolandTheme::ACCENT_AMBER)
                .strong()
        } else {
            RichText::new("🔊 Local PC Monitor: ACTIVE")
                .color(RolandTheme::TEXT_PRIMARY)
        };

        let btn = egui::Button::new(button_text).min_size(Vec2::new(ui.available_width(), 26.0));
        if ui.add(btn).clicked() {
            let new_unlinked = !is_unlinked;
            self.state.set_local_monitor_unlinked(new_unlinked);

            std::thread::spawn(move || {
                if new_unlinked {
                    PortManager::unlink_local_monitor("sp404_streamer_playback");
                } else {
                    PortManager::relink_local_monitor("sp404_streamer_playback");
                }
            });
        }

        ui.add_space(10.0);

        // --- FOOTER: Discord Helper Badge ---
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("💡 In Discord: Share Screen → Window → 'SP-404MKII Streamer'")
                    .size(10.5)
                    .color(Color32::from_white_alpha(170)),
            );
        });

        // --- ADAPTIVE REPAINT RATE ---
        // 60 FPS when active audio is playing or device is connected, drops to ~8 FPS when silent
        let ctx = ui.ctx().clone();
        if is_connected && (cur_l > -55.0 || cur_r > -55.0) {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}
