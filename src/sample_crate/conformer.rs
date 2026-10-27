//! Audio conforming and conversion engine.
//!
//! Converts arbitrary audio files (MP3, FLAC, WAV, AAC, Opus, etc.) to
//! SP-404MKII hardware-native 48kHz, 24-bit linear PCM WAV.
//!
//! Preserves mono (1-channel) to save hardware polyphony, and stereo (2-channel).
//! Applies optional peak normalization and sanitizes filenames for the 128x64 OLED display.

use std::fs::{self, File};
use std::path::Path;

use hound::{WavSpec, WavWriter};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use thiserror::Error;
use tracing::{debug, info, warn};

use super::model::{ConformOptions, StagedSample};

pub const SP404_SAMPLE_RATE: u32 = 48000;
pub const WAVEFORM_BINS: usize = 512;

#[derive(Debug, Error)]
pub enum ConformerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Audio decode error: {0}")]
    Decode(String),
    #[error("Unsupported audio format or no audio track found")]
    NoAudioTrack,
    #[error("Resampling error: {0}")]
    Resample(String),
    #[error("WAV write error: {0}")]
    WavWrite(#[from] hound::Error),
}

/// Decoded raw audio buffer ready for processing or preview.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub channels: usize,
    pub sample_rate: u32,
    /// Channel-planar audio samples: `samples[channel][frame_idx]`
    pub samples: Vec<Vec<f32>>,
}

impl DecodedAudio {
    /// Total duration in seconds
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 || self.samples.is_empty() {
            0.0
        } else {
            self.samples[0].len() as f32 / self.sample_rate as f32
        }
    }

    /// Peak absolute amplitude across all channels (0.0 to 1.0+)
    pub fn peak_amplitude(&self) -> f32 {
        let mut max_val: f32 = 0.0;
        for ch in &self.samples {
            for &s in ch {
                let abs = s.abs();
                if abs > max_val {
                    max_val = abs;
                }
            }
        }
        max_val
    }

    /// Peak level in dBFS
    pub fn peak_db(&self) -> f32 {
        let peak = self.peak_amplitude();
        if peak <= 1e-5 {
            -100.0
        } else {
            20.0 * peak.log10()
        }
    }

    /// Generates a normalized 512-bin high-resolution waveform overview (0.0 to 1.0)
    pub fn generate_waveform_overview(&self) -> Vec<f32> {
        let n_frames = if self.samples.is_empty() {
            0
        } else {
            self.samples[0].len()
        };
        if n_frames == 0 {
            return vec![0.0; WAVEFORM_BINS];
        }

        let mut overview = vec![0.0f32; WAVEFORM_BINS];
        let chunk_size = (n_frames as f32 / WAVEFORM_BINS as f32).max(1.0);

        for (bin, item) in overview.iter_mut().enumerate() {
            let start = (bin as f32 * chunk_size) as usize;
            let end = (((bin + 1) as f32 * chunk_size) as usize).min(n_frames);

            let mut bin_max = 0.0f32;
            for ch in &self.samples {
                for &sample in &ch[start..end] {
                    let abs = sample.abs();
                    if abs > bin_max {
                        bin_max = abs;
                    }
                }
            }
            *item = bin_max.min(1.0);
        }

        overview
    }

    /// Resamples audio to 48,000 Hz if needed.
    pub fn resample_to_48k(&mut self) -> Result<(), ConformerError> {
        if self.sample_rate == SP404_SAMPLE_RATE || self.samples.is_empty() {
            self.sample_rate = SP404_SAMPLE_RATE;
            return Ok(());
        }

        let n_channels = self.channels;
        let in_rate = self.sample_rate as usize;
        let out_rate = SP404_SAMPLE_RATE as usize;

        debug!(
            "Resampling audio from {} Hz to {} Hz (channels: {})",
            in_rate, out_rate, n_channels
        );

        let mut resampler = FastFixedIn::<f32>::new(
            out_rate as f64 / in_rate as f64,
            1.0,
            PolynomialDegree::Cubic,
            1024,
            n_channels,
        )
        .map_err(|e| ConformerError::Resample(format!("Failed to create resampler: {e}")))?;

        let mut resampled_channels = vec![Vec::new(); n_channels];
        let chunk_size = resampler.input_frames_next();
        let total_frames = self.samples[0].len();
        let mut offset = 0;

        while offset < total_frames {
            let frames_this_chunk = (total_frames - offset).min(chunk_size);
            let mut input_chunk = vec![vec![0.0f32; chunk_size]; n_channels];

            for (ch, chunk) in input_chunk.iter_mut().enumerate() {
                chunk[..frames_this_chunk]
                    .copy_from_slice(&self.samples[ch][offset..offset + frames_this_chunk]);
            }

            let out_chunk = resampler
                .process(&input_chunk, None)
                .map_err(|e| ConformerError::Resample(format!("Resampling failed: {e}")))?;

            let out_frames = if offset + chunk_size >= total_frames {
                // Last chunk: compute proportional frames
                let ratio = out_rate as f64 / in_rate as f64;
                (frames_this_chunk as f64 * ratio).round() as usize
            } else {
                out_chunk[0].len()
            };

            for ch in 0..n_channels {
                let valid_out = out_frames.min(out_chunk[ch].len());
                resampled_channels[ch].extend_from_slice(&out_chunk[ch][..valid_out]);
            }

            offset += chunk_size;
        }

        self.samples = resampled_channels;
        self.sample_rate = SP404_SAMPLE_RATE;
        Ok(())
    }

    /// Normalizes peak amplitude to a target dBFS ceiling (e.g. -0.5 dBFS)
    pub fn normalize_peak(&mut self, ceiling_db: f32) {
        let current_peak = self.peak_amplitude();
        if current_peak <= 1e-5 {
            return;
        }

        let target_linear = 10.0f32.powf(ceiling_db / 20.0);
        let gain = target_linear / current_peak;

        for ch in &mut self.samples {
            for s in ch {
                *s *= gain;
            }
        }
    }
}

/// Decodes any supported audio file into planar f32 memory buffers.
pub fn decode_audio_file(path: &Path) -> Result<DecodedAudio, ConformerError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| ConformerError::Decode(format!("Probe failed for {}: {e}", path.display())))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(ConformerError::NoAudioTrack)?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(SP404_SAMPLE_RATE);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2)
        .min(2); // SP-404 supports mono (1) or stereo (2)

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| ConformerError::Decode(format!("Codec creation failed: {e}")))?;

    let mut channel_samples: Vec<Vec<f32>> = vec![Vec::new(); channels];
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(ref err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => continue,
            Err(err) => {
                debug!("End of stream reached or packet error: {:?}", err);
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let spec = *decoded.spec();
                    let duration = decoded.capacity() as u64;
                    sample_buf = Some(SampleBuffer::new(duration, spec));
                }

                if let Some(buf) = sample_buf.as_mut() {
                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();
                    let n_frames = samples.len() / channels;

                    for frame in 0..n_frames {
                        for ch in 0..channels {
                            channel_samples[ch].push(samples[frame * channels + ch]);
                        }
                    }
                }
            }
            Err(SymphoniaError::IoError(ref err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::DecodeError(err)) => {
                warn!("Decode frame error: {}", err);
                continue;
            }
            Err(err) => {
                return Err(ConformerError::Decode(format!("Fatal decode error: {err}")));
            }
        }
    }

    Ok(DecodedAudio {
        channels,
        sample_rate,
        samples: channel_samples,
    })
}

/// Analyzes an audio file and creates a `StagedSample` with waveform and peak metadata.
pub fn stage_audio_file(
    id: u64,
    path: &Path,
    max_name_len: usize,
) -> Result<StagedSample, ConformerError> {
    let audio = decode_audio_file(path)?;
    let clean_name = sanitize_filename(path, max_name_len);
    let duration_secs = audio.duration_secs();
    let peak_db = audio.peak_db();
    let waveform_overview = audio.generate_waveform_overview();
    let is_mono = audio.channels == 1;

    Ok(StagedSample {
        id,
        source_path: path.to_path_buf(),
        clean_name,
        sample_rate: audio.sample_rate,
        channels: audio.channels as u16,
        duration_secs,
        peak_db,
        waveform_overview,
        is_mono,
    })
}

/// Sanitizes a filename for FAT32 and the SP-404MKII OLED 14-16 character limit.
pub fn sanitize_filename(path: &Path, max_len: usize) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sample");

    // Replace illegal FAT32/exFAT characters with underscores
    let mut cleaned: String = stem
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect();

    cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        cleaned = "sample".to_string();
    }

    // Truncate to max_len (preserving valid UTF-8 boundary)
    if cleaned.chars().count() > max_len {
        cleaned = cleaned.chars().take(max_len).collect();
        cleaned = cleaned.trim_end().to_string();
    }

    cleaned
}

/// Writes processed audio to a 24-bit linear PCM WAV file.
///
/// Ensures mono samples are written as 1-channel mono WAV (saving polyphony)
/// and stereo samples as 2-channel WAV. Flushes to disk via `sync_all()`.
pub fn export_conformed_wav(
    audio: &DecodedAudio,
    dest_path: &Path,
    options: &ConformOptions,
) -> Result<(), ConformerError> {
    let mut processed = audio.clone();

    // 1. Resample to 48kHz
    processed.resample_to_48k()?;

    // 2. Apply Peak Normalization if enabled
    if options.normalize_peak {
        processed.normalize_peak(options.peak_ceiling_db);
    }

    let channels = processed.channels as u16;
    let n_frames = if processed.samples.is_empty() {
        0
    } else {
        processed.samples[0].len()
    };

    let spec = WavSpec {
        channels,
        sample_rate: SP404_SAMPLE_RATE,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write to a temporary file first for atomic export safety
    let tmp_path = dest_path.with_extension("wav.tmp");
    {
        let mut writer = WavWriter::create(&tmp_path, spec)?;

        // 24-bit signed integer range: -8,388,608 to 8,388,607
        const MAX_24BIT: f32 = 8_388_607.0;

        for frame in 0..n_frames {
            for ch in 0..processed.channels {
                let sample = processed.samples[ch][frame].clamp(-1.0, 1.0);
                let val_24bit = (sample * MAX_24BIT).round() as i32;
                writer.write_sample(val_24bit)?;
            }
        }

        writer.flush()?;
    }

    // Rename temp file to final target
    fs::rename(&tmp_path, dest_path)?;

    // Sync filesystem to prevent FAT32/exFAT corruption on SD card pull
    if let Ok(file) = File::open(dest_path) {
        let _ = file.sync_all();
    }
    if let Some(parent) = dest_path.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }

    info!(
        "Successfully exported conformed 48kHz/24-bit WAV to {}",
        dest_path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(
            sanitize_filename(Path::new("normal_kick.wav"), 16),
            "normal_kick"
        );
        assert_eq!(
            sanitize_filename(Path::new("bad:file_name*test?.mp3"), 16),
            "bad_file_name_te"
        );
        assert_eq!(
            sanitize_filename(Path::new("ThisIsAVeryLongFileNameThatWillTruncate.wav"), 14),
            "ThisIsAVeryLon"
        );
    }

    #[test]
    fn test_export_conformed_wav_mono_and_stereo() {
        let temp_dir = std::env::temp_dir().join("sp404_conformer_test");
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Mono test
        let mono_audio = DecodedAudio {
            channels: 1,
            sample_rate: 44100,             // Non-48k to test resampling
            samples: vec![vec![0.3; 4410]], // 0.1s
        };
        let mono_dest = temp_dir.join("test_mono.wav");
        let opts = ConformOptions {
            normalize_peak: true,
            peak_ceiling_db: -0.5,
            ..Default::default()
        };
        export_conformed_wav(&mono_audio, &mono_dest, &opts).unwrap();

        let reader = hound::WavReader::open(&mono_dest).unwrap();
        let spec = reader.spec();
        assert_eq!(
            spec.channels, 1,
            "Mono input must produce 1-channel mono WAV"
        );
        assert_eq!(spec.sample_rate, 48000, "Sample rate must be 48kHz");
        assert_eq!(spec.bits_per_sample, 24, "Bit depth must be 24-bit");

        // 2. Stereo test
        let stereo_audio = DecodedAudio {
            channels: 2,
            sample_rate: 48000,
            samples: vec![vec![0.2; 4800], vec![-0.4; 4800]],
        };
        let stereo_dest = temp_dir.join("test_stereo.wav");
        export_conformed_wav(&stereo_audio, &stereo_dest, &opts).unwrap();

        let reader_st = hound::WavReader::open(&stereo_dest).unwrap();
        let spec_st = reader_st.spec();
        assert_eq!(
            spec_st.channels, 2,
            "Stereo input must produce 2-channel stereo WAV"
        );
        assert_eq!(spec_st.sample_rate, 48000);
        assert_eq!(spec_st.bits_per_sample, 24);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_normalize_peak() {
        let mut audio = DecodedAudio {
            channels: 1,
            sample_rate: 48000,
            samples: vec![vec![0.25, -0.5, 0.1, -0.2]],
        };

        // Peak is 0.5. Normalize to 0.0 dBFS (1.0 linear)
        audio.normalize_peak(0.0);
        let peak = audio.peak_amplitude();
        assert!((peak - 1.0).abs() < 1e-4);
        assert!((audio.samples[0][1] - (-1.0)).abs() < 1e-4);
    }

    #[test]
    fn test_generate_waveform_overview() {
        let audio = DecodedAudio {
            channels: 1,
            sample_rate: 48000,
            samples: vec![vec![0.5; 1000]],
        };

        let overview = audio.generate_waveform_overview();
        assert_eq!(overview.len(), WAVEFORM_BINS);
        for val in overview {
            assert!((val - 0.5).abs() < 1e-4);
        }
    }

    #[test]
    fn test_end_to_end_stage_and_export() {
        let temp_dir = std::env::temp_dir().join("sp404_stage_export_e2e");
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Create a dummy input WAV (44.1kHz, 16-bit mono)
        let input_path = temp_dir.join("Dirty_Vinyl_Kick_01.wav");
        let spec_in = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&input_path, spec_in).unwrap();
            for i in 0..4410 {
                let sample = ((i as f32 / 441.0).sin() * 16000.0) as i16;
                writer.write_sample(sample).unwrap();
            }
            writer.flush().unwrap();
        }

        // 2. Stage the audio file
        let staged = stage_audio_file(1, &input_path, 16).expect("Staging should succeed");
        assert_eq!(staged.clean_name, "Dirty_Vinyl_Kick");
        assert_eq!(staged.channels, 1);
        assert!(staged.is_mono);
        assert_eq!(staged.waveform_overview.len(), WAVEFORM_BINS);

        // 3. Export to a mock SD card
        let mock_sd = temp_dir.join("mock_sd");
        let mock_import = mock_sd.join("IMPORT").join("My_Kit");
        fs::create_dir_all(&mock_import).unwrap();

        let decoded = decode_audio_file(&input_path).unwrap();
        let export_dest = mock_import.join(format!("{}.wav", staged.clean_name));
        let opts = ConformOptions {
            normalize_peak: true,
            peak_ceiling_db: -0.5,
            ..Default::default()
        };
        export_conformed_wav(&decoded, &export_dest, &opts).unwrap();

        // 4. Verify output file
        assert!(export_dest.is_file());
        let reader = hound::WavReader::open(&export_dest).unwrap();
        let spec_out = reader.spec();
        assert_eq!(spec_out.channels, 1, "Mono input must stay mono");
        assert_eq!(spec_out.sample_rate, 48000, "Output must be 48kHz");
        assert_eq!(spec_out.bits_per_sample, 24, "Output must be 24-bit PCM");

        // 5. Verify SD detection sees the new kit
        let sd_info = crate::sample_crate::sd_detector::build_sd_info(
            &mock_sd,
            &mock_sd.join("IMPORT"),
            None,
        )
        .unwrap();
        assert!(sd_info.existing_kits.contains(&"My_Kit".to_string()));

        let _ = fs::remove_dir_all(temp_dir);
    }
}
