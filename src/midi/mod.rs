//! SP-404MKII USB MIDI Controller & Protocol Engine (Verified against Roland v5.00+ MIDI Chart).
//!
//! Implements:
//! - **MIDI Mode A** (Channels 1–10 for Banks A–J, default notes `36–51`)
//! - **MIDI Mode B** (Channel 1 for Banks A–E, Channel 2 for Banks F–J, default notes `12–91`)
//! - **Note On only** pad triggers (`Note Off` is not recognized per Roland's chart)
//! - **Bus EFX Control Changes**: `CC #16–18` (CTRL 1–3), `CC #19` (EFX On/Off),
//!   `CC #80–82` (CTRL 4–6), `CC #83` (EFX Type Number)
//! - **Pattern Program Change**: `PC #0..15` (Patterns 1–16)
//! - Non-blocking background ALSA MIDI port discovery and packet transmission.

use std::process::Command;
use std::sync::mpsc;
use std::thread;

use strum::{Display, EnumIter, IntoStaticStr};
use tracing::{debug, warn};

/// Pad bank (`A` through `J`) on the SP-404MKII.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum PadBank {
    #[strum(to_string = "BANK A")]
    A = 0,
    #[strum(to_string = "BANK B")]
    B = 1,
    #[strum(to_string = "BANK C")]
    C = 2,
    #[strum(to_string = "BANK D")]
    D = 3,
    #[strum(to_string = "BANK E")]
    E = 4,
    #[strum(to_string = "BANK F")]
    F = 5,
    #[strum(to_string = "BANK G")]
    G = 6,
    #[strum(to_string = "BANK H")]
    H = 7,
    #[strum(to_string = "BANK I")]
    I = 8,
    #[strum(to_string = "BANK J")]
    J = 9,
}

impl PadBank {
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::F => "F",
            Self::G => "G",
            Self::H => "H",
            Self::I => "I",
            Self::J => "J",
        }
    }
}

/// SP-404MKII System MIDI Pad Mode (`UTILITY -> SYSTEM -> MIDI`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum MidiPadMode {
    #[strum(to_string = "Mode A (Ch 1–10 per Bank, Notes 36–51)")]
    ModeA,
    #[strum(to_string = "Mode B (Ch 1: A–E, Ch 2: F–J, Notes 12–91)")]
    ModeB,
}

/// SP-404MKII Effect Bus (`BUS 1` through `BUS 4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum EfxBus {
    #[strum(to_string = "BUS 1")]
    Bus1 = 0,
    #[strum(to_string = "BUS 2")]
    Bus2 = 1,
    #[strum(to_string = "BUS 3")]
    Bus3 = 2,
    #[strum(to_string = "BUS 4")]
    Bus4 = 3,
}

impl EfxBus {
    /// Default 0-based MIDI channel (`0..=3`, i.e., MIDI Channels 1–4) for this effect bus.
    pub const fn default_channel_0_based(self) -> u8 {
        self as u8
    }
}

/// Official Roland v5.00+ MIDI Control Change numbers for SP-404MKII Effect Buses.
pub mod cc {
    pub const CTRL_1: u8 = 16;
    pub const CTRL_2: u8 = 17;
    pub const CTRL_3: u8 = 18;
    pub const EFX_SWITCH: u8 = 19;
    pub const CTRL_4: u8 = 80;
    pub const CTRL_5: u8 = 81;
    pub const CTRL_6: u8 = 82;
    pub const EFX_NUMBER: u8 = 83;
}

/// Discovered ALSA hardware MIDI port (from `amidi -l`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiPortInfo {
    /// ALSA rawmidi device identifier (e.g. `hw:2,0,0`)
    pub device_id: String,
    /// Friendly hardware name (e.g. `SP-404MKII MIDI 1`)
    pub name: String,
}

impl MidiPortInfo {
    pub fn is_sp404(&self) -> bool {
        let up = self.name.to_uppercase();
        up.contains("404") || up.contains("ROLAND")
    }
}

/// Pure function: Computes `(channel_0_based, midi_note)` for a given `MidiPadMode`,
/// `PadBank` (`A..J`), `pad_index` (`0..16` for Pads 1–16), and user `note_offset`.
pub fn resolve_pad_midi_target(
    mode: MidiPadMode,
    bank: PadBank,
    pad_index: usize,
    note_offset: i16,
) -> (u8, u8) {
    let bank_idx = bank.index();
    let pad_clamped = pad_index.min(15) as i16;

    match mode {
        MidiPadMode::ModeA => {
            let channel = (bank_idx as u8).min(15);
            let note = (36 + note_offset + pad_clamped).clamp(0, 127) as u8;
            (channel, note)
        }
        MidiPadMode::ModeB => {
            let channel = if bank_idx < 5 { 0u8 } else { 1u8 };
            let rel_bank = (bank_idx % 5) as i16;
            let note = (12 + note_offset + rel_bank * 16 + pad_clamped).clamp(0, 127) as u8;
            (channel, note)
        }
    }
}

/// Encodes a 3-byte MIDI `Note On` packet (`[0x90 | ch, note, velocity]`).
/// Note that Roland's MIDI Implementation Chart specifies `Note Off` as unrecognized (`x`),
/// so pad triggers always emit `Note On` with non-zero velocity (`1..=127`).
#[inline]
pub fn encode_pad_note_on(
    mode: MidiPadMode,
    bank: PadBank,
    pad_index: usize,
    velocity: u8,
    note_offset: i16,
) -> [u8; 3] {
    let (ch, note) = resolve_pad_midi_target(mode, bank, pad_index, note_offset);
    let vel = velocity.clamp(1, 127);
    [0x90 | (ch & 0x0F), note & 0x7F, vel]
}

/// Encodes a 3-byte MIDI `Control Change` packet (`[0xB0 | ch, cc_num, value]`).
#[inline]
pub const fn encode_cc(channel_0_based: u8, cc_number: u8, value: u8) -> [u8; 3] {
    [
        0xB0 | (channel_0_based & 0x0F),
        cc_number & 0x7F,
        value & 0x7F,
    ]
}

/// Encodes a 2-byte MIDI `Program Change` packet (`[0xC0 | ch, program_num]`) for triggering Patterns 1–16.
#[inline]
pub const fn encode_pattern_program_change(channel_0_based: u8, pattern_index: u8) -> [u8; 2] {
    [0xC0 | (channel_0_based & 0x0F), pattern_index & 0x7F]
}

/// Parses `amidi -l` stdout into a list of available ALSA hardware MIDI ports.
pub fn parse_amidi_list(stdout: &str) -> Vec<MidiPortInfo> {
    let mut ports = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Dir") {
            continue;
        }
        // Typical line: "IO  hw:2,0,0  SP-404MKII MIDI 1"
        let mut parts = trimmed.split_whitespace();
        let Some(_dir) = parts.next() else {
            continue;
        };
        let Some(dev_id) = parts.next() else {
            continue;
        };
        if !dev_id.starts_with("hw:") && !dev_id.starts_with("virtual") {
            continue;
        }
        let name: Vec<&str> = parts.collect();
        let friendly = if name.is_empty() {
            dev_id.to_string()
        } else {
            name.join(" ")
        };
        ports.push(MidiPortInfo {
            device_id: dev_id.to_string(),
            name: friendly,
        });
    }
    ports
}

/// Scans the Linux system for ALSA MIDI ports via `amidi -l`.
pub fn scan_midi_ports() -> Vec<MidiPortInfo> {
    let Ok(output) = Command::new("amidi").arg("-l").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_amidi_list(&stdout)
}

/// Commands dispatched from the `egui` MIDI UI to the background MIDI sender thread.
#[derive(Debug, Clone)]
pub enum MidiCommand {
    SendBytes { device_id: String, bytes: Vec<u8> },
}

/// Background worker handle for non-blocking MIDI transmission.
pub struct MidiDispatcher {
    tx: mpsc::Sender<MidiCommand>,
}

impl Default for MidiDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiDispatcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<MidiCommand>();
        thread::Builder::new()
            .name("sp404-midi-tx".to_string())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        MidiCommand::SendBytes { device_id, bytes } => {
                            let hex: Vec<String> =
                                bytes.iter().map(|b| format!("{b:02X}")).collect();
                            let hex_str = hex.join(" ");
                            debug!("MIDI TX [{}]: {}", device_id, hex_str);
                            if let Err(e) = Command::new("amidi")
                                .args(["-p", &device_id, "-S", &hex_str])
                                .output()
                            {
                                warn!("Failed to send MIDI via amidi to {}: {}", device_id, e);
                            }
                        }
                    }
                }
            })
            .expect("Failed to spawn MIDI dispatcher thread");
        Self { tx }
    }

    pub fn send_raw(&self, device_id: &str, bytes: &[u8]) {
        if device_id.is_empty() || bytes.is_empty() {
            return;
        }
        let _ = self.tx.send(MidiCommand::SendBytes {
            device_id: device_id.to_string(),
            bytes: bytes.to_vec(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_a_pad_mapping() {
        // Mode A: Bank A = Ch 0 (MIDI Ch 1), Pad 1 = Note 36, Pad 16 = Note 51
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeA, PadBank::A, 0, 0),
            (0, 36)
        );
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeA, PadBank::A, 15, 0),
            (0, 51)
        );
        // Mode A: Bank J = Ch 9 (MIDI Ch 10), Pad 1 = Note 36
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeA, PadBank::J, 0, 0),
            (9, 36)
        );
    }

    #[test]
    fn test_mode_b_pad_mapping() {
        // Mode B: Banks A-E on Ch 0 (MIDI Ch 1), notes 12..=91
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeB, PadBank::A, 0, 0),
            (0, 12)
        );
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeB, PadBank::E, 15, 0),
            (0, 91)
        );
        // Mode B: Banks F-J on Ch 1 (MIDI Ch 2), notes 12..=91
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeB, PadBank::F, 0, 0),
            (1, 12)
        );
        assert_eq!(
            resolve_pad_midi_target(MidiPadMode::ModeB, PadBank::J, 15, 0),
            (1, 91)
        );
    }

    #[test]
    fn test_cc_and_program_change_encoding() {
        // CC #16 (CTRL 1), #17 (CTRL 2), #18 (CTRL 3), #19 (EFX SW), #80-83 (CTRL 4-6 & EFX Type)
        assert_eq!(encode_cc(0, cc::CTRL_1, 64), [0xB0, 16, 64]);
        assert_eq!(encode_cc(0, cc::CTRL_2, 100), [0xB0, 17, 100]);
        assert_eq!(encode_cc(0, cc::CTRL_3, 127), [0xB0, 18, 127]);
        assert_eq!(encode_cc(0, cc::EFX_SWITCH, 127), [0xB0, 19, 127]);
        assert_eq!(encode_cc(1, cc::CTRL_4, 50), [0xB1, 80, 50]);
        assert_eq!(encode_cc(1, cc::EFX_NUMBER, 12), [0xB1, 83, 12]);

        // Program Change: PC#0 = Pattern 1
        assert_eq!(encode_pattern_program_change(0, 0), [0xC0, 0]);
        assert_eq!(encode_pattern_program_change(0, 15), [0xC0, 15]);
    }

    #[test]
    fn test_parse_amidi_list() {
        let sample = "\
Dir Device    Name
IO  hw:1,0,0  Scarlett 2i2 USB MIDI 1
IO  hw:2,0,0  SP-404MKII MIDI 1
";
        let ports = parse_amidi_list(sample);
        assert_eq!(ports.len(), 2);
        assert!(!ports[0].is_sp404());
        assert!(ports[1].is_sp404());
        assert_eq!(ports[1].device_id, "hw:2,0,0");
        assert_eq!(ports[1].name, "SP-404MKII MIDI 1");
    }
}
