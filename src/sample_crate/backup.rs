//! Full SD card snapshot archiver (`.tar.zst`).
//!
//! Creates standard POSIX compressed tarballs of an SP-404MKII SD card (`ROLAND/`,
//! `IMPORT/`, `EXPORT/`) so Linux users can back up, version, and swap entire live sets
//! without needing multiple physical SD cards or proprietary software.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("I/O error during SD snapshot: {0}")]
    Io(#[from] std::io::Error),
    #[error("Snapshot cancelled by user")]
    Cancelled,
}

/// Live status of a background `.tar.zst` snapshot operation.
#[derive(Debug, Clone)]
pub enum BackupProgress {
    Idle,
    Running {
        files_done: usize,
        total_files: usize,
        current_path: String,
    },
    Completed {
        archive_path: PathBuf,
        size_bytes: u64,
        file_count: usize,
    },
    Failed(String),
}

impl BackupProgress {
    pub const fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// Resolves the default Linux directory for storing SP-404MKII SD card snapshots:
/// `~/Music/SP404-Backups` (or `/tmp/SP404-Backups` if `$HOME` is unavailable).
pub fn default_backup_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let music_dir = PathBuf::from(home).join("Music").join("SP404-Backups");
        return music_dir;
    }
    std::env::temp_dir().join("SP404-Backups")
}

/// Generates a timestamped `.tar.zst` filename for the given SD card label.
pub fn generate_snapshot_filename(card_label: &str) -> String {
    let clean_label: String = card_label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("sp404_sd_{clean_label}_{ts}.tar.zst")
}

/// Recursively collects all regular files under `current`.
fn collect_files_recursive(current: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        // Skip hidden trash/Spotlight/temporary directories
        if name_str.starts_with('.') || name_str.ends_with(".tmp") {
            continue;
        }
        if path.is_dir() {
            collect_files_recursive(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Creates a compressed `.tar.zst` snapshot of `sd_root` at `dest_archive_path`.
pub fn create_sd_snapshot(
    sd_root: &Path,
    dest_archive_path: &Path,
    progress: &Arc<Mutex<BackupProgress>>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(PathBuf, u64, usize), BackupError> {
    if let Some(parent) = dest_archive_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut all_files = Vec::new();
    collect_files_recursive(sd_root, &mut all_files);
    all_files.sort();
    let total_files = all_files.len();

    let tmp_archive = dest_archive_path.with_extension("tar.zst.tmp");
    {
        let out_file = File::create(&tmp_archive)?;
        // Zstd compression level 3: fast studio throughput (~400 MB/s) with strong compression ratio
        let zstd_encoder = zstd::Encoder::new(out_file, 3)?.auto_finish();
        let mut tar_builder = tar::Builder::new(zstd_encoder);

        for (idx, file_path) in all_files.iter().enumerate() {
            if cancel_flag.load(Ordering::Relaxed) {
                let _ = fs::remove_file(&tmp_archive);
                return Err(BackupError::Cancelled);
            }

            let rel_path = file_path.strip_prefix(sd_root).unwrap_or(file_path);
            {
                let mut guard = progress.lock();
                *guard = BackupProgress::Running {
                    files_done: idx + 1,
                    total_files,
                    current_path: rel_path.to_string_lossy().to_string(),
                };
            }

            tar_builder.append_path_with_name(file_path, rel_path)?;
        }

        tar_builder.finish()?;
    }

    fs::rename(&tmp_archive, dest_archive_path)?;
    let size_bytes = fs::metadata(dest_archive_path)?.len();

    info!(
        "Created SD card snapshot at {} ({} files, {} bytes)",
        dest_archive_path.display(),
        total_files,
        size_bytes
    );

    {
        let mut guard = progress.lock();
        *guard = BackupProgress::Completed {
            archive_path: dest_archive_path.to_path_buf(),
            size_bytes,
            file_count: total_files,
        };
    }

    Ok((dest_archive_path.to_path_buf(), size_bytes, total_files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sd_snapshot_tar_zst_roundtrip() {
        let temp_dir = std::env::temp_dir().join("sp404_backup_test");
        let _ = fs::remove_dir_all(&temp_dir);

        let mock_sd = temp_dir.join("mock_sd");
        let import_dir = mock_sd.join("ROLAND").join("IMPORT").join("LoFi_Kit");
        let picture_dir = mock_sd
            .join("EXPORT")
            .join("PROJECT")
            .join("PROJECT_01")
            .join("PICTURE");
        fs::create_dir_all(&import_dir).unwrap();
        fs::create_dir_all(&picture_dir).unwrap();

        fs::write(import_dir.join("01_kick.wav"), b"RIFF_MOCK_WAV_DATA").unwrap();
        fs::write(picture_dir.join("startup_1.bmp"), b"BM_MOCK_BMP_DATA").unwrap();

        let archive_path = temp_dir.join("test_snapshot.tar.zst");
        let progress = Arc::new(Mutex::new(BackupProgress::Idle));
        let cancel = Arc::new(AtomicBool::new(false));

        let (_, size_bytes, file_count) =
            create_sd_snapshot(&mock_sd, &archive_path, &progress, &cancel).unwrap();
        assert_eq!(file_count, 2);
        assert!(size_bytes > 20);
        assert!(archive_path.is_file());

        // Verify round-trip extraction via standard zstd + tar
        let extract_dir = temp_dir.join("extracted");
        fs::create_dir_all(&extract_dir).unwrap();
        let file = File::open(&archive_path).unwrap();
        let decoder = zstd::Decoder::new(file).unwrap();
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(&extract_dir).unwrap();

        assert_eq!(
            fs::read(
                extract_dir
                    .join("ROLAND")
                    .join("IMPORT")
                    .join("LoFi_Kit")
                    .join("01_kick.wav")
            )
            .unwrap(),
            b"RIFF_MOCK_WAV_DATA"
        );
        assert_eq!(
            fs::read(
                extract_dir
                    .join("EXPORT")
                    .join("PROJECT")
                    .join("PROJECT_01")
                    .join("PICTURE")
                    .join("startup_1.bmp")
            )
            .unwrap(),
            b"BM_MOCK_BMP_DATA"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
