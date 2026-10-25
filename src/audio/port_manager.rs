//! PipeWire port, node, and link management.
//!
//! Handles querying available audio playback sink devices, resolving the current OS default
//! audio sink from PipeWire metadata, linking SP-404MKII hardware capture ports to the capture
//! stream, and unlinking local speaker monitor ports so the beatmaker hears only their SP-404
//! hardware headphones while Discord receives the live stream.

use std::process::Command;
use tracing::{debug, info, warn};

/// Describes an audio playback sink discovered from the PipeWire graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSinkDevice {
    /// PipeWire node name (e.g. `alsa_output.pci-0000_03_00.1.hdmi-stereo`)
    pub name: String,
    /// Human-friendly descriptive name (e.g. `pci-0000 03 00.1.hdmi-stereo`)
    pub description: String,
}

/// Port and link management for PipeWire audio routing.
pub struct PortManager;

impl PortManager {
    /// Disconnects our playback stream's output ports from physical speakers/headphones.
    /// This allows Discord to continue streaming our audio without the user hearing an echo
    /// through their PC headphones if they are monitoring directly on the SP-404 hardware.
    pub fn unlink_local_monitor(stream_node_name: &str) {
        info!("Unlinking local monitor for stream: {}", stream_node_name);

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

    /// Pure parser: Extracts available audio playback sink devices from `pw-link -i` stdout.
    /// Excludes the SP-404 hardware to prevent feedback routing.
    pub fn parse_sinks_from_pw_link(stdout: &str) -> Vec<AudioSinkDevice> {
        let mut node_names = std::collections::BTreeSet::new();

        for line in stdout.lines() {
            let port = line.trim();
            let is_supported =
                port.starts_with("alsa_output.") || port.starts_with("bluez_output.");
            if !is_supported || port.to_lowercase().contains("404") {
                continue;
            }
            if let Some((node, _)) = port.split_once(':') {
                node_names.insert(node.to_string());
            }
        }

        node_names
            .into_iter()
            .map(|name| {
                let friendly_desc = name
                    .trim_start_matches("alsa_output.")
                    .trim_start_matches("bluez_output.")
                    .replace('_', " ");
                AudioSinkDevice {
                    name,
                    description: friendly_desc,
                }
            })
            .collect()
    }

    /// Enumerate all available audio playback sink devices from PipeWire (`pw-link -i`).
    /// Excludes the SP-404 hardware to prevent feedback routing.
    pub fn get_available_sinks() -> Vec<AudioSinkDevice> {
        let output = match Command::new("pw-link").arg("-i").output() {
            Ok(out) if out.status.success() => out,
            _ => return Vec::new(),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::parse_sinks_from_pw_link(&stdout)
    }

    /// Pure parser: Extracts the default audio sink node name from `pw-metadata` stdout.
    pub fn parse_default_sink_from_pw_metadata(stdout: &str) -> Option<String> {
        for line in stdout.lines() {
            if !line.contains("default.audio.sink") {
                continue;
            }
            if let Some(start) = line.find("{\"name\":\"") {
                let rest = &line[start + 9..];
                if let Some(end) = rest.find('"') {
                    let name = rest[..end].trim();
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }

    /// Query the current OS default audio sink name using native PipeWire metadata (`pw-metadata`).
    pub fn get_default_sink_name() -> Option<String> {
        let out = Command::new("pw-metadata")
            .args(["0", "default.audio.sink"])
            .output()
            .ok()
            .filter(|o| o.status.success())?;

        let stdout = String::from_utf8_lossy(&out.stdout);
        Self::parse_default_sink_from_pw_metadata(&stdout)
    }

    /// Pure parser: Extracts Left and Right playback ports for a target sink.
    pub fn extract_sink_playback_ports(
        stdout: &str,
        sink_name: &str,
    ) -> (Option<String>, Option<String>) {
        let prefix = format!("{}:", sink_name);
        let mut sink_fl = None;
        let mut sink_fr = None;

        for line in stdout.lines() {
            let port = line.trim();
            if port.starts_with(&prefix) {
                if port.ends_with(":playback_FL") || port.ends_with("_FL") {
                    sink_fl = Some(port.to_string());
                } else if port.ends_with(":playback_FR") || port.ends_with("_FR") {
                    sink_fr = Some(port.to_string());
                }
            }
        }

        // Fallback: If exact prefix didn't match, look for standard alsa_output
        if sink_fl.is_none() || sink_fr.is_none() {
            for line in stdout.lines() {
                let port = line.trim();
                if port.starts_with("alsa_output.") && !port.to_lowercase().contains("404") {
                    if port.ends_with(":playback_FL") && sink_fl.is_none() {
                        sink_fl = Some(port.to_string());
                    } else if port.ends_with(":playback_FR") && sink_fr.is_none() {
                        sink_fr = Some(port.to_string());
                    }
                }
            }
        }

        (sink_fl, sink_fr)
    }

    /// Routes playback stream to a specific sink, or to the system default sink if None.
    pub fn route_playback_to_sink(stream_node_name: &str, target_sink: Option<&str>) {
        info!(
            "Routing playback {} to target sink: {:?}",
            stream_node_name, target_sink
        );

        // 1. Unlink current physical connections from stream_node_name
        Self::unlink_local_monitor(stream_node_name);

        // 2. Identify the target sink name
        let sink_name = match target_sink {
            Some(name) => name.to_string(),
            None => match Self::get_default_sink_name() {
                Some(name) => name,
                None => {
                    warn!("No default sink found to route playback to");
                    return;
                }
            },
        };

        // 3. Find available playback ports for physical audio sinks
        let output = match Command::new("pw-link").arg("-i").output() {
            Ok(out) => out,
            Err(e) => {
                warn!("Failed to execute pw-link -i: {}", e);
                return;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let (sink_fl, sink_fr) = Self::extract_sink_playback_ports(&stdout, &sink_name);

        if let (Some(fl_dst), Some(fr_dst)) = (sink_fl, sink_fr) {
            let fl_src = format!("{}:output_FL", stream_node_name);
            let fr_src = format!("{}:output_FR", stream_node_name);

            info!("Linking Playback: {} -> {}", fl_src, fl_dst);
            let _ = Command::new("pw-link").args([&fl_src, &fl_dst]).output();

            info!("Linking Playback: {} -> {}", fr_src, fr_dst);
            let _ = Command::new("pw-link").args([&fr_src, &fr_dst]).output();
        } else {
            warn!("Could not identify playback ports for sink: {}", sink_name);
        }
    }

    /// Re-links our playback stream's output ports to the default audio sink so the user
    /// can hear the SP-404 through their PC speakers/headphones.
    pub fn relink_local_monitor(stream_node_name: &str) {
        Self::route_playback_to_sink(stream_node_name, None);
    }

    /// Pure parser: Discovers Left and Right capture output ports for an SP-404 node.
    pub fn extract_sp404_ports(
        stdout: &str,
        sp404_node_name: &str,
    ) -> (Option<String>, Option<String>) {
        let prefix = format!("{}:", sp404_node_name);
        let mut sp_fl = None;
        let mut sp_fr = None;
        let mut candidate_ports = Vec::new();

        for line in stdout.lines() {
            let port = line.trim();
            if port.starts_with(&prefix) {
                candidate_ports.push(port.to_string());
                if port.ends_with(":capture_FL")
                    || port.ends_with(":output_FL")
                    || port.ends_with("_FL")
                {
                    sp_fl = Some(port.to_string());
                } else if port.ends_with(":capture_FR")
                    || port.ends_with(":output_FR")
                    || port.ends_with("_FR")
                {
                    sp_fr = Some(port.to_string());
                }
            }
        }

        // Fallback: If FL/FR naming is absent, use first 2 ports if available
        if sp_fl.is_none() && !candidate_ports.is_empty() {
            sp_fl = Some(candidate_ports[0].clone());
        }
        if sp_fr.is_none() && candidate_ports.len() > 1 {
            sp_fr = Some(candidate_ports[1].clone());
        }

        (sp_fl, sp_fr)
    }

    /// Attempts to discover and link SP-404MKII hardware capture ports to the capture stream.
    /// Returns true if both FL and FR ports were discovered and linked, false otherwise.
    pub fn try_link_sp404_capture(sp404_node_name: &str, capture_stream_name: &str) -> bool {
        // 1. Unlink any non-SP404 sources from our capture stream
        Self::unlink_non_sp404_capture(Some(sp404_node_name), capture_stream_name);

        // 2. Discover available output ports for the SP-404 node
        let output = match Command::new("pw-link").arg("-o").output() {
            Ok(out) => out,
            Err(e) => {
                warn!("Failed to execute pw-link -o: {}", e);
                return false;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let (sp_fl, sp_fr) = Self::extract_sp404_ports(&stdout, sp404_node_name);

        let in_fl = format!("{}:input_FL", capture_stream_name);
        let in_fr = format!("{}:input_FR", capture_stream_name);

        let linked_l = if let Some(src_fl) = sp_fl {
            info!("Linking SP-404 L: {} -> {}", src_fl, in_fl);
            Command::new("pw-link")
                .args([&src_fl, &in_fl])
                .status()
                .is_ok()
        } else {
            false
        };

        let linked_r = if let Some(src_fr) = sp_fr {
            info!("Linking SP-404 R: {} -> {}", src_fr, in_fr);
            Command::new("pw-link")
                .args([&src_fr, &in_fr])
                .status()
                .is_ok()
        } else {
            false
        };

        linked_l && linked_r
    }

    /// Links the SP-404MKII hardware capture ports with a bounded retry backoff loop,
    /// accommodating the delay between PipeWire node announcement and ALSA port creation.
    pub fn link_sp404_capture_with_retry(
        sp404_node_name: &str,
        capture_stream_name: &str,
        max_attempts: u32,
        delay: std::time::Duration,
    ) -> bool {
        info!(
            "Linking capture stream {} to SP-404 node {} (max attempts: {})",
            capture_stream_name, sp404_node_name, max_attempts
        );

        for attempt in 1..=max_attempts {
            if Self::try_link_sp404_capture(sp404_node_name, capture_stream_name) {
                info!("Successfully linked SP-404 ports on attempt {}", attempt);
                return true;
            }
            std::thread::sleep(delay);
        }

        warn!(
            "Failed to link SP-404 capture after {} attempts",
            max_attempts
        );
        false
    }

    /// Legacy convenience wrapper
    pub fn link_sp404_capture(sp404_node_name: &str, capture_stream_name: &str) {
        Self::link_sp404_capture_with_retry(
            sp404_node_name,
            capture_stream_name,
            5,
            std::time::Duration::from_millis(40),
        );
    }

    /// Unlinks any non-SP404 sources connected to capture stream (e.g. Blue Yeti, webcam).
    pub fn unlink_non_sp404_capture(sp404_node_name: Option<&str>, capture_stream_name: &str) {
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
                if target_port.starts_with(capture_stream_name) {
                    let is_sp404 =
                        sp404_node_name.is_some_and(|sp| current_source_port.starts_with(sp));

                    if !is_sp404 {
                        info!(
                            "Unlinking non-SP-404 capture link: {} -> {}",
                            current_source_port, target_port
                        );
                        let _ = Command::new("pw-link")
                            .args(["-d", &current_source_port, target_port])
                            .output();
                    }
                }
            }
        }
    }

    /// Disconnects our capture stream when the SP-404 is disconnected.
    pub fn unlink_capture_stream(capture_stream_name: &str) {
        Self::unlink_non_sp404_capture(None, capture_stream_name);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sp404_device_positive() {
        assert!(PortManager::is_sp404_device(
            "alsa_input.usb-Roland_SP-404MKII_000000000000-00.analog-stereo",
            "SP-404MKII Analog Stereo"
        ));
        assert!(PortManager::is_sp404_device(
            "alsa_input.usb-Roland_Corp._SP404-00",
            "Roland SP404"
        ));
        assert!(PortManager::is_sp404_device(
            "generic_node",
            "Roland Sampler 404 MK2"
        ));
    }

    #[test]
    fn test_is_sp404_device_negative() {
        assert!(!PortManager::is_sp404_device(
            "alsa_input.usb-Blue_Microphones_Yeti_Stereo_Microphone_REV8-00.analog-stereo",
            "Yeti Stereo Microphone Analog Stereo"
        ));
        assert!(!PortManager::is_sp404_device(
            "alsa_output.pci-0000_03_00.1.hdmi-stereo",
            "Navi 21/23 HDMI/DP Audio Controller Digital Stereo (HDMI)"
        ));
        assert!(!PortManager::is_sp404_device(
            "alsa_output.pci-0000_00_1f.3.analog-stereo",
            "Built-in Audio Analog Stereo"
        ));
    }

    #[test]
    fn test_parse_sinks_from_pw_link() {
        let sample_pw_link_output = r#"
Midi-Bridge:Midi Through: Port-0 (playback)
Midi-Bridge:SP-404MKII: MIDI 1 (playback)
alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1:playback_FL
alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1:playback_FR
bluez_output.00_1B_66_81_2A_3C.1:playback_FL
bluez_output.00_1B_66_81_2A_3C.1:playback_FR
alsa_output.usb-Roland_SP-404MKII_000-00.analog-stereo:playback_FL
alsa_output.usb-Roland_SP-404MKII_000-00.analog-stereo:playback_FR
vesktop:input_FL
vesktop:input_FR
"#;

        let sinks = PortManager::parse_sinks_from_pw_link(sample_pw_link_output);
        assert_eq!(sinks.len(), 2);

        // Verify HDMI sink
        assert!(sinks.iter().any(
            |s| s.name == "alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1"
                && s.description == "pci-0000 03 00.1.hdmi-stereo-extra1"
        ));

        // Verify Bluetooth sink
        assert!(
            sinks
                .iter()
                .any(|s| s.name == "bluez_output.00_1B_66_81_2A_3C.1"
                    && s.description == "00 1B 66 81 2A 3C.1")
        );

        // Verify SP-404 is excluded
        assert!(!sinks.iter().any(|s| s.name.contains("SP-404MKII")));
    }

    #[test]
    fn test_parse_default_sink_from_pw_metadata() {
        let sample_metadata = r#"
Found "default" metadata 42
update: id:0 key:'default.audio.sink' value:'{"name":"alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1"}' type:'Spa:String:JSON'
update: id:0 key:'default.audio.source' value:'{"name":"alsa_input.usb-Blue-00"}' type:'Spa:String:JSON'
"#;

        let sink = PortManager::parse_default_sink_from_pw_metadata(sample_metadata);
        assert_eq!(
            sink,
            Some("alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1".to_string())
        );
    }

    #[test]
    fn test_parse_default_sink_from_pw_metadata_missing() {
        let empty_metadata = "Found \"default\" metadata 42\n";
        assert_eq!(
            PortManager::parse_default_sink_from_pw_metadata(empty_metadata),
            None
        );
    }

    #[test]
    fn test_extract_sp404_ports_standard() {
        let sample_pw_link_o = r#"
alsa_input.usb-Roland_SP-404MKII_000-00.analog-stereo:capture_FL
alsa_input.usb-Roland_SP-404MKII_000-00.analog-stereo:capture_FR
alsa_input.usb-Blue_Yeti_00-00.analog-stereo:capture_FL
"#;

        let (fl, fr) = PortManager::extract_sp404_ports(
            sample_pw_link_o,
            "alsa_input.usb-Roland_SP-404MKII_000-00.analog-stereo",
        );
        assert_eq!(
            fl,
            Some("alsa_input.usb-Roland_SP-404MKII_000-00.analog-stereo:capture_FL".to_string())
        );
        assert_eq!(
            fr,
            Some("alsa_input.usb-Roland_SP-404MKII_000-00.analog-stereo:capture_FR".to_string())
        );
    }

    #[test]
    fn test_extract_sp404_ports_numbered_fallback() {
        let sample_pw_link_o = r#"
alsa_input.usb-Roland_SP404:port_1
alsa_input.usb-Roland_SP404:port_2
"#;

        let (fl, fr) =
            PortManager::extract_sp404_ports(sample_pw_link_o, "alsa_input.usb-Roland_SP404");
        assert_eq!(fl, Some("alsa_input.usb-Roland_SP404:port_1".to_string()));
        assert_eq!(fr, Some("alsa_input.usb-Roland_SP404:port_2".to_string()));
    }
}
