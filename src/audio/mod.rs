//! Native PipeWire audio subsystem.
//!
//! Encompasses background device discovery, low-latency audio capture and playback streams,
//! thread-safe atomic metering state, and a FIFO asynchronous command worker.

pub mod command;
pub mod pipewire_client;
pub mod port_manager;
pub mod preview_player;
pub mod state;

pub use command::AudioCommand;
pub use pipewire_client::PipewireClient;
pub use port_manager::PortManager;
pub use preview_player::PreviewPlayer;
pub use state::SharedAudioState;
