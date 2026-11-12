//! SP-404MKII Custom Startup & Screensaver OLED Studio (`128 × 64` 1-bit Monochrome BMP).
//!
//! Converts animated GIFs, static images (`.png`, `.jpg`, `.webp`, `.bmp`), or built-in
//! procedural animations into hardware-compliant `128 × 64` 1-bit monochrome Windows BMPs
//! (`startup_1.bmp`..`startup_2.bmp` and `screen_saver_1.bmp`..`screen_saver_16.bmp`) inside
//! `EXPORT/PROJECT/PROJECT_XX/PICTURE/`.

use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{AnimationDecoder, DynamicImage, GenericImageView, ImageFormat, imageops::FilterType};
use strum::{Display, EnumIter, IntoStaticStr};
use thiserror::Error;
use tracing::info;

pub const OLED_WIDTH: usize = 128;
pub const OLED_HEIGHT: usize = 64;
pub const OLED_PIXELS: usize = OLED_WIDTH * OLED_HEIGHT;
pub const MAX_SCREENSAVER_FRAMES: usize = 16;
pub const MAX_STARTUP_FRAMES: usize = 2;
/// Exact byte size of a 128×64 1-bit Windows v3 BITMAPINFOHEADER BMP file:
/// 14 (File Header) + 40 (Info Header) + 8 (2-Color Palette) + 1024 (64 rows × 16 bytes) = 1086 bytes.
pub const SP404_BMP_BYTE_SIZE: usize = 1086;

#[derive(Debug, Error)]
pub enum OledError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image decode error: {0}")]
    Image(#[from] image::ImageError),
    #[error("No frames found in source image")]
    EmptySource,
}

/// Dithering algorithm used to convert 8-bit grayscale (`0..=255`) into 1-bit OLED pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum DitherMode {
    #[strum(to_string = "Floyd-Steinberg (Smooth)")]
    FloydSteinberg,
    #[strum(to_string = "Bayer 4×4 (Retro Grid)")]
    Bayer4x4,
    #[strum(to_string = "Hard Threshold (Crisp Logo)")]
    Threshold,
}

/// Which set of hardware BMP files to write into `EXPORT/PROJECT/PROJECT_XX/PICTURE/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum OledExportTarget {
    #[strum(to_string = "Screensaver + Startup (All 18 BMPs)")]
    Both,
    #[strum(to_string = "Screensaver Only (16 Frames)")]
    ScreensaverOnly,
    #[strum(to_string = "Startup Screen Only (2 Frames)")]
    StartupOnly,
}

/// Built-in procedural animations so Linux users can preview and sync animations immediately
/// without needing an external `.gif` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, IntoStaticStr)]
pub enum DemoPreset {
    #[strum(to_string = "Cassette Tape Reels")]
    TapeReels,
    #[strum(to_string = "Harmonic Oscilloscope")]
    Oscilloscope,
    #[strum(to_string = "SP-404 Beat Spectrum")]
    BeatSpectrum,
}

/// Real-time image adjustment and dithering parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct OledAdjustments {
    pub dither_mode: DitherMode,
    /// Threshold cutoff (`1..=254`, default `128`)
    pub threshold: u8,
    /// Contrast boost (`-0.8..=0.8`, default `0.15`)
    pub contrast: f32,
    /// Brightness offset (`-0.8..=0.8`, default `0.0`)
    pub brightness: f32,
    /// Invert black and white pixels
    pub invert: bool,
}

impl Default for OledAdjustments {
    fn default() -> Self {
        Self {
            dither_mode: DitherMode::FloydSteinberg,
            threshold: 128,
            contrast: 0.15,
            brightness: 0.0,
            invert: false,
        }
    }
}

/// A single `128 × 64` grayscale source frame (`0..=255` luminance per pixel).
#[derive(Debug, Clone)]
pub struct GrayFrame {
    pub luma: [u8; OLED_PIXELS],
}

impl Default for GrayFrame {
    fn default() -> Self {
        Self {
            luma: [0u8; OLED_PIXELS],
        }
    }
}

/// A quantized `128 × 64` 1-bit monochrome OLED frame (`true` = White/Lit, `false` = Black/Off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoFrame {
    pub pixels: [bool; OLED_PIXELS],
}

impl Default for MonoFrame {
    fn default() -> Self {
        Self {
            pixels: [false; OLED_PIXELS],
        }
    }
}

impl MonoFrame {
    #[inline]
    pub const fn get(&self, x: usize, y: usize) -> bool {
        if x < OLED_WIDTH && y < OLED_HEIGHT {
            self.pixels[y * OLED_WIDTH + x]
        } else {
            false
        }
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        if x < OLED_WIDTH && y < OLED_HEIGHT {
            self.pixels[y * OLED_WIDTH + x] = on;
        }
    }
}

/// Converts a grayscale `128 × 64` frame into a 1-bit `MonoFrame` using the active `OledAdjustments`.
pub fn dither_frame(source: &GrayFrame, adj: &OledAdjustments) -> MonoFrame {
    // 1. Apply brightness and contrast in float space [0.0, 255.0]
    let mut working = [0.0f32; OLED_PIXELS];
    let contrast_factor = (1.0 + adj.contrast).clamp(0.1, 3.0);
    let brightness_offset = adj.brightness * 128.0;

    for (i, &raw) in source.luma.iter().enumerate() {
        let centered = (raw as f32 - 128.0) * contrast_factor + 128.0 + brightness_offset;
        let mut val = centered.clamp(0.0, 255.0);
        if adj.invert {
            val = 255.0 - val;
        }
        working[i] = val;
    }

    let mut out = MonoFrame::default();
    let thresh = adj.threshold as f32;

    match adj.dither_mode {
        DitherMode::Threshold => {
            for (dst, &val) in out.pixels.iter_mut().zip(working.iter()) {
                *dst = val >= thresh;
            }
        }
        DitherMode::Bayer4x4 => {
            const BAYER_4X4: [[f32; 4]; 4] = [
                [0.0, 8.0, 2.0, 10.0],
                [12.0, 4.0, 14.0, 6.0],
                [3.0, 11.0, 1.0, 9.0],
                [15.0, 7.0, 13.0, 5.0],
            ];
            for y in 0..OLED_HEIGHT {
                for x in 0..OLED_WIDTH {
                    let bayer_offset = (BAYER_4X4[y & 3][x & 3] / 16.0 - 0.5) * 64.0;
                    let v = working[y * OLED_WIDTH + x] + bayer_offset;
                    out.set(x, y, v >= thresh);
                }
            }
        }
        DitherMode::FloydSteinberg => {
            for y in 0..OLED_HEIGHT {
                for x in 0..OLED_WIDTH {
                    let idx = y * OLED_WIDTH + x;
                    let old_px = working[idx];
                    let is_on = old_px >= thresh;
                    let new_px = if is_on { 255.0 } else { 0.0 };
                    out.pixels[idx] = is_on;
                    let err = old_px - new_px;

                    if x + 1 < OLED_WIDTH {
                        working[idx + 1] += err * (7.0 / 16.0);
                    }
                    if y + 1 < OLED_HEIGHT {
                        let next_row = (y + 1) * OLED_WIDTH;
                        if x > 0 {
                            working[next_row + x - 1] += err * (3.0 / 16.0);
                        }
                        working[next_row + x] += err * (5.0 / 16.0);
                        if x + 1 < OLED_WIDTH {
                            working[next_row + x + 1] += err * (1.0 / 16.0);
                        }
                    }
                }
            }
        }
    }

    out
}

/// Crops (center-fit 2:1 aspect ratio) and resizes any `DynamicImage` into a `128 × 64` `GrayFrame`.
pub fn image_to_gray_frame(img: &DynamicImage) -> GrayFrame {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return GrayFrame::default();
    }

    // Center-crop to 2:1 aspect ratio before scaling to 128x64 so graphics aren't stretched
    let target_ratio = OLED_WIDTH as f32 / OLED_HEIGHT as f32;
    let current_ratio = w as f32 / h as f32;

    let cropped = if (current_ratio - target_ratio).abs() > 0.05 {
        if current_ratio > target_ratio {
            let new_w = ((h as f32 * target_ratio) as u32).clamp(1, w);
            let x_off = (w - new_w) / 2;
            img.crop_imm(x_off, 0, new_w, h)
        } else {
            let new_h = ((w as f32 / target_ratio) as u32).clamp(1, h);
            let y_off = (h - new_h) / 2;
            img.crop_imm(0, y_off, w, new_h)
        }
    } else {
        img.clone()
    };

    let gray = cropped
        .resize_exact(OLED_WIDTH as u32, OLED_HEIGHT as u32, FilterType::Lanczos3)
        .to_luma8();

    let mut frame = GrayFrame::default();
    for (i, px) in gray.pixels().enumerate().take(OLED_PIXELS) {
        frame.luma[i] = px.0[0];
    }
    frame
}

/// Decodes an image or animated GIF file into 16 `GrayFrame`s ready for real-time dithering.
pub fn load_frames_from_path(path: &Path) -> Result<Vec<GrayFrame>, OledError> {
    let bytes = fs::read(path)?;
    load_frames_from_bytes(&bytes)
}

/// Decodes raw bytes of a `.gif`, `.png`, `.jpg`, `.webp`, or `.bmp` into 16 `GrayFrame`s.
pub fn load_frames_from_bytes(bytes: &[u8]) -> Result<Vec<GrayFrame>, OledError> {
    let format = image::guess_format(bytes).unwrap_or(ImageFormat::Png);

    if format == ImageFormat::Gif {
        let cursor = Cursor::new(bytes);
        let decoder = image::codecs::gif::GifDecoder::new(cursor)?;
        let raw_frames = decoder.into_frames().collect_frames()?;
        if raw_frames.is_empty() {
            return Err(OledError::EmptySource);
        }

        // Sample 16 evenly spaced keyframes across the GIF
        let total = raw_frames.len();
        let mut out = Vec::with_capacity(MAX_SCREENSAVER_FRAMES);
        for i in 0..MAX_SCREENSAVER_FRAMES {
            let src_idx = ((i * total) / MAX_SCREENSAVER_FRAMES).min(total - 1);
            let rgba = raw_frames[src_idx].buffer().clone();
            let dyn_img = DynamicImage::ImageRgba8(rgba);
            out.push(image_to_gray_frame(&dyn_img));
        }
        return Ok(out);
    }

    // Static image: replicate across 16 frames with subtle optional pulse or static consistency
    let img = image::load_from_memory(bytes)?;
    let frame = image_to_gray_frame(&img);
    Ok(vec![frame; MAX_SCREENSAVER_FRAMES])
}

/// Generates a 16-frame procedural animation for immediate testing without external files.
pub fn generate_demo_preset(preset: DemoPreset) -> Vec<GrayFrame> {
    let mut frames = Vec::with_capacity(MAX_SCREENSAVER_FRAMES);

    for f in 0..MAX_SCREENSAVER_FRAMES {
        let phase = (f as f32 / MAX_SCREENSAVER_FRAMES as f32) * std::f32::consts::TAU;
        let mut gray = GrayFrame::default();

        match preset {
            DemoPreset::TapeReels => {
                // Draw outer cassette housing border
                for x in 6..122 {
                    gray.luma[4 * OLED_WIDTH + x] = 240;
                    gray.luma[59 * OLED_WIDTH + x] = 240;
                }
                for y in 4..60 {
                    gray.luma[y * OLED_WIDTH + 6] = 240;
                    gray.luma[y * OLED_WIDTH + 121] = 240;
                }

                // Draw two spinning tape hubs at (40, 32) and (88, 32)
                for &(cx, cy) in &[(40.0f32, 32.0f32), (88.0f32, 32.0f32)] {
                    for y in 10..54 {
                        for x in 16..112 {
                            let dx = x as f32 - cx;
                            let dy = y as f32 - cy;
                            let dist = (dx * dx + dy * dy).sqrt();
                            if (dist - 16.0).abs() < 1.3 || (dist - 6.0).abs() < 1.2 {
                                gray.luma[y * OLED_WIDTH + x] = 255;
                            } else if dist > 6.5 && dist < 15.5 {
                                let angle = dy.atan2(dx) + phase;
                                // 3-spoke tape reel teeth
                                if (angle * 3.0).cos() > 0.72 {
                                    gray.luma[y * OLED_WIDTH + x] = 220;
                                }
                            }
                        }
                    }
                }
            }
            DemoPreset::Oscilloscope => {
                // Center axis grid dots
                for x in (0..OLED_WIDTH).step_by(4) {
                    gray.luma[32 * OLED_WIDTH + x] = 90;
                }
                for x in 0..OLED_WIDTH {
                    let norm_x = x as f32 / OLED_WIDTH as f32;
                    let y_f = 32.0
                        + 18.0 * (norm_x * std::f32::consts::TAU * 2.0 + phase).sin()
                        + 7.0 * (norm_x * std::f32::consts::TAU * 5.0 - phase * 2.0).cos();
                    let y = (y_f.round() as isize).clamp(2, (OLED_HEIGHT - 3) as isize) as usize;
                    gray.luma[y * OLED_WIDTH + x] = 255;
                    gray.luma[(y - 1) * OLED_WIDTH + x] = 180;
                    gray.luma[(y + 1) * OLED_WIDTH + x] = 180;
                }
            }
            DemoPreset::BeatSpectrum => {
                // 16 animated EQ bars matching the 16 pads
                for bar in 0..16 {
                    let bar_phase = phase + (bar as f32 * 0.55);
                    let height = (10.0 + 44.0 * ((bar_phase.sin() * 0.5 + 0.5).powf(1.4))) as usize;
                    let x_start = 4 + bar * 7 + (bar / 4) * 2;
                    for bx in x_start..(x_start + 5).min(OLED_WIDTH) {
                        for by in (OLED_HEIGHT - 4 - height.min(54))..(OLED_HEIGHT - 4) {
                            // Segmented LED meter look
                            if by % 3 != 0 {
                                gray.luma[by * OLED_WIDTH + bx] = 255;
                            }
                        }
                    }
                }
            }
        }

        frames.push(gray);
    }

    frames
}

/// Encodes a `MonoFrame` into an exact **1,086-byte** classic Windows v3 `BITMAPINFOHEADER`
/// 1-bit monochrome `.BMP` byte buffer compatible with the SP-404MKII OLED parser.
pub fn encode_sp404_bmp(frame: &MonoFrame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(SP404_BMP_BYTE_SIZE);

    // 1. BITMAPFILEHEADER (14 bytes)
    buf.extend_from_slice(b"BM"); // Signature
    buf.extend_from_slice(&(SP404_BMP_BYTE_SIZE as u32).to_le_bytes()); // File size (1086)
    buf.extend_from_slice(&0u16.to_le_bytes()); // Reserved 1
    buf.extend_from_slice(&0u16.to_le_bytes()); // Reserved 2
    buf.extend_from_slice(&62u32.to_le_bytes()); // Offset to pixel array (14 + 40 + 8 = 62)

    // 2. BITMAPINFOHEADER (40 bytes)
    buf.extend_from_slice(&40u32.to_le_bytes()); // Header size (40)
    buf.extend_from_slice(&(OLED_WIDTH as i32).to_le_bytes()); // Width (128)
    buf.extend_from_slice(&(OLED_HEIGHT as i32).to_le_bytes()); // Height (+64, standard bottom-up)
    buf.extend_from_slice(&1u16.to_le_bytes()); // Color planes (1)
    buf.extend_from_slice(&1u16.to_le_bytes()); // Bits per pixel (1-bit monochrome)
    buf.extend_from_slice(&0u32.to_le_bytes()); // Compression (0 = BI_RGB uncompressed)
    buf.extend_from_slice(&1024u32.to_le_bytes()); // Image data size (16 bytes/row * 64 rows = 1024)
    buf.extend_from_slice(&2835i32.to_le_bytes()); // X pixels per meter (72 DPI)
    buf.extend_from_slice(&2835i32.to_le_bytes()); // Y pixels per meter (72 DPI)
    buf.extend_from_slice(&2u32.to_le_bytes()); // Colors in color table (2)
    buf.extend_from_slice(&2u32.to_le_bytes()); // Important color count (2)

    // 3. Color Table (2 × 4-byte RGBQUAD = 8 bytes)
    // Index 0 = Black (B=0, G=0, R=0, Reserved=0)
    buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    // Index 1 = White (B=255, G=255, R=255, Reserved=0)
    buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);

    // 4. Bottom-Up Pixel Array (64 rows × 16 bytes = 1,024 bytes)
    // Row 63 is bottom of image, Row 0 is top of image.
    for y in (0..OLED_HEIGHT).rev() {
        for byte_col in 0..(OLED_WIDTH / 8) {
            let mut byte_val = 0u8;
            for bit in 0..8 {
                let x = byte_col * 8 + bit;
                if frame.get(x, y) {
                    byte_val |= 1 << (7 - bit);
                }
            }
            buf.push(byte_val);
        }
    }

    debug_assert_eq!(buf.len(), SP404_BMP_BYTE_SIZE);
    buf
}

/// Writes the dithered `MonoFrame` sequence to `picture_dir` (`EXPORT/PROJECT/PROJECT_XX/PICTURE`)
/// using Roland's exact naming specification:
/// - `startup_1.bmp`, `startup_2.bmp`
/// - `screen_saver_1.bmp` .. `screen_saver_16.bmp`
pub fn export_oled_pack_to_dir(
    frames: &[MonoFrame],
    picture_dir: &Path,
    target: OledExportTarget,
) -> Result<Vec<PathBuf>, OledError> {
    if frames.is_empty() {
        return Err(OledError::EmptySource);
    }

    fs::create_dir_all(picture_dir)?;
    let mut written = Vec::new();

    // 1. Write startup_1.bmp and startup_2.bmp if requested
    if matches!(
        target,
        OledExportTarget::Both | OledExportTarget::StartupOnly
    ) {
        let f1 = &frames[0];
        let f2 = frames.get(frames.len() / 2).unwrap_or(f1);
        for (idx, frame) in [f1, f2].into_iter().enumerate() {
            let path = picture_dir.join(format!("startup_{}.bmp", idx + 1));
            let bmp_bytes = encode_sp404_bmp(frame);
            fs::write(&path, bmp_bytes)?;
            if let Ok(file) = File::open(&path) {
                let _ = file.sync_all();
            }
            written.push(path);
        }
    }

    // 2. Write screen_saver_1.bmp .. screen_saver_16.bmp if requested
    if matches!(
        target,
        OledExportTarget::Both | OledExportTarget::ScreensaverOnly
    ) {
        for i in 0..MAX_SCREENSAVER_FRAMES {
            let frame = &frames[i % frames.len()];
            let path = picture_dir.join(format!("screen_saver_{}.bmp", i + 1));
            let bmp_bytes = encode_sp404_bmp(frame);
            fs::write(&path, bmp_bytes)?;
            if let Ok(file) = File::open(&path) {
                let _ = file.sync_all();
            }
            written.push(path);
        }
    }

    if let Ok(dir) = File::open(picture_dir) {
        let _ = dir.sync_all();
    }

    info!(
        "Exported {} SP-404MKII OLED BMP files to {}",
        written.len(),
        picture_dir.display()
    );

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_sp404_bmp_header_and_size() {
        let mut frame = MonoFrame::default();
        // Set top-left pixel (0, 0) and bottom-right pixel (127, 63)
        frame.set(0, 0, true);
        frame.set(127, 63, true);

        let bmp = encode_sp404_bmp(&frame);
        assert_eq!(bmp.len(), SP404_BMP_BYTE_SIZE);
        assert_eq!(&bmp[0..2], b"BM");

        let width = i32::from_le_bytes(bmp[18..22].try_into().unwrap());
        let height = i32::from_le_bytes(bmp[22..26].try_into().unwrap());
        let bpp = u16::from_le_bytes(bmp[28..30].try_into().unwrap());
        assert_eq!(width, 128);
        assert_eq!(height, 64);
        assert_eq!(bpp, 1);

        // Bottom-up verification:
        // First pixel row in file (offset 62..78) is y = 63 (bottom row).
        // Pixel (127, 63) is bit 0 of byte 15 of the first scanline in the file!
        assert_eq!(bmp[62 + 15], 0b0000_0001);

        // Last pixel row in file (offset 62 + 63*16) is y = 0 (top row).
        // Pixel (0, 0) is bit 7 of byte 0 of the last scanline in the file!
        assert_eq!(bmp[62 + 63 * 16], 0b1000_0000);
    }

    #[test]
    fn test_export_oled_pack_filenames() {
        let temp_dir = std::env::temp_dir().join("sp404_oled_pack_test");
        let _ = fs::remove_dir_all(&temp_dir);

        let gray_frames = generate_demo_preset(DemoPreset::TapeReels);
        let adj = OledAdjustments::default();
        let mono_frames: Vec<MonoFrame> =
            gray_frames.iter().map(|g| dither_frame(g, &adj)).collect();

        let picture_dir = temp_dir
            .join("EXPORT")
            .join("PROJECT")
            .join("PROJECT_01")
            .join("PICTURE");
        let written =
            export_oled_pack_to_dir(&mono_frames, &picture_dir, OledExportTarget::Both).unwrap();

        // 2 startup + 16 screensaver = 18 BMP files
        assert_eq!(written.len(), 18);
        assert!(picture_dir.join("startup_1.bmp").is_file());
        assert!(picture_dir.join("startup_2.bmp").is_file());
        assert!(picture_dir.join("screen_saver_1.bmp").is_file());
        assert!(picture_dir.join("screen_saver_16.bmp").is_file());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
