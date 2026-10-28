//! Data models for the SP-404MKII Sample Crate.

use std::path::PathBuf;

/// Metadata and waveform preview for a staged sample.
#[derive(Debug, Clone)]
pub struct StagedSample {
    pub id: u64,
    pub source_path: PathBuf,
    pub clean_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_secs: f32,
    pub peak_db: f32,
    /// 512 downsampled peak values (0.0 to 1.0) for the high-resolution waveform strip
    pub waveform_overview: Vec<f32>,
    pub is_mono: bool,
}

impl StagedSample {
    /// Formats duration as e.g. "0.4s" or "2m 14s"
    pub fn duration_formatted(&self) -> String {
        if self.duration_secs < 60.0 {
            format!("{:.1}s", self.duration_secs)
        } else {
            let mins = (self.duration_secs / 60.0).floor() as u32;
            let secs = (self.duration_secs % 60.0).floor() as u32;
            format!("{mins}m {secs:02}s")
        }
    }

    /// Formats peak in decibels or "-inf dB" if silence
    pub fn peak_db_formatted(&self) -> String {
        if self.peak_db <= -90.0 {
            "-∞ dB".to_string()
        } else {
            format!("{:+.1} dB", self.peak_db)
        }
    }
}

/// Options controlling audio conformation and export to the SP-404 SD card.
#[derive(Debug, Clone)]
pub struct ConformOptions {
    /// True peak normalization enabled
    pub normalize_peak: bool,
    /// Target peak ceiling in dBFS (default: -0.5 dBFS)
    pub peak_ceiling_db: f32,
    /// Whether to prefix pad numbers (e.g. "01_", "02_") on export
    pub pad_numbered: bool,
    /// Max character length for base name (default: 16 chars for clean OLED readability)
    pub max_name_len: usize,
}

impl Default for ConformOptions {
    fn default() -> Self {
        Self {
            normalize_peak: true,
            peak_ceiling_db: -0.5,
            pad_numbered: false,
            max_name_len: 16,
        }
    }
}

/// Information about a detected SD card or target storage volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdCardInfo {
    pub mount_path: PathBuf,
    pub import_path: PathBuf,
    pub label: String,
    pub device_node: Option<String>,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub is_read_only: bool,
    /// Existing kit folder names found under IMPORT/
    pub existing_kits: Vec<String>,
}

impl SdCardInfo {
    pub fn free_space_formatted(&self) -> String {
        let gb = self.free_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        if gb >= 1.0 {
            format!("{gb:.1} GB Free")
        } else {
            let mb = self.free_bytes as f64 / (1024.0 * 1024.0);
            format!("{mb:.0} MB Free")
        }
    }

    /// Returns the display path relative to the SD card root (e.g. "ROLAND/IMPORT" or "IMPORT")
    pub fn relative_import_path(&self) -> String {
        if let Ok(rel) = self.import_path.strip_prefix(&self.mount_path) {
            rel.to_string_lossy().to_string()
        } else {
            "IMPORT".to_string()
        }
    }
}

/// Information about an unmounted removable block device detected on the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmountedCard {
    pub device_node: String, // e.g. "/dev/sdc1"
    pub label: String,       // e.g. "SP404MKII" or "NO NAME"
    pub fstype: String,      // e.g. "vfat"
    pub size_formatted: String,
}
