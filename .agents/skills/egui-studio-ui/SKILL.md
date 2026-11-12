---
name: egui-studio-ui
description: >-
  Guidelines, API reference, and architectural patterns for building structured,
  hardware-inspired Pro-Audio UIs in Rust with egui 0.37 and eframe 0.37.
  Use this skill whenever creating, modifying, or reviewing UI views, custom painter
  widgets, layouts, or themes in sp404-streamer.
---

# egui 0.37 Pro-Audio Studio UI Skill

Use this skill to ensure every UI panel and custom widget in `sp404-streamer` compiles cleanly against **`egui 0.37`**, respects the compact Wayland window geometry (`480×545`), maintains zero-blocking frame times, and delivers a tactile hardware aesthetic.

---

## 1. Critical `egui 0.37` API Rules (Do Not Use Legacy 0.2x Syntax)

`eframe` and `egui` `0.37.0` introduced breaking API changes compared to older `0.24–0.30` training examples. Always use the `0.37` signatures:

| Concept | ❌ Legacy `egui` (Will Not Compile) | ✅ `egui 0.37.0` (Required) |
| :--- | :--- | :--- |
| **`eframe::App` entry** | `fn update(&mut self, ctx: &Context, frame: &mut Frame)` | `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)` |
| **Accessing `Context`** | Passed directly to `update` | Call `ui.ctx()` inside `fn ui(...)` |
| **Corner Rounding** | `egui::Rounding::same(4.0)` | `egui::CornerRadius::same(4)` (takes `u8`) |
| **Frame Margins** | `egui::Margin::symmetric(12.0, 10.0)` | `egui::Margin::symmetric(12, 10)` (takes `i8`) |
| **`rect_stroke`** | `painter.rect_stroke(rect, rounding, stroke)` | `painter.rect_stroke(rect, CornerRadius::same(3), stroke, StrokeKind::Inside)` |
| **Keyboard Focus Check** | `ctx.wants_keyboard_input()` | `ui.ctx().egui_wants_keyboard_input()` |
| **Dropped File Path** | `dropped_file.path.clone()` (`Option<PathBuf>`) | `dropped_file.path().to_path_buf()` (`&Path` trait method) |
| **`line_segment`** | `painter.line_segment([p1, p2], stroke)` | `painter.line_segment([p1, p2], stroke)` |
| **Slider Trailing Fill** | `slider.trailing_fill(true)` | `slider.trailing_fill(true)` |

---

## 2. Layout Architecture & Wayland Viewport Discipline

The app targets a compact floating window (`480 × 545` default, `380 × 360` minimum) for Wayland tiling compositors (`niri`, `Hyprland`, `Sway`).

1. **Root Window Shell (Pinned Header, Fluid Body, Pinned Footer)**:
   * Build the root viewport in `fn ui` using vertical `StripBuilder`:
     * `Size::exact(64.0)`: Pinned hardware bar (USB PipeWire audio + live SD card status) + 3-column equal-width tab switcher (`StripBuilder::sizes(Size::remainder(), 3)`).
     * `Size::remainder()`: Fluid active tab viewport (`STREAM HUD`, `SAMPLE CRATE`, `OLED STUDIO`).
     * `Size::exact(24.0)`: Anchored bottom status guidance and brand footer.
2. **Rack Card Hierarchy & Anatomy**:
   * Group related controls inside `RolandTheme::rack_card_frame().show(ui, |ui| { ... })`.
   * Inside every card, call `ui.set_width(ui.available_width());` first so cards span the full rack width.
   * Standardized header with hairline divider:
     ```rust
     RolandTheme::section_header(ui, "CARD TITLE", |ui| {
         // Right-aligned status chips or actions
     });
     ```
   * Separate stacked rack cards with `RolandTheme::CARD_GAP` (`8.0px`).
3. **Aligned 2-Column Control Decks (`RolandTheme::form_grid`)**:
   * All forms and control rows must use `RolandTheme::form_grid(ui, "unique_id", |ui| { ... })`.
   * Column 0 is a fixed `90px` uppercase label rendered via `RolandTheme::grid_label(ui, "LABEL")`.
   * Column 1 spans the remaining width for controls (sliders, comboboxes, buttons). Always end rows with `ui.end_row();`.
4. **Recessed Display Wells**:
   * Readouts, waveforms, VU meters, and OLED previews must sit inside a recessed well:
     * Background: `RolandTheme::WELL_BG` (`rgb(9, 10, 13)`)
     * Border: `Stroke::new(1.0, RolandTheme::BORDER_SUBTLE)` (`rgb(26, 28, 36)`)
     * Corner Radius: `CornerRadius::same(4)`
5. **Natural Vertical Flow vs Rigid Pixel Math**:
   * Do NOT force non-table views (`STREAM HUD`, `OLED STUDIO`) into rigid internal `StripBuilder` cell heights (`Size::exact(142.0)`). Hardcoded pixel budgets cause clipping whenever window sizes or font metrics shift.
   * Instead, wrap the fluid tab body in `egui::ScrollArea::vertical().auto_shrink([false, false])`.
   * Let rack cards determine their natural height (`RolandTheme::rack_card_frame().show(ui, |ui| { ... })`).
   * For the hardware `OledDisplayWidget`, size it predictably to rack width (`scale = ((avail_w - 32.0) / 128.0).clamp(1.8, 2.5)`).
   * Result: When window has sufficient height, cards expand cleanly with zero scrollbar; when window is resized small or guides are toggled, smooth scrolling prevents any widget or slider from ever clipping.
6. **Keyboard Focus & State Discipline**:
   * When listening for keyboard shortcuts (`Space`, `Delete`, `Arrows`), always check `if !ui.ctx().egui_wants_keyboard_input()` to prevent capturing keys while typing in text inputs.
   * Inside `ui.horizontal(...)`, defer mutations (like deleting or reordering items) using an enum action (e.g. `RowAction`) applied *after* the UI closure finishes.

---

## 3. Color Palette & Contrast Rules (`RolandTheme`)

Always reference constants from [`src/ui/theme.rs`](../../src/ui/theme.rs):

* **Surfaces**:
  * `RolandTheme::BG_CHARCOAL` (`#0B0C10`) — Window background
  * `RolandTheme::PANEL_BG` (`#111217`) — Rack card fill
  * `RolandTheme::WELL_BG` (`#090A0D`) — Recessed screen/meter well
* **Accents & Status LEDs**:
  * `RolandTheme::ACCENT_ORANGE` (`#FF7A00`) — Primary active control / Roland signature orange
  * `RolandTheme::ACCENT_AMBER` (`#FFB300`) — Secondary highlight / warning / OLED amber
  * `RolandTheme::ACCENT_CYAN` (`#38BDF8`) — Info / waveform playhead / slice markers
  * `RolandTheme::METER_GREEN` (`#10B981`) — Connected / active audio / nominal signal
  * `RolandTheme::METER_RED` (`#EF4444`) — Error / clip / destructive action
* **Contrast Rule**:
  * Whenever rendering text on top of `ACCENT_ORANGE`, `ACCENT_AMBER`, or `METER_GREEN` filled buttons/badges, **always use `RolandTheme::TEXT_ON_ACCENT` (`Color32::BLACK`)** with `.strong()`. Never put white text on bright orange/amber fills.

---

## 4. Custom Hardware Widget Pattern (`ui.painter_at`)

Do not add third-party `egui_*` widget crates (they lag behind `egui 0.37` and cause duplicate crate versions). Build hardware controls in `src/ui/widgets/` using `ui.allocate_exact_size` and `ui.painter_at(rect)`:

```rust
pub fn ui(self, ui: &mut Ui, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        // 1. Draw recessed well or bevel background
        painter.rect_filled(rect, CornerRadius::same(4), RolandTheme::WELL_BG);
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, if response.hovered() {
                RolandTheme::ACCENT_ORANGE
            } else {
                RolandTheme::BORDER_COLOR
            }),
            StrokeKind::Inside,
        );
        // 2. Render foreground geometry (arcs, pixel matrices, LED bars)
    }
    response
}
```

### Standard Custom Interaction Idioms:
* **Rotary Knobs / Faders**:
  * Dragging vertically (`-response.drag_delta().y * sensitivity`) adjusts value.
  * Holding `Shift` while dragging enables fine-grain adjustment (`0.1×` speed).
  * `response.double_clicked()` resets the parameter to its default/unity value.
* **OLED Pixel Matrix (`128×64`)**:
  * Render active pixels with a `1px` dark gap (`cell_size - 1.0`) when zoomed (`>= 3×`) so the user sees an authentic hardware dot-matrix grid rather than a blurry scaled texture.

---

## 5. Non-Blocking Concurrency & Adaptive Repainting

1. **Never Block `fn ui()`**:
   * Any disk I/O, GIF decoding, audio resampling, transient detection, or `.tar.zst` archiving **must** run on a background thread (`std::thread::spawn`).
   * Share progress with the UI via `Arc<Mutex<ProgressEnum>>` or lock-free atomics (`AtomicU64`, `AtomicBool`).
2. **Adaptive Frame Rate**:
   * Only request `30–60 FPS` (`request_repaint_after(Duration::from_millis(16))`) when audio peaks are active, a preview is playing, or an OLED animation is running.
   * Drop to idle refresh (`100–250 ms`) when static to conserve CPU/GPU on Linux laptops.

---

## 6. Linux Onboarding & Hardware Guidance UX

* **Triple Input Support**: For file ingestion (audio or images/GIFs), always support:
  1. Native portal file picker (`rfd::FileDialog` via `xdg-desktop-portal`).
  2. Window drag-and-drop (`ui.ctx().input(|i| i.raw.dropped_files.clone())`).
  3. Built-in procedural presets or one-click defaults where applicable.
* **Hardware Round-Trip Checklists**:
  * Whenever an action requires follow-up steps on the physical SP-404MKII (e.g. `SHIFT + PAD 13` or `SHIFT + PAD 14`), display a concise, scannable hardware instruction pill directly inside the card rather than assuming the user has read the Roland manual.

