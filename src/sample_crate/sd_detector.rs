//! Detection and safe unmounting of SP-404MKII SD cards on Linux.
//!
//! Provides proactive detection of both mounted and unmounted removable block devices,
//! automatic udisks2 mounting for re-inserted cards, support for both ROLAND/IMPORT and IMPORT
//! folder hierarchies, read-only/lock-switch detection, and reliable unmounting via udisksctl.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;
use tracing::info;

use super::model::{SdCardInfo, UnmountedCard};

/// Storage operation errors for block devices and filesystem interactions.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Failed to execute storage command: {0}")]
    Io(#[from] std::io::Error),

    #[error("udisksctl mount failed: {0}")]
    MountFailed(String),

    #[error("Mount succeeded but mount point could not be verified")]
    MountPointNotFound,

    #[error("Invalid path string for statvfs: {0}")]
    InvalidPath(#[from] std::ffi::NulError),

    #[error("Filesystem statvfs system call failed")]
    StatvfsFailed,

    #[error("Unmount failed: {0}")]
    UnmountFailed(String),
}

/// Filesystem capacity metrics and mount status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FilesystemCapacity {
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub is_read_only: bool,
}

/// Result of a comprehensive SD card scan.
#[derive(Debug, Clone, Default)]
pub struct SdDetectionStatus {
    pub card: Option<SdCardInfo>,
    pub unmounted_candidates: Vec<UnmountedCard>,
}

/// Resolves the optimal SP-404MKII import path on the given mounted volume.
///
/// Priority:
/// 1. `mount/ROLAND/IMPORT` (Official Roland SP-404MKII hardware format)
/// 2. `mount/IMPORT` (Legacy/user custom import folder)
/// 3. If `mount/ROLAND` directory exists, returns `mount/ROLAND/IMPORT`
/// 4. Default: `mount/ROLAND/IMPORT` (Standard Roland hardware structure)
pub fn resolve_import_path(mount: &Path) -> PathBuf {
    let roland_import = mount.join("ROLAND").join("IMPORT");
    if roland_import.is_dir() {
        return roland_import;
    }
    let root_import = mount.join("IMPORT");
    if root_import.is_dir() {
        return root_import;
    }
    let roland_dir = mount.join("ROLAND");
    if roland_dir.is_dir() {
        return roland_import;
    }
    // Default to official Roland hardware path
    roland_import
}

/// Parses KEY="VALUE" pairs from `lsblk -P` output lines.
pub fn parse_lsblk_line(line: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_alphabetic() || c == '_' {
            let mut key = String::new();
            key.push(c);
            while let Some(ch) = chars.next_if(|&next| next != '=') {
                key.push(ch);
            }
            chars.next(); // Consume '='

            if chars.peek() == Some(&'"') {
                chars.next(); // consume opening quote
                let mut val = String::new();
                for ch in chars.by_ref() {
                    if ch == '"' {
                        break;
                    }
                    val.push(ch);
                }
                map.insert(key, val);
            }
        }
    }

    map
}

/// Scans for unmounted removable block devices (e.g. SD cards plugged in without desktop auto-mount).
pub fn scan_for_unmounted_cards() -> Vec<UnmountedCard> {
    let mut unmounted = Vec::new();
    let output = match Command::new("lsblk")
        .args(["-P", "-o", "NAME,MOUNTPOINT,RM,HOTPLUG,FSTYPE,LABEL,SIZE"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return unmounted,
    };

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let kv = parse_lsblk_line(line);
        let name = kv.get("NAME").cloned().unwrap_or_default();
        let mountpoint = kv.get("MOUNTPOINT").cloned().unwrap_or_default();
        let rm = kv.get("RM").cloned().unwrap_or_default();
        let hotplug = kv.get("HOTPLUG").cloned().unwrap_or_default();
        let fstype = kv.get("FSTYPE").cloned().unwrap_or_default().to_lowercase();
        let label = kv.get("LABEL").cloned().unwrap_or_default();
        let size = kv.get("SIZE").cloned().unwrap_or_default();

        // Must be currently unmounted
        if !mountpoint.is_empty() {
            continue;
        }

        // Must be removable or hotplug
        if rm != "1" && hotplug != "1" {
            continue;
        }

        // Must be a filesystem suitable for SD cards
        if fstype != "vfat" && fstype != "exfat" && fstype != "msdos" {
            continue;
        }

        let device_node = if name.starts_with('/') {
            name
        } else {
            format!("/dev/{name}")
        };

        unmounted.push(UnmountedCard {
            device_node,
            label: if label.is_empty() {
                "SD Card".to_string()
            } else {
                label
            },
            fstype,
            size_formatted: size,
        });
    }

    unmounted
}

/// Attempts to mount a block device using `udisksctl mount -b <device> --no-user-interaction`.
pub fn mount_block_device(device_node: &str) -> Result<PathBuf, StorageError> {
    info!("Mounting block device {device_node} via udisksctl...");
    let output = Command::new("udisksctl")
        .args(["mount", "-b", device_node, "--no-user-interaction"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return Err(StorageError::MountFailed(stderr.trim().to_string()));
    }

    // Typical stdout: "Mounted /dev/sdc1 at /run/media/username/SP404MKII.\n"
    if let Some(pos) = stdout.find(" at ") {
        let path_str = stdout[pos + 4..].trim().trim_end_matches('.');
        let path = PathBuf::from(path_str);
        if path.is_dir() {
            return Ok(path);
        }
    }

    // Fallback: lookup in /proc/mounts
    if let Some(mount) = find_mount_point_for_device(device_node) {
        return Ok(mount);
    }

    Err(StorageError::MountPointNotFound)
}

/// Looks up the mount point directory for a block device in `/proc/mounts`.
pub fn find_mount_point_for_device(device_node: &str) -> Option<PathBuf> {
    if let Ok(mounts_data) = fs::read_to_string("/proc/mounts") {
        for line in mounts_data.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0] == device_node {
                let p = PathBuf::from(parts[1]);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Looks up the block device node for a mount point in `/proc/mounts`.
pub fn find_device_for_mount_point(mount_path: &Path) -> Option<String> {
    let mount_str = mount_path.to_str()?;
    if let Ok(mounts_data) = fs::read_to_string("/proc/mounts") {
        for line in mounts_data.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == mount_str {
                return Some(parts[0].to_string());
            }
        }
    }
    None
}

/// Deep scan for mounted and unmounted SD cards, with automatic auto-mounting for SP-404 cards.
pub fn detect_sd_card_full() -> SdDetectionStatus {
    // 1. Try finding an already mounted SP-404 card
    if let Some(card) = find_mounted_sd_card() {
        return SdDetectionStatus {
            card: Some(card),
            unmounted_candidates: Vec::new(),
        };
    }

    // 2. Scan for unmounted removable cards (handles cards pulled and plugged back in)
    let unmounted = scan_for_unmounted_cards();

    // 3. Proactive auto-mount: If an unmounted card is labeled SP404 or ROLAND, mount it automatically!
    for candidate in &unmounted {
        let lbl = candidate.label.to_uppercase();
        if lbl.contains("SP404") || lbl.contains("SP-404") || lbl.contains("ROLAND") {
            info!(
                "Found unmounted SP-404 card at {}. Auto-mounting...",
                candidate.device_node
            );
            if let Ok(mount_path) = mount_block_device(&candidate.device_node) {
                let import_path = resolve_import_path(&mount_path);
                if let Some(card) =
                    build_sd_info(&mount_path, &import_path, Some(&candidate.device_node))
                {
                    return SdDetectionStatus {
                        card: Some(card),
                        unmounted_candidates: Vec::new(),
                    };
                }
            }
        }
    }

    // 4. Return unmounted candidates for manual 1-click mounting in UI
    SdDetectionStatus {
        card: None,
        unmounted_candidates: unmounted,
    }
}

/// Scans mounted filesystems for an SP-404 card.
pub fn find_mounted_sd_card() -> Option<SdCardInfo> {
    let mut candidate_mounts: Vec<PathBuf> = Vec::new();

    // 1. Scan /run/media/$USER and /media/$USER
    let user = std::env::var("USER").unwrap_or_default();
    let media_roots = [
        format!("/run/media/{user}"),
        format!("/media/{user}"),
        "/run/media".to_string(),
        "/media".to_string(),
    ];

    for root in &media_roots {
        let p = Path::new(root);
        if p.is_dir()
            && let Ok(entries) = fs::read_dir(p)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && !candidate_mounts.contains(&path) {
                    candidate_mounts.push(path);
                }
            }
        }
    }

    // 2. Scan /proc/mounts for vfat / exfat mounts
    if let Ok(mounts_data) = fs::read_to_string("/proc/mounts") {
        for line in mounts_data.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let mount_point = parts[1];
                let fs_type = parts[2];
                if fs_type == "vfat" || fs_type == "exfat" {
                    let path = PathBuf::from(mount_point);
                    if path.is_dir() && !candidate_mounts.contains(&path) {
                        candidate_mounts.push(path);
                    }
                }
            }
        }
    }

    // 3. Inspect candidates: check for ROLAND/IMPORT or IMPORT directory
    for mount in &candidate_mounts {
        let import_path = resolve_import_path(mount);
        if import_path.is_dir() {
            let device_node = find_device_for_mount_point(mount);
            if let Some(info) = build_sd_info(mount, &import_path, device_node.as_deref()) {
                return Some(info);
            }
        }
    }

    // Fallback: Check if any candidate has an SP-404 related label
    for mount in candidate_mounts {
        let name = mount
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_uppercase();
        if name.contains("SP404") || name.contains("SP-404") || name.contains("ROLAND") {
            let import_path = resolve_import_path(&mount);
            let device_node = find_device_for_mount_point(&mount);
            if let Some(info) = build_sd_info(&mount, &import_path, device_node.as_deref()) {
                return Some(info);
            }
        }
    }

    None
}

/// Detects an inserted SP-404MKII SD card mounted on the Linux system.
pub fn detect_sd_card() -> Option<SdCardInfo> {
    detect_sd_card_full().card
}

/// Gathers capacity stats, read-only status, and existing kit names for a mount point.
pub fn build_sd_info(
    mount: &Path,
    import_path: &Path,
    device_node: Option<&str>,
) -> Option<SdCardInfo> {
    if !mount.is_dir() {
        return None;
    }

    let capacity = get_filesystem_capacity(mount).unwrap_or_default();

    let label = mount
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("SD Card")
        .to_string();

    let mut existing_kits = Vec::new();
    if import_path.is_dir()
        && let Ok(entries) = fs::read_dir(import_path)
    {
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                existing_kits.push(name.to_string());
            }
        }
    }
    existing_kits.sort();

    Some(SdCardInfo {
        mount_path: mount.to_path_buf(),
        import_path: import_path.to_path_buf(),
        label,
        device_node: device_node.map(str::to_string),
        free_bytes: capacity.free_bytes,
        total_bytes: capacity.total_bytes,
        is_read_only: capacity.is_read_only,
        existing_kits,
    })
}

/// Reads free bytes, total bytes, and read-only status for a mounted filesystem via POSIX statvfs.
fn get_filesystem_capacity(path: &Path) -> Result<FilesystemCapacity, StorageError> {
    let c_path = CString::new(path.as_os_str().as_bytes())?;

    unsafe {
        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
            let stat = stat.assume_init();
            let block_size = stat.f_frsize;
            let free_bytes = stat.f_bavail * block_size;
            let total_bytes = stat.f_blocks * block_size;
            let is_read_only = (stat.f_flag & (libc::ST_RDONLY as libc::c_ulong)) != 0;
            Ok(FilesystemCapacity {
                free_bytes,
                total_bytes,
                is_read_only,
            })
        } else {
            Err(StorageError::StatvfsFailed)
        }
    }
}

/// Flushes kernel write buffers and safely unmounts the SD card partition via udisksctl or umount.
pub fn safe_eject(mount_path: &Path, device_node: Option<&str>) -> Result<(), StorageError> {
    info!(
        "Flushing write buffers to SD card at {}...",
        mount_path.display()
    );

    // 1. Flush kernel filesystem write buffers
    unsafe {
        libc::sync();
    }

    // 2. Resolve block device
    let dev = device_node
        .map(|s| s.to_string())
        .or_else(|| find_device_for_mount_point(mount_path));

    if let Some(ref dev_node) = dev {
        info!("Unmounting block device {} via udisksctl...", dev_node);
        let udisks_res = Command::new("udisksctl")
            .args(["unmount", "-b", dev_node, "--no-user-interaction"])
            .output();

        if let Ok(output) = udisks_res
            && output.status.success()
        {
            info!("Successfully unmounted SD card via udisksctl.");
            return Ok(());
        }
    }

    // 3. Fallback: try standard umount
    let output = Command::new("umount").arg(mount_path).output()?;

    if output.status.success() {
        info!("Successfully unmounted SD card via umount.");
        Ok(())
    } else {
        let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(StorageError::UnmountFailed(err_msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filesystem_capacity_root() {
        let cap = get_filesystem_capacity(Path::new("/")).expect("Root should have capacity");
        assert!(cap.total_bytes > 0, "Root total bytes should be non-zero");
        assert!(cap.free_bytes > 0, "Root free bytes should be non-zero");
    }

    #[test]
    fn test_parse_lsblk_line() {
        let line = r#"NAME="sdc1" MOUNTPOINT="" RM="1" HOTPLUG="1" FSTYPE="vfat" LABEL="SP404MKII" SIZE="59.6G""#;
        let map = parse_lsblk_line(line);
        assert_eq!(map.get("NAME").map(|s| s.as_str()), Some("sdc1"));
        assert_eq!(map.get("MOUNTPOINT").map(|s| s.as_str()), Some(""));
        assert_eq!(map.get("RM").map(|s| s.as_str()), Some("1"));
        assert_eq!(map.get("FSTYPE").map(|s| s.as_str()), Some("vfat"));
        assert_eq!(map.get("LABEL").map(|s| s.as_str()), Some("SP404MKII"));
        assert_eq!(map.get("SIZE").map(|s| s.as_str()), Some("59.6G"));
    }

    #[test]
    fn test_resolve_import_path() {
        let tmp = std::env::temp_dir().join("test_sp404_import_resolve");
        let _ = fs::create_dir_all(&tmp);

        // Without any folders, defaults to ROLAND/IMPORT
        let p = resolve_import_path(&tmp);
        assert_eq!(p, tmp.join("ROLAND").join("IMPORT"));

        // With IMPORT on root
        let root_imp = tmp.join("IMPORT");
        let _ = fs::create_dir_all(&root_imp);
        assert_eq!(resolve_import_path(&tmp), root_imp);

        // With ROLAND/IMPORT
        let roland_imp = tmp.join("ROLAND").join("IMPORT");
        let _ = fs::create_dir_all(&roland_imp);
        assert_eq!(resolve_import_path(&tmp), roland_imp);

        let _ = fs::remove_dir_all(&tmp);
    }
}
