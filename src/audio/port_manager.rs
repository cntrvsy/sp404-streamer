use std::process::Command;
use tracing::{debug, info, warn};

/// Port and link management for PipeWire.
/// Handles unlinking/linking local monitor output without cutting the audio feed to Discord.
pub struct PortManager;

impl PortManager {
    /// Disconnects our playback stream's output ports from physical speakers/headphones.
    /// This allows Discord to continue streaming our audio without the user hearing an echo
    /// through their PC headphones if they are monitoring directly on the SP-404 hardware.
    pub fn unlink_local_monitor(stream_node_name: &str) {
        info!("Unlinking local monitor for stream: {}", stream_node_name);

        // Find links currently connected to stream_node_name's output ports
        let output = match Command::new("pw-link").arg("-l").output() {
            Ok(out) => out,
            Err(e) => {
                warn!("Failed to execute pw-link -l: {}", e);
                return;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current_source_port = String::new();

        for line in stdout.lines() {
            let trimmed = line.trim();
            if !line.starts_with(' ') && !line.starts_with('\t') {
                current_source_port = trimmed.to_string();
            } else if trimmed.starts_with("|->") {
                let target_port = trimmed.trim_start_matches("|->").trim();
                if current_source_port.starts_with(stream_node_name) {
                    debug!("Unlinking: {} -> {}", current_source_port, target_port);
                    let _ = Command::new("pw-link")
                        .args(["-d", &current_source_port, target_port])
                        .output();
                }
            }
        }
    }

    /// Re-links our playback stream's output ports to the default audio sink so the user
    /// can hear the SP-404 through their PC speakers/headphones.
    pub fn relink_local_monitor(stream_node_name: &str) {
        info!("Relinking local monitor for stream: {}", stream_node_name);

        // Find available playback ports for physical audio sinks
        let output = match Command::new("pw-link").arg("-i").output() {
            Ok(out) => out,
            Err(e) => {
                warn!("Failed to execute pw-link -i: {}", e);
                return;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut sink_fl = None;
        let mut sink_fr = None;

        for line in stdout.lines() {
            let port = line.trim();
            // Look for standard physical sink playback ports
            if port.starts_with("alsa_output.") {
                if port.ends_with(":playback_FL") && sink_fl.is_none() {
                    sink_fl = Some(port.to_string());
                } else if port.ends_with(":playback_FR") && sink_fr.is_none() {
                    sink_fr = Some(port.to_string());
                }
            }
        }

        if let (Some(fl_dst), Some(fr_dst)) = (sink_fl, sink_fr) {
            let fl_src = format!("{}:output_FL", stream_node_name);
            let fr_src = format!("{}:output_FR", stream_node_name);

            debug!("Linking {} -> {}", fl_src, fl_dst);
            let _ = Command::new("pw-link").args([&fl_src, &fl_dst]).output();

            debug!("Linking {} -> {}", fr_src, fr_dst);
            let _ = Command::new("pw-link").args([&fr_src, &fr_dst]).output();
        } else {
            warn!("Could not identify default stereo sink playback ports to relink");
        }
    }

    /// Checks if a PipeWire device/node matches the Roland SP-404MKII.
    pub fn is_sp404_device(name: &str, description: &str) -> bool {
        let name_lower = name.to_lowercase();
        let desc_lower = description.to_lowercase();

        desc_lower.contains("sp-404")
            || desc_lower.contains("sp404")
            || (desc_lower.contains("roland") && desc_lower.contains("404"))
            || name_lower.contains("sp-404")
            || name_lower.contains("sp404")
            || (name_lower.contains("roland") && name_lower.contains("404"))
    }
}
