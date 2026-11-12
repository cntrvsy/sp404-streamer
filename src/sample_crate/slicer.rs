//! Transient onset detection and loop auto-chopping into individual pad WAVs.
//!
//! Exports numbered slices (`01_name.wav` .. `16_name.wav`) into `IMPORT/<Kit_Name>/`
//! with automatic micro-fades on slice boundaries. Note that the user still assigns
//! exported slices to pads using the SP-404MKII's on-device import menu (`SHIFT + PAD 14`).

use std::fs;
use std::path::{Path, PathBuf};

use super::conformer::{ConformerError, DecodedAudio, export_conformed_wav};
use super::model::ConformOptions;

pub const MAX_PAD_SLICES: usize = 16;

/// Slicing strategy for chopping a loop or break.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChopMode {
    /// Energy-based transient onset detection (`sensitivity` in `0.0..=1.0`)
    Transient { sensitivity: f32 },
    /// Equal-length musical grid division (e.g. 4, 8, or 16 equal slices)
    EvenGrid,
}

impl Default for ChopMode {
    fn default() -> Self {
        Self::Transient { sensitivity: 0.65 }
    }
}

/// Computes frame indices (`0..n_frames`) where slices begin, capped at `max_slices` (1..=16).
pub fn compute_slice_frames(audio: &DecodedAudio, max_slices: usize, mode: ChopMode) -> Vec<usize> {
    let n_frames = audio.samples.first().map_or(0, Vec::len);
    let target_slices = max_slices.clamp(1, MAX_PAD_SLICES);
    if n_frames == 0 || target_slices <= 1 {
        return vec![0];
    }

    match mode {
        ChopMode::EvenGrid => compute_even_grid(n_frames, target_slices),
        ChopMode::Transient { sensitivity } => {
            let mut slices =
                compute_transient_onsets(audio, target_slices, sensitivity.clamp(0.05, 0.99));
            if slices.len() < 2 {
                // Fallback to even grid if the audio is a smooth pad/drone with no sharp transients
                slices = compute_even_grid(n_frames, target_slices);
            }
            slices
        }
    }
}

fn compute_even_grid(n_frames: usize, target_slices: usize) -> Vec<usize> {
    let step = (n_frames / target_slices).max(1);
    (0..target_slices)
        .map(|i| (i * step).min(n_frames.saturating_sub(1)))
        .collect()
}

/// Detects percussive transients using short-window energy flux.
fn compute_transient_onsets(
    audio: &DecodedAudio,
    max_slices: usize,
    sensitivity: f32,
) -> Vec<usize> {
    let n_frames = audio.samples.first().map_or(0, Vec::len);
    let sr = audio.sample_rate.max(8000) as usize;

    // 10ms energy analysis hop
    let hop = (sr / 100).max(64);
    let n_windows = n_frames / hop;
    if n_windows < 4 {
        return vec![0];
    }

    let mut energies = Vec::with_capacity(n_windows);
    for w in 0..n_windows {
        let start = w * hop;
        let end = (start + hop).min(n_frames);
        let mut sum_sq = 0.0f32;
        for ch in &audio.samples {
            for &s in &ch[start..end] {
                sum_sq += s * s;
            }
        }
        energies.push((sum_sq / ((end - start) * audio.channels.max(1)) as f32).sqrt());
    }

    // Positive first-order energy flux (onset strength)
    let mut flux = vec![0.0f32; n_windows];
    let mut max_flux = 0.0f32;
    for w in 1..n_windows {
        let diff = (energies[w] - energies[w - 1]).max(0.0);
        flux[w] = diff;
        if diff > max_flux {
            max_flux = diff;
        }
    }

    if max_flux <= 1e-6 {
        return vec![0];
    }

    // Lower threshold when sensitivity is higher
    let threshold = max_flux * (1.0 - sensitivity * 0.88).clamp(0.05, 0.90);
    // Minimum spacing between slices: ~80ms (8 hops)
    let min_hop_dist = 8usize;

    // Candidate local peaks `(window_idx, flux_val)`
    let mut candidates: Vec<(usize, f32)> = Vec::new();
    for w in 2..(n_windows - 1) {
        if flux[w] >= threshold && flux[w] >= flux[w - 1] && flux[w] >= flux[w + 1] {
            candidates.push((w, flux[w]));
        }
    }

    // Sort strongest transients first
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut chosen_windows: Vec<usize> = vec![0];
    for (win_idx, _) in candidates {
        if chosen_windows.len() >= max_slices {
            break;
        }
        if chosen_windows
            .iter()
            .all(|&existing| existing.abs_diff(win_idx) >= min_hop_dist)
        {
            chosen_windows.push(win_idx);
        }
    }

    chosen_windows.sort_unstable();
    chosen_windows
        .into_iter()
        .map(|w| (w * hop).min(n_frames.saturating_sub(1)))
        .collect()
}

/// Converts frame indices into normalized `0.0..=1.0` ratios for UI waveform markers.
pub fn slice_frames_to_ratios(audio: &DecodedAudio, slice_frames: &[usize]) -> Vec<f32> {
    let n_frames = audio.samples.first().map_or(0, Vec::len);
    if n_frames == 0 {
        return Vec::new();
    }
    slice_frames
        .iter()
        .map(|&f| (f as f32 / n_frames as f32).clamp(0.0, 1.0))
        .collect()
}

/// Exports each slice `[slice_frames[i] .. slice_frames[i+1]]` as a numbered 16-bit/48kHz WAV file
/// (`01_<base_name>.wav` .. `16_<base_name>.wav`) in `dest_dir`.
pub fn export_chopped_slices(
    audio: &DecodedAudio,
    slice_frames: &[usize],
    base_name: &str,
    dest_dir: &Path,
    options: &ConformOptions,
) -> Result<Vec<PathBuf>, ConformerError> {
    let n_frames = audio.samples.first().map_or(0, Vec::len);
    if n_frames == 0 || slice_frames.is_empty() {
        return Ok(Vec::new());
    }

    fs::create_dir_all(dest_dir)?;

    // Ensure slice boundaries have micro-fades enabled to prevent clicks at chop points
    let mut slice_opts = options.clone();
    slice_opts.micro_fades = true;
    slice_opts.trim_start_silence = false; // Keep exact slice start timing

    let max_base_len = options.max_name_len.saturating_sub(3).max(4);
    let trimmed_base: String = base_name.chars().take(max_base_len).collect();

    let mut written_paths = Vec::with_capacity(slice_frames.len());
    for (idx, &start_frame) in slice_frames.iter().take(MAX_PAD_SLICES).enumerate() {
        let end_frame = slice_frames
            .get(idx + 1)
            .copied()
            .unwrap_or(n_frames)
            .max(start_frame + 1)
            .min(n_frames);

        let sub_audio = audio.slice_range(start_frame, end_frame);
        let filename = format!("{:02}_{}.wav", idx + 1, trimmed_base);
        let dest_path = dest_dir.join(filename);
        export_conformed_wav(&sub_audio, &dest_path, &slice_opts)?;
        written_paths.push(dest_path);
    }

    Ok(written_paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_even_grid_chopping() {
        let audio = DecodedAudio {
            channels: 1,
            sample_rate: 48000,
            samples: vec![vec![0.25; 48000]], // 1.0 sec
        };

        let slices = compute_slice_frames(&audio, 8, ChopMode::EvenGrid);
        assert_eq!(slices.len(), 8);
        assert_eq!(slices[0], 0);
        assert_eq!(slices[1], 6000);
    }

    #[test]
    fn test_transient_onset_detection_and_export() {
        let sr = 48000;
        let mut buf = vec![0.0f32; sr * 2]; // 2.0s buffer
        // Inject 4 distinct drum hits at 0.0s, 0.5s, 1.0s, 1.5s
        for &hit_sec in &[0.0f32, 0.5, 1.0, 1.5] {
            let start = (hit_sec * sr as f32) as usize;
            for i in 0..1200 {
                let env = 1.0 - (i as f32 / 1200.0);
                buf[start + i] = ((i as f32 * 0.25).sin()) * env * 0.9;
            }
        }

        let audio = DecodedAudio {
            channels: 1,
            sample_rate: sr as u32,
            samples: vec![buf],
        };

        let slices = compute_slice_frames(&audio, 4, ChopMode::Transient { sensitivity: 0.7 });
        assert_eq!(slices.len(), 4, "Should detect all 4 synthetic drum hits");

        let temp_dir = std::env::temp_dir().join("sp404_slicer_test");
        let _ = fs::remove_dir_all(&temp_dir);
        let opts = ConformOptions::default();
        let exported =
            export_chopped_slices(&audio, &slices, "Amen_Break", &temp_dir, &opts).unwrap();
        assert_eq!(exported.len(), 4);
        assert!(exported[0].ends_with("01_Amen_Break.wav"));
        assert!(exported[3].ends_with("04_Amen_Break.wav"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
