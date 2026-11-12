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
| **`line_segment`** | `painter.line_segment([p1, p2], stroke)` | `painter.line_segment([p1, p2], stroke)` |
| **Slider Trailing Fill** | `slider.trailing_fill(true)` | `slider.trailing_fill(true)` |

---

## 2. Layout Architecture & Wayland Viewport Discipline

The app targets a compact floating window (`480 × 545` default, `380 × 360` minimum) for Wayland tiling compositors (`niri`, `Hyprland`, `Sway`).

1. **Rack Card Hierarchy**:
   * Group related controls inside `RolandTheme::rack_card_frame().show(ui, |ui| { ... })`.
   * Inside every rack card, call `ui.set_width(ui.available_width());` first so cards span the full rack width cleanly without jittering when content changes.
   * Separate stacked rack cards with `ui.add_space(8.0);`.
2. **Recessed Display Wells**:
   * Readouts, waveforms, VU meters, and OLED previews must sit inside a recessed well:
     * Background: `RolandTheme::WELL_BG` (`rgb(9, 10, 13)`)
     * Border: `Stroke::new(1.0, RolandTheme::BORDER_SUBTLE)` (`rgb(26, 28, 36)`)
     * Corner Radius: `CornerRadius::same(4)`
3. **Avoid Nested Scrollbars & Layout Thrashing**:
   * Never nest vertical `ScrollArea`s. Only one primary table/list area in a tab should scroll vertically; headers and footers remain fixed.
   * For multi-column tables, always use `egui_extras::TableBuilder` with explicit `Column::exact(...)` and one `Column::remainder()`.
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
