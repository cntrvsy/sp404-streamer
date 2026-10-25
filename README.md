# SP-404MKII Streamer

A small, lightweight audio bridge that lets you stream your Roland SP-404MKII live to Discord/OBS etc.. on Linux in full stereo.

![SP-404MKII Streamer Preview](assets/sp404-streamer.png)

---

## The Problem

I just got the sp-404 mk2, love it, yes i have a windows laptop but my main rig is linux (CachyOS) and i wanted to use it with my friends on discord. But getting it to work was a hassle, if you've ever tried to jam on an SP-404 over a Discord call on Linux, you've probably run into these walls:

1. **Discord only streams application windows, not hardware inputs:** When you share your screen in Discord (or Vesktop), it captures audio coming _out_ of an app (like a game or media player). It completely ignores USB hardware audio inputs like your sampler.
2. **Voice chat ruins beat audio:** If you route your SP-404 into your microphone input instead, Discord runs it through aggressive noise suppression, echo cancellation, and mono compression. Kicks get muffled, 808s disappear, and sidechains sound glitchy.
3. **Patchbay fatigue:** While you _can_ manually link nodes together in `qpwgraph` or `helvum`, nobody wants to spend 5 minutes drawing cables in a matrix just to play a quick pattern for a friend.

**SP-404MKII Streamer solves this in one click.** Plug your SP-404 into your PC over USB-C, launch the app, share the window in Discord, and you're streaming 48kHz stereo.

---

## The Philosophy

- **Do one job and get out of the way:** I wanted a tiny window that lives on your desktop, gives you clean stereo meters, routes the audio cleanly, and doesn't get in your face.
- **Zero clutter, zero configuration:** It detects your SP-404 automatically, grabs the audio, and presents a playback stream that Discord recognizes.
- **Respect your resources:** Written in native Rust with PipeWire and egui. It uses almost zero CPU and drops to a low refresh rate when you're not playing so it won't drain your laptop or heat up your rig.

---

## What This App Is NOT

To keep this tool focused, reliable, and fast, it is deliberately simple:

- **It is NOT a DAW or audio recorder:** It does not record tracks, export stems, or trim samples. If you want to record your live session, use a dedicated tool like Reaper, Audacity, or Ardour.
- **It is NOT a sample manager or project editor:** It doesn't back up SD cards, assign samples to pads, or edit project files (for that, check out the official Roland software or community librarians).
- **It is NOT a general audio patchbay:** It won't route your guitars, synths, and webcams simultaneously, and it doesn't host audio plugins/VSTs. If you need complex multi-device routing, tools like `qpwgraph` or `Carla` are built for that.
- **It is NOT a permanent background daemon:** It runs as a normal desktop app when you want to stream. When you close the window, it cleanly detaches and leaves your audio setup exactly as it found it.

---

## Features

- **Stereo VU Meters:** Roland-styled dual meters with peak hold and clip indicators so you know your signal isn't redlining into Discord.
- **Audio Output Selector:** Choose whether you want to hear the sound through your headphones, monitor speakers, USB DAC, or system default.
- **Local Monitor Mute:** If you have your headphones plugged directly into the front headphone jack of the SP-404, click **Mute Local PC Monitor**. This stops the PC from playing the sound back into your ears (preventing delay/echo) while Discord continues to receive the live audio stream.
- **Low Latency:** Uses native PipeWire buffers (~5.3ms) with automatic jitter buffering so your finger drumming feels responsive and sounds clean without clicks or dropouts.
- **Hotplug-Friendly:** You can plug or unplug your SP-404 while the app is open—it connects automatically when the device is detected.

---

## Getting Started

### 1. Check Your SP-404 Settings (One-time check)

For Linux to talk to the SP-404 without proprietary drivers, the unit needs to be in class-compliant mode:

1. On the SP-404MKII, press **[SHIFT] + [PAD 13] (UTILITY)**.
2. Select **SYSTEM** and check:
   - **USB Driver:** Must be set to **`GENERIC`** (not `VENDOR`).
   - **USB IN:** Set to **`MIX OUT`** so your internal pads, effects, and patterns are sent over USB.

### 2. Run the App

#### Pre-built Binaries (All Linux Distros)
Download the latest `sp404-streamer-x86_64.AppImage` (or `sp404-streamer-x86_64-linux.tar.gz`) from [Releases](https://github.com/cntrvsy/sp404-streamer/releases):

```bash
chmod +x sp404-streamer-x86_64.AppImage
./sp404-streamer-x86_64.AppImage
```

#### Build Locally
If you have Rust and PipeWire installed, you can build and run directly with Cargo:

```bash
cargo run --release
```

To install it to your user environment (`~/.cargo/bin`):

```bash
cargo install --path .
```

### 3. Stream to Discord

1. Open **Discord** (or **Vesktop** on Wayland).
2. Join a voice channel and click **Share Screen**.
3. Under the **Window** tab, select **SP-404MKII Streamer**.
4. Make sure **"Also share audio"** is checked.
5. Hit your pads! Your friends will hear your beats in high-fidelity stereo.

---

## Building from Source

To compile from source, you only need Rust and the PipeWire development headers:

```bash
# Arch Linux / CachyOS / Manjaro
sudo pacman -S pipewire clang wayland libxkbcommon

# Fedora
sudo dnf install pipewire-devel clang wayland-devel libxkbcommon-devel

# Ubuntu (24.04+) / Debian (12+) [requires PipeWire >= 0.3.50]
sudo apt install libpipewire-0.3-dev libspa-0.2-dev libclang-dev libwayland-dev libxkbcommon-dev

# Build release binary
cargo build --release
```

---

## Optional: Wayland Window Rules

If you use a tiling or dynamic Wayland compositor (like `niri`, `Hyprland`, or `Sway`), you can set a floating rule for the window so it sits neatly in the corner of your screen:

**Niri (`~/.config/niri/config.kdl`):**

```kdl
window-rule {
    match app-id="sp404-streamer"
    open-floating true
    default-column-width { fixed 370; }
    default-window-height { fixed 255; }
}
```

**Hyprland (`~/.config/hypr/hyprland.conf`):**

```ini
windowrulev2 = float, class:^(sp404-streamer)$
windowrulev2 = size 370 255, class:^(sp404-streamer)$
```

---

## License

MIT
