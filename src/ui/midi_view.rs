//! SP-404MKII USB MIDI Pad & EFX Deck UI View.
//!
//! Provides a 4×4 virtual pad grid (Mode A & Mode B, Note-On-only per Roland's v5 chart),
//! QWERTY keyboard pad triggering, Bus 1–4 EFX rotary knobs (`CC #16–19`, `CC #80–83`),
//! and Pattern Program Change (`PC #0..15`) triggers.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Slider, Stroke, StrokeKind, Ui,
    Vec2,
};
use strum::IntoEnumIterator;

use crate::midi::{
    EfxBus, MidiDispatcher, MidiPadMode, MidiPortInfo, PadBank, cc, encode_cc, encode_pad_note_on,
    encode_pattern_program_change, resolve_pad_midi_target, scan_midi_ports,
};
use crate::ui::theme::RolandTheme;
use crate::ui::widgets::RotaryKnob;

/// Per-bus EFX parameter state (`CTRL 1–6`, `EFX On/Off`, `EFX Number`, and MIDI channel).
#[derive(Debug, Clone)]
pub struct BusEfxState {
    pub midi_channel_1_based: u8,
    pub efx_enabled: bool,
    pub efx_number: u8,
    pub ctrl_1: u8,
    pub ctrl_2: u8,
    pub ctrl_3: u8,
    pub ctrl_4: u8,
    pub ctrl_5: u8,
    pub ctrl_6: u8,
}

impl BusEfxState {
    pub const fn new(default_ch_1_based: u8) -> Self {
        Self {
            midi_channel_1_based: default_ch_1_based,
            efx_enabled: true,
            efx_number: 1,
            ctrl_1: 64,
            ctrl_2: 64,
            ctrl_3: 64,
            ctrl_4: 64,
            ctrl_5: 64,
            ctrl_6: 64,
        }
    }
}

pub struct MidiDeckState {
    pub dispatcher: MidiDispatcher,
    pub ports: Vec<MidiPortInfo>,
    pub selected_device_id: String,
    pub last_port_scan: Instant,
    pub pad_mode: MidiPadMode,
    pub active_bank: PadBank,
    pub velocity: u8,
    pub note_offset: i16,
    pub active_bus: EfxBus,
    pub bus_states: [BusEfxState; 4],
    pub selected_pattern_1_based: u8,
    pub pad_flash_times: [Option<Instant>; 16],
    pub last_tx_summary: String,
}

impl Default for MidiDeckState {
    fn default() -> Self {
        let ports = scan_midi_ports();
        let selected_device_id = ports
            .iter()
            .find(|p| p.is_sp404())
            .or_else(|| ports.first())
            .map(|p| p.device_id.clone())
            .unwrap_or_default();

        Self {
            dispatcher: MidiDispatcher::new(),
            ports,
            selected_device_id,
            last_port_scan: Instant::now(),
            pad_mode: MidiPadMode::ModeA,
            active_bank: PadBank::A,
            velocity: 110,
            note_offset: 0,
            active_bus: EfxBus::Bus1,
            bus_states: [
                BusEfxState::new(1),
                BusEfxState::new(2),
                BusEfxState::new(3),
                BusEfxState::new(4),
            ],
            selected_pattern_1_based: 1,
            pad_flash_times: [None; 16],
            last_tx_summary: "Ready (Note-On Mode)".to_string(),
        }
    }
}

impl MidiDeckState {
    pub fn refresh_ports(&mut self) {
        self.last_port_scan = Instant::now();
        self.ports = scan_midi_ports();
        if self.selected_device_id.is_empty()
            || !self
                .ports
                .iter()
                .any(|p| p.device_id == self.selected_device_id)
        {
            self.selected_device_id = self
                .ports
                .iter()
                .find(|p| p.is_sp404())
                .or_else(|| self.ports.first())
                .map(|p| p.device_id.clone())
                .unwrap_or_default();
        }
    }

    pub fn trigger_pad(&mut self, pad_idx: usize) {
        if pad_idx >= 16 {
            return;
        }
        self.pad_flash_times[pad_idx] = Some(Instant::now());
        let packet = encode_pad_note_on(
            self.pad_mode,
            self.active_bank,
            pad_idx,
            self.velocity,
            self.note_offset,
        );
        let (ch0, note) =
            resolve_pad_midi_target(self.pad_mode, self.active_bank, pad_idx, self.note_offset);
        self.last_tx_summary = format!(
            "PAD {:02} ({}) → Ch {} Note {} Vel {}",
            pad_idx + 1,
            self.active_bank.short_label(),
            ch0 + 1,
            note,
            self.velocity
        );
        self.dispatcher.send_raw(&self.selected_device_id, &packet);
    }

    pub fn send_bus_cc(&mut self, cc_num: u8, value: u8, label: &str) {
        let bus_idx = self.active_bus as usize;
        let ch0 = self.bus_states[bus_idx]
            .midi_channel_1_based
            .saturating_sub(1)
            .min(15);
        let packet = encode_cc(ch0, cc_num, value);
        self.last_tx_summary = format!(
            "{} {} (CC#{}) → Ch {} Val {}",
            self.active_bus,
            label,
            cc_num,
            ch0 + 1,
            value
        );
        self.dispatcher.send_raw(&self.selected_device_id, &packet);
    }

    pub fn trigger_pattern(&mut self, pattern_1_to_16: u8) {
        let pat0 = pattern_1_to_16.saturating_sub(1).min(15);
        let (ch0, _) = resolve_pad_midi_target(self.pad_mode, self.active_bank, 0, 0);
        let packet = encode_pattern_program_change(ch0, pat0);
        self.last_tx_summary = format!(
            "PATTERN {:02} → Ch {} PC#{}",
            pattern_1_to_16,
            ch0 + 1,
            pat0
        );
        self.dispatcher.send_raw(&self.selected_device_id, &packet);
    }
}

const PAD_HOTKEYS: [(Key, &str); 16] = [
    (Key::Num1, "1"),
    (Key::Num2, "2"),
    (Key::Num3, "3"),
    (Key::Num4, "4"),
    (Key::Q, "Q"),
    (Key::W, "W"),
    (Key::E, "E"),
    (Key::R, "R"),
    (Key::A, "A"),
    (Key::S, "S"),
    (Key::D, "D"),
    (Key::F, "F"),
    (Key::Z, "Z"),
    (Key::X, "X"),
    (Key::C, "C"),
    (Key::V, "V"),
];

pub fn show_midi_deck_ui(ui: &mut Ui, state: &mut MidiDeckState) {
    // Periodic ALSA MIDI port hotplug scan
    if state.last_port_scan.elapsed() > Duration::from_secs(3) {
        state.refresh_ports();
    }

    // Handle QWERTY hotkeys (1-4, Q-R, A-F, Z-V) when no text input is focused
    if !ui.ctx().egui_wants_keyboard_input() {
        for (pad_idx, (key, _)) in PAD_HOTKEYS.iter().enumerate() {
            if ui.input(|i| i.key_pressed(*key)) {
                state.trigger_pad(pad_idx);
            }
        }
    }

    // ====================================================================
    // RACK CARD 1: USB MIDI Endpoint & Mode Configuration
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let has_port = !state.selected_device_id.is_empty();
            ui.label(
                RichText::new(if has_port { "● MIDI" } else { "○ MIDI" })
                    .color(if has_port {
                        RolandTheme::METER_GREEN
                    } else {
                        RolandTheme::ACCENT_AMBER
                    })
                    .size(10.0)
                    .monospace()
                    .strong(),
            );

            let selected_label = state
                .ports
                .iter()
                .find(|p| p.device_id == state.selected_device_id)
                .map(|p| format!("{} ({})", p.name, p.device_id))
                .unwrap_or_else(|| "No ALSA MIDI Port Connected".to_string());

            egui::ComboBox::from_id_salt("midi_port_selector")
                .selected_text(RichText::new(selected_label).size(9.5))
                .width(195.0)
                .show_ui(ui, |ui| {
                    for p in &state.ports {
                        let label = format!("{} ({})", p.name, p.device_id);
                        ui.selectable_value(
                            &mut state.selected_device_id,
                            p.device_id.clone(),
                            label,
                        );
                    }
                });

            if ui
                .small_button("⟳")
                .on_hover_text("Rescan ALSA hardware MIDI endpoints (amidi -l)")
                .clicked()
            {
                state.refresh_ports();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mode_short = match state.pad_mode {
                    MidiPadMode::ModeA => "MODE A (10-CH)",
                    MidiPadMode::ModeB => "MODE B (2-CH)",
                };
                if ui
                    .button(
                        RichText::new(mode_short)
                            .size(9.5)
                            .color(RolandTheme::ACCENT_ORANGE)
                            .monospace()
                            .strong(),
                    )
                    .on_hover_text(
                        "Toggle between SP-404MKII MIDI Mode A (Ch 1–10 per bank, notes 36–51) and Mode B (Ch 1: A–E, Ch 2: F–J, notes 12–91)",
                    )
                    .clicked()
                {
                    state.pad_mode = match state.pad_mode {
                        MidiPadMode::ModeA => MidiPadMode::ModeB,
                        MidiPadMode::ModeB => MidiPadMode::ModeA,
                    };
                }
            });
        });

        ui.add_space(4.0);

        // Bank selector strip (A..J) + Velocity & Offset
        ui.horizontal(|ui| {
            for bank in PadBank::iter() {
                let is_sel = state.active_bank == bank;
                let btn = egui::Button::new(
                    RichText::new(bank.short_label())
                        .size(9.5)
                        .monospace()
                        .strong()
                        .color(if is_sel {
                            RolandTheme::TEXT_ON_ACCENT
                        } else {
                            RolandTheme::TEXT_SECONDARY
                        }),
                )
                .fill(if is_sel {
                    RolandTheme::ACCENT_ORANGE
                } else {
                    Color32::from_rgb(14, 15, 20)
                })
                .min_size(Vec2::new(22.0, 18.0));

                if ui.add(btn).clicked() {
                    state.active_bank = bank;
                }
            }

            ui.separator();
            ui.label(
                RichText::new("VEL")
                    .size(8.5)
                    .color(RolandTheme::TEXT_MUTED)
                    .monospace(),
            );
            ui.add(
                Slider::new(&mut state.velocity, 1..=127)
                    .show_value(true)
                    .trailing_fill(true),
            );
        });
    });

    ui.add_space(6.0);

    // ====================================================================
    // RACK CARD 2: 4×4 Hardware Pad Matrix (Pads 1–16, Note-On Only)
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let avail_w = ui.available_width();
        let spacing = 6.0;
        let pad_w = ((avail_w - spacing * 3.0) / 4.0).max(60.0);
        let pad_h = 40.0;
        let now = Instant::now();

        for row in 0..4 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                for col in 0..4 {
                    let pad_idx = row * 4 + col;
                    let (_, key_label) = PAD_HOTKEYS[pad_idx];
                    let (ch0, note) = resolve_pad_midi_target(
                        state.pad_mode,
                        state.active_bank,
                        pad_idx,
                        state.note_offset,
                    );

                    let is_flashing = state.pad_flash_times[pad_idx]
                        .is_some_and(|t| now.duration_since(t) < Duration::from_millis(140));

                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(pad_w, pad_h), Sense::click());
                    if resp.clicked() {
                        state.trigger_pad(pad_idx);
                    }

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter_at(rect);
                        let fill = if is_flashing {
                            RolandTheme::ACCENT_ORANGE
                        } else if resp.hovered() {
                            Color32::from_rgb(26, 29, 38)
                        } else {
                            Color32::from_rgb(13, 14, 19)
                        };
                        let border = if is_flashing {
                            RolandTheme::ACCENT_AMBER
                        } else if resp.hovered() {
                            RolandTheme::ACCENT_ORANGE
                        } else {
                            RolandTheme::BORDER_COLOR
                        };
                        let fg_main = if is_flashing {
                            RolandTheme::TEXT_ON_ACCENT
                        } else {
                            RolandTheme::TEXT_PRIMARY
                        };
                        let fg_sub = if is_flashing {
                            RolandTheme::TEXT_ON_ACCENT
                        } else {
                            RolandTheme::TEXT_MUTED
                        };

                        painter.rect_filled(rect, CornerRadius::same(5), fill);
                        painter.rect_stroke(
                            rect,
                            CornerRadius::same(5),
                            Stroke::new(1.0, border),
                            StrokeKind::Inside,
                        );

                        // Top-left: Pad Number
                        painter.text(
                            Pos2::new(rect.left() + 6.0, rect.top() + 10.0),
                            egui::Align2::LEFT_CENTER,
                            format!("PAD {:02}", pad_idx + 1),
                            egui::FontId::monospace(9.5),
                            fg_main,
                        );

                        // Top-right: Hotkey Badge
                        let badge_rect = Rect::from_min_size(
                            Pos2::new(rect.right() - 19.0, rect.top() + 4.0),
                            Vec2::new(14.0, 12.0),
                        );
                        painter.rect_filled(
                            badge_rect,
                            CornerRadius::same(2),
                            if is_flashing {
                                Color32::from_rgba_unmultiplied(0, 0, 0, 45)
                            } else {
                                RolandTheme::WELL_BG
                            },
                        );
                        painter.text(
                            badge_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            key_label,
                            egui::FontId::monospace(8.0),
                            if is_flashing {
                                Color32::WHITE
                            } else {
                                RolandTheme::ACCENT_AMBER
                            },
                        );

                        // Bottom: Resolved MIDI Channel & Note
                        painter.text(
                            Pos2::new(rect.left() + 6.0, rect.bottom() - 8.5),
                            egui::Align2::LEFT_CENTER,
                            format!("Ch{} • N{}", ch0 + 1, note),
                            egui::FontId::monospace(8.0),
                            fg_sub,
                        );
                    }
                }
            });
            if row < 3 {
                ui.add_space(5.0);
            }
        }
    });

    ui.add_space(6.0);

    // ====================================================================
    // RACK CARD 3: Bus 1–4 EFX Rotary Knobs (CC #16–19, #83) & Pattern Trigger
    // ====================================================================
    RolandTheme::rack_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        let bus_idx = state.active_bus as usize;
        ui.horizontal(|ui| {
            for bus in EfxBus::iter() {
                let is_sel = state.active_bus == bus;
                if RolandTheme::tab_button(ui, is_sel, bus.into()).clicked() {
                    state.active_bus = bus;
                }
            }

            ui.separator();

            let efx_on = state.bus_states[bus_idx].efx_enabled;
            let sw_btn = egui::Button::new(
                RichText::new(if efx_on {
                    "EFX ON (CC19)"
                } else {
                    "EFX BYPASS"
                })
                .size(9.5)
                .color(if efx_on {
                    RolandTheme::TEXT_ON_ACCENT
                } else {
                    RolandTheme::TEXT_SECONDARY
                })
                .strong(),
            )
            .fill(if efx_on {
                RolandTheme::METER_GREEN
            } else {
                Color32::from_rgb(20, 22, 28)
            });

            if ui.add(sw_btn).clicked() {
                let next = !efx_on;
                state.bus_states[bus_idx].efx_enabled = next;
                state.send_bus_cc(cc::EFX_SWITCH, if next { 127 } else { 0 }, "EFX SW");
            }
        });

        ui.add_space(5.0);

        ui.horizontal(|ui| {
            let mut c1 = state.bus_states[bus_idx].ctrl_1;
            if RotaryKnob::new("CTRL 1", "CC16", &mut c1).show(ui) {
                state.bus_states[bus_idx].ctrl_1 = c1;
                state.send_bus_cc(cc::CTRL_1, c1, "CTRL 1");
            }

            let mut c2 = state.bus_states[bus_idx].ctrl_2;
            if RotaryKnob::new("CTRL 2", "CC17", &mut c2).show(ui) {
                state.bus_states[bus_idx].ctrl_2 = c2;
                state.send_bus_cc(cc::CTRL_2, c2, "CTRL 2");
            }

            let mut c3 = state.bus_states[bus_idx].ctrl_3;
            if RotaryKnob::new("CTRL 3", "CC18", &mut c3).show(ui) {
                state.bus_states[bus_idx].ctrl_3 = c3;
                state.send_bus_cc(cc::CTRL_3, c3, "CTRL 3");
            }

            ui.vertical(|ui| {
                ui.label(
                    RichText::new("EFX TYPE (CC83)")
                        .size(8.5)
                        .color(RolandTheme::TEXT_MUTED)
                        .monospace(),
                );
                let mut efx_num = state.bus_states[bus_idx].efx_number;
                if ui
                    .add(
                        Slider::new(&mut efx_num, 0..=40)
                            .show_value(true)
                            .trailing_fill(true),
                    )
                    .changed()
                {
                    state.bus_states[bus_idx].efx_number = efx_num;
                    state.send_bus_cc(cc::EFX_NUMBER, efx_num, "EFX TYPE");
                }

                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("PTN (PC):")
                            .size(8.5)
                            .color(RolandTheme::TEXT_MUTED)
                            .monospace(),
                    );
                    egui::ComboBox::from_id_salt("midi_pattern_pc")
                        .selected_text(
                            RichText::new(format!("PTN {:02}", state.selected_pattern_1_based))
                                .size(9.5),
                        )
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            for p in 1..=16u8 {
                                ui.selectable_value(
                                    &mut state.selected_pattern_1_based,
                                    p,
                                    format!("PTN {p:02} (PC#{})", p - 1),
                                );
                            }
                        });
                    if ui
                        .button(RichText::new("▶ Fire").size(9.5).strong())
                        .on_hover_text(
                            "Send MIDI Program Change (PC#0..15) to trigger Pattern 1..16",
                        )
                        .clicked()
                    {
                        let p = state.selected_pattern_1_based;
                        state.trigger_pattern(p);
                    }
                });
            });
        });

        ui.add_space(3.0);
        ui.label(
            RichText::new(&state.last_tx_summary)
                .size(9.0)
                .color(RolandTheme::TEXT_MUTED)
                .monospace(),
        );
    });
}
