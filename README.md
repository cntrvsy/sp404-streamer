# SP-404MKII Streamer & Sample Crate

A lightweight Linux audio bridge and sample workstation for the Roland SP-404MKII. Enables native 48 kHz stereo streaming directly into Discord, Vesktop, and OBS via application screen share, paired with an integrated sample conforming and SD card management deck.

![SP-404MKII Streamer Preview](assets/sp404-streamer.png)

---

## Overview

On Linux, Discord and Vesktop capture audio exclusively from application windows during screen shares, ignoring USB hardware inputs. Routing sampler outputs into a microphone channel forces the audio through aggressive noise suppression, echo cancellation, and mono compression.

**SP-404MKII Streamer** bridges your sampler directly to an application playback stream in native PipeWire without manual patchbay configuration:
- **Zero-Latency Monitoring:** Full 48 kHz stereo audio routed through a virtual application node recognized by Discord screen capture.
- **Hardware-Aware Sample Management:** Batch auditions, trims, normalizes, reorders, conforms, and syncs kits directly to your SP-404MKII SD card.
- **Resource Efficient:** Built in Rust using `egui` and native PipeWire C FFI. Runs with minimal CPU utilization and throttles repaint rates when idle.

---

## Features

### Stream & Monitor Deck
- **Stereo VU Meters:** Dual 32-segment LED peak meters with peak-hold readouts, decibel scale, and latching overload clip indicators.
- **Output Device Routing:** Switch monitor playback between headphones, studio monitors, USB DACs, or the system default sink.
- **Monitor Volume & Mute:** Precision gain fader with unity reset (`↺ 100% / 0 dB`), numeric readout, and a one-click mute button for zero-latency monitoring from the SP-404MKII headphone jack.
- **Low Latency:** Native PipeWire buffer allocation (~5.3 ms) with automatic jitter compensation.
- **Automatic Hotplug:** Instantly connects and restores audio routing whenever the SP-404MKII is connected or reconnected over USB.

### Sample Crate & SD Sync
- **Multi-File & Folder Ingestion:** Native file dialogs with batch multi-selection and recursive directory scanning with kit auto-naming.
- **Instant Auditioning:** Spacebar-triggered playback routed straight through the PipeWire monitor engine with immediate stop cutoff.
- **Waveform Overview & Auto-Chop Slicer:** 512-bin transient display with duration, peak dBFS metrics, live playhead tracking, interactive scrubbing, and transient/grid auto-chopping (`4/8/12/16` slices exported as `01_..16_` WAVs).
- **Kit Arrangement:** Live search filtering and pad ordering (`01–16`) with manual controls or keyboard shortcuts.
- **Hardware Conforming Engine:** Resamples audio to 48,000 Hz 16-bit linear PCM WAV with optional **TPDF dithering**, **leading silence trimming**, and **de-click micro-fades**, matching the SP-404MKII's native SD card import specification (preventing "File Not Supported" hardware rejection). Preserves mono files as single-channel mono to conserve SP-404MKII polyphony (1 voice instead of 2).
- **Display & FAT32 Sanitization:** Strips illegal FAT32 characters and formats filenames (default: 16 characters) for clean, readable rendering on the SP-404MKII OLED display.
- **SD Hotplug, `.tar.zst` Snapshots & Safe Ejection:** Discovers SD cards via `udisks2`, creates one-click compressed `.tar.zst` project backups in `~/Music/SP404-Backups/`, checks hardware write-lock switches, verifies available disk capacity, executes kernel cache flushes (`sync_all()`), and unmounts safely.

### OLED Screen Studio (`128 × 64` 1-Bit BMP)
- **GIF & Image Ingestion:** Load any animated `.gif`, `.png`, `.jpg`, `.webp`, or `.bmp` via native desktop portal dialogs, drag-and-drop, or Linux clipboard paste (`arboard`).
- **Built-In Procedural Presets:** Test custom animations immediately with built-in *Cassette Tape Reels*, *Harmonic Oscilloscope*, and *SP-404 Beat Spectrum* animations.
- **1-Bit Dithering & OLED Simulator:** Real-time Floyd-Steinberg, Bayer 4×4 retro grid, and hard threshold modes previewed on a dot-matrix `128 × 64` OLED simulator.
- **One-Click Project Sync:** Writes exact 1,086-byte Windows v3 `BITMAPINFOHEADER` 1-bit monochrome BMPs (`startup_1..2.bmp` and `screen_saver_1..16.bmp`) into `EXPORT/PROJECT/PROJECT_XX/PICTURE/` with an inline hardware round-trip checklist.

### USB MIDI Pad & EFX Deck (Verified Roland v5.00+ Spec)
- **4×4 Virtual Pad Matrix:** Supports **MIDI Mode A** (Channels 1–10 per Bank A–J, notes `36–51`) and **MIDI Mode B** (Channel 1: Banks A–E, Channel 2: Banks F–J, notes `12–91`), firing strictly `Note On` messages via mouse or QWERTY hotkeys (`1-4 / Q-R / A-F / Z-V`).
- **Bus 1–4 EFX Macro Knobs:** Studio rotary encoders mapped to `CC #16–18` (`CTRL 1–3`), `CC #19` (`EFX On/Off`), and `CC #83` (`EFX Type`), plus `Program Change` (`PC #0..15`) pattern triggers.

---

## Controls & Shortcuts

### Sample Crate
| Key / Gesture | Context | Action |
| :--- | :--- | :--- |
| **`Space`** | Crate Table | Play / Pause sample audition |
| **`↑` / `↓`** | Crate Table | Select previous / next sample |
| **`Ctrl + ↑` / `Ctrl + ↓`** | Crate Table | Reorder selected sample pad position (`01–16`) |
| **`Delete` / `Backspace`** | Crate Table | Remove selected sample from staged crate |
| **`Escape`** | Crate Search | Clear active search filter |
| **`↕` Left-Click** | Pad Order Column | Move pad up |
| **`↕` Right-Click** | Pad Order Column | Move pad down |
| **Click / Drag** | Waveform Strip | Scrub playhead position |

### Monitor Deck
| Control | Action |
| :--- | :--- |
| **`● CLIP`** | Resets latched audio overload indicator |
| **`↺ 100% (0 dB)`** | Resets monitor gain fader to unity gain |
| **`Mute`** | Toggles local monitor audio output |

---

## Supported Audio Formats

Decoded natively using pure-Rust decoders (Symphonia) without external library dependencies:
- **WAV** (Linear PCM 16/24/32-bit, IEEE Float)
- **AIFF** / **AIFC**
- **FLAC** (Lossless)
- **MP3** (MPEG-1/2 Audio Layer III)
- **OGG Vorbis**
- **OPUS**

---

## SP-404MKII Conforming Specifications

Exported samples are conformed to Roland SP-404MKII hardware and workflow conventions:
- **48,000 Hz Linear PCM:** Matches the SP-404MKII's native internal sample rate, eliminating hardware resampling overhead during import.
- **Polyphony Preservation:** Preserves mono sources as single-channel mono files, reducing voice consumption against the hardware's 32-voice limit (1 voice for mono vs. 2 voices for stereo).
- **FAT32 Sanitization & OLED Sizing:** Strips FAT32-illegal characters and optionally shortens names for clean, readable display on the 128×64 OLED screen without long marquee scrolls.
- **Kit Ordering (01_–16_):** Optional sequential numbering ensures predictable pad assignment (Pads 1–16) during batch import and in the official Roland SP-404MKII desktop app.
- **Mastering Headroom:** Optional true-peak normalization to a user-defined ceiling (default: `-0.5 dBFS`) to ensure consistent kit volume and prevent inter-sample clipping.

---

## Setup & Usage

### 1. Hardware Configuration (One-Time)
Configure the SP-404MKII for class-compliant USB audio on Linux:
1. Press **[SHIFT] + [PAD 13] (UTILITY)**.
2. Navigate to **SYSTEM** and verify:
   - **USB Driver:** Set to **`GENERIC`** (not `VENDOR`).
   - **USB IN:** Set to **`MIX OUT`** to route pads, effects, and patterns over USB.

### 2. Streaming to Discord or Vesktop
1. Connect the SP-404MKII to your PC via USB-C.
2. Launch `sp404-streamer`.
3. In Discord or Vesktop, join a voice channel and click **Share Screen**.
4. Select the **SP-404MKII Streamer** window and ensure **"Also share audio"** is checked.

---

## Installation & Building

### Pre-built Binaries
Download the latest `sp404-streamer-x86_64.AppImage` from [Releases](https://github.com/cntrvsy/sp404-streamer/releases):

```bash
chmod +x sp404-streamer-x86_64.AppImage
./sp404-streamer-x86_64.AppImage
```

### Building from Source

#### Dependencies
```bash
# Arch Linux / CachyOS / Manjaro
sudo pacman -S pipewire clang wayland libxkbcommon

# Fedora
sudo dnf install pipewire-devel clang wayland-devel libxkbcommon-devel

# Ubuntu (24.04+) / Debian (12+)
sudo apt install libpipewire-0.3-dev libspa-0.2-dev libclang-dev libwayland-dev libxkbcommon-dev
```

#### Build & Run
```bash
# Run in development mode
cargo run --release

# Install binary to ~/.cargo/bin
cargo install --path .
```

#### Developer Workflow
This repository includes a preconfigured [`.bacon.toml`](.bacon.toml) for background compilation, clippy lint checks, and testing:
```bash
bacon clippy
bacon test
```

---

## Compositor Window Rules (Wayland)

For tiling and dynamic Wayland compositors (`niri`, `Hyprland`, `Sway`), configure floating window rules matching the default dimensions (`480x545`):

### Niri (`~/.config/niri/config.kdl`)
```kdl
window-rule {
    match app-id="sp404-streamer"
    open-floating true
    default-column-width { fixed 480; }
    default-window-height { fixed 545; }
}
```

### Hyprland (`~/.config/hypr/hyprland.conf`)
```ini
windowrulev2 = float, class:^(sp404-streamer)$
windowrulev2 = size 480 545, class:^(sp404-streamer)$
```

---

## Roadmap & Hardware Capabilities

For an in-depth breakdown of potential features and officially documented SP-404MKII hardware capabilities (custom 128×64 OLED animations, transient auto-slicing, USB MIDI integration, stem recording, and SD card archiving), see the [SP-404MKII Integration Roadmap](ROADMAP.md).

---

## License

Distributed under the [MIT License](LICENSE).

Developed by [frStudios](https://frstudios.co.ke/).
