pub mod audio;
pub mod ui;

use std::sync::Arc;
use eframe::egui;
use tracing::info;

use audio::{PipewireClient, SharedAudioState};
use ui::{RolandTheme, Sp404StreamerApp};

fn main() -> eframe::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting SP-404MKII Discord Streamer...");

    // 1. Initialize shared audio state
    let audio_state = SharedAudioState::new();

    // 2. Start PipeWire audio engine in background thread
    let _pipewire_client = PipewireClient::start(Arc::clone(&audio_state));

    // 3. Configure compact Wayland-friendly viewport
    let icon_data = eframe::icon_data::from_png_bytes(include_bytes!("../assets/sp404-streamer.png")).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([360.0, 220.0])
        .with_min_inner_size([320.0, 190.0])
        .with_resizable(true)
        .with_app_id("sp404-streamer")
        .with_title("SP-404MKII Streamer");

    if let Some(icon) = icon_data {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // 4. Launch GUI
    let state_for_ui = Arc::clone(&audio_state);
    eframe::run_native(
        "SP-404MKII Streamer",
        native_options,
        Box::new(move |cc| {
            RolandTheme::apply(&cc.egui_ctx);
            Ok(Box::new(Sp404StreamerApp::new(state_for_ui)))
        }),
    )
}
