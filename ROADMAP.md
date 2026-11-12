# SP-404MKII Integration Roadmap & Verified Capabilities Reference

This document tracks features, enhancements, and hardware integrations for `sp404-streamer` that **do not require reverse engineering firmware, undocumented SysEx, or proprietary `.BIN` project files**.

All specifications below are cross-referenced against Roland's official **SP-404MKII Reference Manual (v4/v5)** and **MIDI Implementation Chart (v5.00+)**.

---

## Recommended Implementation Order

Features are ordered by reliability against Roland's documented hardware behavior. **Items 1, 2, 8, and 4** form the recommended initial release milestone.

### Tier 1: Safe & Officially Documented
1. **Screen Editor (Startup & Screensaver):**
   * 128×64 preview, Floyd-Steinberg dithering / thresholding, and a 16-frame animation sampler.
   * Writes into `EXPORT/PROJECT/PROJECT_XX/PICTURE/` and walks the user through the required hardware project export/re-import round trip.
2. **Sample Prep & Conforming (Core Implemented):**
   * Convert to 48 kHz / 16-bit linear PCM WAV (preserving mono/stereo), with optional TPDF dither, leading silence trimming, short de-click micro-fades, and peak normalization. Writes results to `IMPORT/`.
3. **Auto-Chop to Individual WAVs:**
   * Onset detection slices loops/breaks into numbered WAV files (`01_slice.wav` .. `16_slice.wav`) inside `IMPORT/<Kit_Name>/`.
   * *Note:* The user still assigns samples to pads via the on-device import menu (`SHIFT + PAD 14`); filenames do not automatically populate pads.
4. **Virtual Pad Grid over USB MIDI:**
   * Configurable for **MIDI Mode A** (10 channels, notes `36–51`) and **MIDI Mode B** (2 channels, notes `12–91`), plus adjustable note offset settings.
   * Sends **Note On only** (Roland's MIDI implementation chart lists `Note Off` as unrecognized).
5. **Effect Controller (USB MIDI CC):**
   * `CC #16–18` for CTRL 1–3, `CC #19` for EFX On/Off switch, `CC #80–82` for CTRL 4–6, and `CC #83` for EFX Type number.
   * Supports per-bus MIDI channel routing, X/Y pads, MIDI learn, and software LFOs.
6. **Pattern Selector / Chainer:**
   * Uses standard **Program Change** messages (`PC #0` = Pattern 1) to sequence or trigger patterns on a timer. (Does not rely on undocumented note triggers).
7. **MIDI Clock Master:**
   * Sends Realtime Clock (`0xF8`), Start (`0xFA`), and Stop (`0xFC`) with a tempo knob and tap tempo, paired with verification against the unit's MIDI sync settings.

### Tier 2: Safe Plain Filesystem Utilities
8. **Full SD Card Snapshot:**
   * Archive the entire SD card (`ROLAND/`, `IMPORT/`, `EXPORT/`) as a timestamped `.tar.zst` archive for instant backup and live-set swapping.
9. **Duplicate Finder & Library Browser:**
   * Content-hash (`BLAKE3`) files on the SD card, provide waveform auditioning, search, and compare `IMPORT/` usage against free card capacity.
10. **Multipad Export Organizer:**
    * Organize and batch-rename individual pattern WAV stems exported by the SP-404MKII into clean DAW stem folders.

### Tier 3: Linux Audio (PipeWire)
11. **One-Click Resample Recorder:**
    * Capture the SP-404MKII's class-compliant USB audio stream into lossless WAV files with auto-trim and normalization (verifying actual PipeWire channel layout via `pw-dump`).
12. **Desktop-to-SP Audio Bridge (Core Implemented):**
    * Route desktop application audio into the SP-404MKII's USB input via PipeWire virtual sinks and loopbacks.

### Explicitly Skipped / Out of Scope
* **Editing `.BIN` project files or SysEx Pad-to-RAM injection:** Proprietary and undocumented.
* **Writing musical key metadata to hardware:** The SP-404MKII does not persist key metadata (as of firmware v5.52); any key/BPM detection stays strictly inside the desktop app UI.
* **Automatic bank folder mapping:** Folders inside `IMPORT/` are for human browsing on the OLED screen; they do not auto-assign to hardware banks.

---

## Detailed Hardware Specifications

### 1. Custom Startup Screens & Screensavers (OLED)

* **Dimensions:** Exactly `128 × 64` pixels.
* **Format:** Windows Bitmap (`.BMP`).
* **Color Depth:** Hardware accepts 1-, 4-, 8-, and 24-bit BMPs, but renders strictly in monochrome on the OLED. Exporting **1-bit monochrome** is preferred to give the user exact WYSIWYG dithering control and avoid internal gray thresholding artifacts.
* **Naming Conventions:**
  * Startup (Opening) Images: `startup_1.bmp`, `startup_2.bmp` (2 images per project).
  * Screensaver Frames: `screen_saver_1.bmp` through `screen_saver_16.bmp` (up to 16 frames cycled sequentially).
* **SD Card Path & Hardware Round-Trip Workflow:**
  ```text
  SD-CARD/
  └── EXPORT/
      └── PROJECT/
          └── PROJECT_01/   <-- Numbered project folder exported by hardware
              └── PICTURE/
                  ├── startup_1.bmp
                  ├── startup_2.bmp
                  ├── screen_saver_1.bmp
                  ├── ...
                  └── screen_saver_16.bmp
  ```
  **Required User Steps on Hardware:**
  1. Export the target project to the SD card (`SHIFT + PAD 14` -> Export -> Project).
  2. Sync the generated `.bmp` files into `EXPORT/PROJECT/PROJECT_XX/PICTURE/` using `sp404-streamer`.
  3. Enable custom screensavers on the unit: `SHIFT + PAD 13 (UTILITY)` -> `SYSTEM` -> `Screen Saver Type: Custom`.
  4. Re-import the project from the SD card (`SHIFT + PAD 14` -> Import -> Project).

---

### 2. USB MIDI Implementation (Verified v5.00+ Spec)

#### A. Pad Triggering Modes (Mode A vs. Mode B)
The SP-404MKII provides two MIDI pad modes in `UTILITY -> SYSTEM -> MIDI`. The desktop app must allow switching between both modes and configuring note offsets:

| Mode | Channel Layout | Default Note Range | Behavior |
| :--- | :--- | :--- | :--- |
| **Mode A** | **Channels 1–10** (1 channel per Bank A–J) | Notes `36–51` (C2–D#3) per channel | Each bank is isolated on its own MIDI channel. Note offsets are adjustable. |
| **Mode B** | **Channel 1** (Banks A–E) & **Channel 2** (Banks F–J) | Notes `12–91` (C0–G6) per channel | Compresses 5 banks (80 pads) onto each of the two channels. Note offsets are adjustable. |

* **Note On Only:** According to the official MIDI Implementation Chart, **Note Off is not recognized (`x`)**. Virtual pad triggers must send `Note On` messages; sample stop/sustain behavior is controlled by the hardware pad's Gate/One-Shot and Mute Group settings.

#### B. Effect Bus Control Changes (CC)
Effect controls operate across the unit's effect buses (with a dedicated MIDI channel per bus according to the MIDI implementation chart):

| Control Change | Target Parameter | Value Range / Notes |
| :--- | :--- | :--- |
| **`CC #16`** | **CTRL 1** Knob | `0–127` (Primary effect parameter 1) |
| **`CC #17`** | **CTRL 2** Knob | `0–127` (Primary effect parameter 2) |
| **`CC #18`** | **CTRL 3** Knob | `0–127` (Primary effect parameter 3) |
| **`CC #19`** | **EFX On/Off Switch** | `0–63` = Off, `64–127` = On |
| **`CC #80`** | **CTRL 4** Parameter | `0–127` (Sub-page parameter 4) |
| **`CC #81`** | **CTRL 5** Parameter | `0–127` (Sub-page parameter 5) |
| **`CC #82`** | **CTRL 6** Parameter | `0–127` (Sub-page parameter 6) |
| **`CC #83`** | **Effect Number** | Selects active MFX type on the target bus |

#### C. Pattern Selection
* **Program Change (`PC`):** Patterns are triggered via standard MIDI Program Change messages (`PC #0` selects Pattern 1, `PC #1` selects Pattern 2, etc.).

---

### 3. Audio Conforming & Sample Crate (`IMPORT/`)

* **Internal Hardware Format:** The SP-404MKII converts all imported audio to **48 kHz / 16-bit linear PCM** internally.
* **SD Card Import Safety:** Roland's documentation notes 16-bit linear WAV/AIFF/MP3 for SD card import (with conflicting tables mentioning 24-bit). Conforming all exported crate samples to **48,000 Hz, 16-bit linear PCM WAV** guarantees 100% compatibility and zero on-device conversion overhead.
* **Polyphony Preservation:** Exporting mono sources as 1-channel mono WAVs preserves hardware voice polyphony (consuming 1 voice instead of 2).

---

## Official References & Sources

1. **Roland SP-404MKII Reference Manual v4 (Custom Opening & Screensaver Images):**
   [https://static.roland.com/manuals/sp-404mk2_reference_v4/en-US/7948148378524043.html](https://static.roland.com/manuals/sp-404mk2_reference_v4/en-US/7948148378524043.html)
2. **Roland SP-404MKII Reference Manual v5.00 (MIDI Implementation Chart):**
   [https://static.roland.com/manuals/sp-404mk2_reference_v500/en-US/8010996378593163.html](https://static.roland.com/manuals/sp-404mk2_reference_v500/en-US/8010996378593163.html)
3. **Roland Support — How Are the MIDI Notes Mapped?:**
   [https://support.roland.com/hc/en-us/articles/25793219901851-SP-404MK2-How-are-the-MIDI-notes-mapped](https://support.roland.com/hc/en-us/articles/25793219901851-SP-404MK2-How-are-the-MIDI-notes-mapped)
4. **Roland Support — What Audio File Formats Can Be Imported?:**
   [https://support.roland.com/hc/en-us/articles/4408190553883-SP-404MK2-Audio-file-formats-that-can-be-imported](https://support.roland.com/hc/en-us/articles/4408190553883-SP-404MK2-Audio-file-formats-that-can-be-imported)
5. **Roland Articles — Customizing Faceplates, Startup Screens & Screensavers:**
   [https://articles.roland.com/?p=40192](https://articles.roland.com/?p=40192)
6. **Sweetwater — Working with Projects and Samples on the Roland SP-404MKII:**
   [https://www.sweetwater.com/sweetcare/articles/working-with-projects-and-samples-on-the-roland-sp-404mkii/](https://www.sweetwater.com/sweetcare/articles/working-with-projects-and-samples-on-the-roland-sp-404mkii/)
7. **SparkyMK2 Release Notes (Hardware Key Metadata Behavior):**
   [https://github.com/TuscanPL/SparkyMK2/releases](https://github.com/TuscanPL/SparkyMK2/releases)
