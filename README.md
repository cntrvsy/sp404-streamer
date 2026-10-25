# SP-404MKII Discord Streamer

A zero-configuration, low-latency audio bridge that allows **Roland SP-404MKII** users on Linux to stream pristine 48kHz stereo directly to Discord via a compact Wayland HUD.

![SP-404MKII Streamer Preview](assets/sp404-streamer.png)

---

## Why This Exists

1. **Discord Screen-Sharing Only Captures Playback Streams:** When you share a window in Discord / Vesktop on Linux, it only captures audio outputs (playback nodes) originating from that application's PID. It cannot capture input devices (like the SP-404 USB-C audio).
2. **Prinstine Stereo Audio:** Discord Voice calls compress mic inputs to mono with noise suppression. Screensharing an application window delivers full 48kHz stereo up to 384kbps.
3. **No Complex Linux Patchbays:** You don't need to open `helvum`, `qpwgraph`, or run shell scripts each time you want to play live pads to friends.

---

## Features

- **PipeWire Native:** Zero clock drift, automatic sample rate negotiation, and ultra-low buffer quantum (`256/48000` $\approx$ 5.3ms latency) for tight finger drumming.
- **Hardware-Inspired HUD:** Compact floating window designed with Roland SP-404 aesthetic (matte charcoal, amber/orange glowing LEDs, real-time stereo meters with peak hold and clip indicators).
- **Physical Monitor Unlinking:** Click **"Mute Local PC Monitor"** to disconnect PC speakers/headphones while Discord continues streaming at full volume (ideal if you already have headphones plugged directly into the SP-404 hardware).
- **Adaptive CPU Usage:** Runs at 60 FPS when active audio is playing, then automatically drops to ~8 FPS when silent to save battery and GPU cycles.
- **Universal AppImage:** Builds on `ubuntu-22.04` (glibc 2.35) for friction-free execution across Arch/CachyOS, Debian, Fedora, Ubuntu, and SteamOS.

---

## Quick Start

### 1. Hardware Setting (Important!)
Ensure your SP-404MKII is set to standard USB Audio Class 2 mode:
* On the SP-404MKII: Press **[SHIFT] + [PAD 13] (UTILITY)** $\to$ **SYSTEM** $\to$ set **USB Driver** to **`GENERIC`** (not `VENDOR`).

### 2. Run the App
Download `sp404-streamer-x86_64.AppImage` from [Releases](https://github.com/your-username/sp404-streamer/releases), make it executable, and run:
```bash
chmod +x sp404-streamer-x86_64.AppImage
./sp404-streamer-x86_64.AppImage
```

### 3. Stream in Discord
1. Open Discord or Vesktop.
2. Join a voice channel $\to$ Click **Share Screen**.
3. Select **Window** $\to$ Choose **SP-404MKII Streamer** (ensure "Also share audio" is checked).
4. Jam away! Friends hear your sampler in high-fidelity stereo.

---

## Niri Wayland Window Rule

To make the streamer open automatically as a floating HUD in `niri`, add this to `~/.config/niri/config.kdl`:

```kdl
window-rule {
    match app-id="sp404-streamer"
    open-floating true
    default-column-width { fixed 360; }
    default-window-height { fixed 220; }
}
```

---

## Building from Source

```bash
# Install dependencies (Ubuntu/Debian)
sudo apt install libpipewire-0.3-dev libspa-0.2-dev libclang-dev libwayland-dev libxkbcommon-dev

# Install dependencies (Arch/CachyOS)
sudo pacman -S pipewire clang wayland libxkbcommon

# Build & Run
cargo run --release
```

---

## Automated Releases

A GitHub Actions workflow (`.github/workflows/release.yaml`) automatically compiles and releases portable AppImages when a tag is pushed to `main`:

```bash
git checkout main
git tag v0.1.0
git push origin v0.1.0
```
