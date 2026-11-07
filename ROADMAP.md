# SP-404MKII Integration Roadmap & Capabilities Reference

This document tracks all potential features, enhancements, and hardware integrations for `sp404-streamer` that **do not require reverse engineering firmware or proprietary binary blobs**.

Because the Roland SP-404MKII is **USB Audio Class Compliant**, **USB MIDI Class Compliant**, and relies on a standard **FAT32/exFAT SD card filesystem**, extensive hardware integration is achievable using officially documented specifications and standard Linux audio/MIDI infrastructure.

---

## Feasibility & Protocol Matrix

| Capability Area | Protocols / Standards Used | Requires Firmware RE? | Roland Documentation Status | Feasibility |
| :--- | :--- | :---: | :---: | :---: |
| **Custom Launch & Screensaver Animations** | 1-bit BMP, FAT32/exFAT SD Card | ❌ No | Officially Documented (Firmware v2.0+) | **100% Ready** |
| **Audio Conforming (16-bit/48kHz WAV)** | Linear PCM WAV (`hound`), Resampling (`rubato`) | ❌ No | Officially Documented (SD Import Spec) | **Implemented** |
| **Transient Auto-Chop (16 Slices)** | DSP onset detection, FAT32 SD Card | ❌ No | Open DSP standard | **100% Ready** |
| **Zero-Crossing & Click Prevention** | PCM audio sample manipulation | ❌ No | Open DSP standard | **100% Ready** |
| **BPM & Key Detection / Pitch Tuning** | Autocorrelation / FFT pitch detection | ❌ No | Open DSP standard | **100% Ready** |
| **Multi-Bank Kit Builder (160 Pads)** | FAT32 filesystem (`IMPORT/Bank_X/`) | ❌ No | Officially Documented SD structure | **100% Ready** |
| **Virtual 4×4 Pad Triggering** | USB MIDI Note On/Off (Ch 1 & Ch 2) | ❌ No | Officially Documented MIDI Spec | **100% Ready** |
| **Live MFX / Macro Knob Control** | USB MIDI CC (#19, #20, #21, etc.) | ❌ No | Officially Documented MIDI Spec | **100% Ready** |
| **BPM Master Clock & Transport** | USB MIDI Realtime (`0xF8`, `0xFA`, `0xFC`) | ❌ No | Standard MIDI 1.0 Spec | **100% Ready** |
| **Pattern Song Arranger** | USB MIDI Program Change & Note Trigger | ❌ No | Officially Documented MIDI Spec | **100% Ready** |
| **Desktop-to-SP Audio Bridge** | PipeWire Loopback & Virtual Sinks | ❌ No | Linux standard (PipeWire API) | **Implemented** |
| **One-Click Resample Stem Recorder** | PipeWire capture, WAV writer | ❌ No | Class Compliant USB Audio | **100% Ready** |
| **SD Project Backup & Archive** | Filesystem tarball (`.tar.zst` / `.zip`) | ❌ No | Standard POSIX / FAT32 file I/O | **100% Ready** |
| *Direct Pad-to-RAM Sample Injection* | Proprietary USB SysEx commands | ⚠️ Yes | Undocumented (Roland Desktop App only) | Deferred |
| *Direct `.BIN` Project File Editing* | Proprietary binary data format | ⚠️ Yes | Undocumented internal memory layout | Deferred |

---

## 1. Custom Launch Animations & Screensavers (OLED Engine)

The SP-404MKII graphic OLED display operates at **128 × 64 pixels**. Firmware v2.0+ allows loading custom startup pictures and multi-frame screensaver loops.

### Official Roland Specifications
* **Dimensions:** Exactly `128 x 64` pixels.
* **Format:** Windows Bitmap (`.BMP`).
* **Color Depth:** 1-bit Monochrome (Pure Black `#000000` / Pure White `#FFFFFF`).
* **Startup Images:**
  * `startup_1.bmp`, `startup_2.bmp` (shown sequentially upon boot/load).
* **Screensaver Animation Frames:**
  * `screen_saver_1.bmp` through `screen_saver_16.bmp` (cycles up to a 16-frame looping flipbook when the unit goes idle).
* **Target Path on SD Card:**
  ```text
  SD-CARD/
  └── EXPORT/
      └── PROJECT/
          └── <PROJECT_NAME>/
              └── PICTURE/
                  ├── startup_1.bmp
                  ├── startup_2.bmp
                  ├── screen_saver_1.bmp
                  ├── ...
                  └── screen_saver_16.bmp
  ```

### Proposed `sp404-streamer` Feature
* **GIF / Video / Image Ingestion:** Drag-and-drop any `.gif`, `.png`, or `.mp4` clip into the UI.
* **16-Frame Sampling:** Auto-distribute 16 evenly spaced keyframes across the animation loop.
* **Dithering & Threshold Engine:**
  * Floyd-Steinberg error diffusion dithering.
  * Contrast, brightness, and inversion (negative) adjustments.
  * Live pixel-perfect 128×64 OLED preview widget in `egui` (simulating the amber/white display).
* **1-Click Sync to SD:** Auto-generate 1-bit BMP files and write directly to `EXPORT/PROJECT/<selected_project>/PICTURE/`.

---

## 2. Advanced Sample Crate & DSP Tools

### A. 16-Bit / 48 kHz Hardware Conforming (Current & Enhancements)
* **Status:** Implemented (`src/sample_crate/conformer.rs`).
* **Verification:** Roland SP-404MKII standalone SD card import (`SHIFT + PAD 14`) strictly rejects 24-bit WAVs with `"File Not Supported!"`. All exports must remain 16-bit linear PCM at 48 kHz.
* **Next DSP Step:** Optional TPDF (Triangular Probability Density Function) dither during float-to-16-bit quantization.

### B. Transient Auto-Chop to 16 Pads
* **Concept:** Drop a drum break or sample loop into the crate and slice it into exactly 16 pads.
* **Algorithm:** High-frequency energy onset detection (or spectral flux) to detect drum transients.
* **Export Mapping:** Automatically outputs slices formatted as `01_kick.wav` to `16_crash.wav` directly into a target folder under `IMPORT/<Kit_Name>/` for immediate 1-to-1 pad loading on the hardware.

### C. Zero-Crossing & De-Clicking
* **Silence Stripping:** Trims leading silence to the first non-zero sample so pads hit with zero latency.
* **Micro-Fading:** Applies a 1–2 ms raised-cosine fade at the start and end of every sample to eliminate DC pops and truncation clicks.

### D. BPM & Pitch Detection / Chromatic Tuning
* **Pitch Detection:** Auto-detect the root musical key of one-shots (e.g. F#2) and calculate the semitone pitch shift to transpose it to C.
* **Utility:** SP-404MKII Chromatic Mode (`SHIFT + PAD 4`) assumes samples are tuned to C for melodic pad play.

### E. Multi-Bank Kit Builder (160 Pads)
* **Concept:** Organize samples across all 10 banks (Bank A through Bank J = 160 pads).
* **Folder Hierarchy:** Exports structured folders to the SD card:
  ```text
  SD-CARD/
  └── IMPORT/
      ├── 01_BANK_A/ (Pads 01-16)
      ├── 02_BANK_B/ (Pads 01-16)
      └── ...
  ```

---

## 3. USB MIDI Live Hardware Integration

The SP-404MKII exposes standard USB MIDI class compliance on Linux (`/dev/snd/midi*` and ALSA/PipeWire MIDI endpoints). Roland officially publishes the [SP-404MKII MIDI Implementation](https://www.roland.com/global/support/by_product/sp-404mk2/owners_manuals/).

### A. Virtual 4×4 Pad Controller
* **Triggering Scheme (Officially Documented):**
  * **MIDI Channel 1:**
    * Bank A: Notes 36–51 (C2–D#3)
    * Bank B: Notes 52–67 (E3–G4)
    * Bank C: Notes 68–83 (G#4–B5)
    * Bank D: Notes 84–99 (C6–D#7)
    * Bank E: Notes 100–115 (E7–G8)
  * **MIDI Channel 2:**
    * Banks F through J (same note layout).
* **UI Component:** A visual 4×4 grid in `sp404-streamer` that lights up in real time when pads are triggered, allowing desktop triggering via mouse or keyboard hotkeys.

### B. Real-Time MFX / Macro Knob Control
* **Documented Control Changes:**
  * `CC #19`: CTRL 1 Knob
  * `CC #20`: CTRL 2 Knob
  * `CC #21`: CTRL 3 Knob
  * `CC #22`: Main Volume / Master Level
  * `CC #16`: Bus 1 Effect Select
  * `CC #17`: Bus 2 Effect Select
* **Proposed Features:**
  * Onscreen macro sliders and X/Y control pads.
  * Desktop software LFOs to automate filter sweeps, vinyl warp, or delay repeats on the hardware over MIDI.

### C. Master MIDI Clock & Transport Sync
* **MIDI Realtime Messages:**
  * `0xF8`: Timing Clock (24 pulses per quarter note).
  * `0xFA`: Start.
  * `0xFC`: Stop.
* **Utility:** Lock the SP-404MKII's internal delay, roll, and pattern sequencer to your computer's tempo with zero timing drift.

### D. Visual Song Arranger / Pattern Trigger
* **Pattern Triggering:** Patterns 1–16 across banks can be triggered via MIDI Program Change or Note messages.
* **Proposed Feature:** A visual timeline sequencer on Linux where users lay out patterns (e.g. Intro -> Verse -> Hook -> Outro) and `sp404-streamer` triggers pattern changes over MIDI, providing an arranger view for live performances.

---

## 4. PipeWire Audio & Resample Workflow

The SP-404MKII operates as a **24-bit / 48,000 Hz** multi-channel USB Audio interface on Linux.

### A. One-Click Resample & Stem Capture
* Stream audio from the SP-404MKII into a low-overhead memory ring buffer.
* One-click "Record Stem / Jam" button that writes conformed WAV files directly to disk with auto-normalization and silence trimming.

### B. Desktop-to-SP Quick Sampling Bridge
* Allow routing arbitrary application audio (browser, Spotify, DAW, media player) directly into the SP-404MKII's USB In.
* Eliminates audio cables and DAC conversion noise when sampling audio directly from the computer.

### C. Cue Bus Monitoring
* Route audition previews directly to the SP-404MKII Cue channel (Headphones) without bleeding into the Main output mix or stream broadcast.

---

## 5. SD Card Project Snapshots & Library Management

### A. One-Click Full Project Backup
* Create atomic compressed archives (`.tar.zst` or `.zip`) of the entire `ROLAND/` and `IMPORT/` tree with automated timestamping.
* Enables swapping entire multi-gigabyte project states without buying multiple SD cards.

### B. Duplicate Sample Identification
* Fast BLAKE3 or SHA-256 hash scanning across SD card directories to flag and clean up redundant samples.

---

## References & Official Documentation

1. **Roland SP-404MKII Support - Supported Audio Formats:**
   [https://support.roland.com/hc/en-us/articles/4408190553883](https://support.roland.com/hc/en-us/articles/4408190553883)
2. **Roland SP-404MKII Reference Manual & MIDI Implementation:**
   [https://www.roland.com/global/support/by_product/sp-404mk2/owners_manuals/](https://www.roland.com/global/support/by_product/sp-404mk2/owners_manuals/)
3. **Roland SP-404MKII Version 2.0 / 3.0 / 4.0 Supplementary Manuals (Custom BMP Specifications):**
   [https://www.roland.com/global/support/by_product/sp-404mk2/updates_drivers/](https://www.roland.com/global/support/by_product/sp-404mk2/updates_drivers/)
