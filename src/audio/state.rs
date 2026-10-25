use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

/// Shared thread-safe, lock-free audio state between PipeWire processing and egui UI.
pub struct SharedAudioState {
    /// Peak audio level in left channel (stored as f32 bits, in dBFS: -60.0 to 0.0+)
    pub peak_l_db: AtomicU32,
    /// Peak audio level in right channel (stored as f32 bits, in dBFS: -60.0 to 0.0+)
    pub peak_r_db: AtomicU32,
    /// RMS audio level in left channel (stored as f32 bits, in dBFS)
    pub rms_l_db: AtomicU32,
    /// RMS audio level in right channel (stored as f32 bits, in dBFS)
    pub rms_r_db: AtomicU32,

    /// Whether peak exceeded -0.1 dBFS (clipping flag)
    pub is_clipped_l: AtomicBool,
    pub is_clipped_r: AtomicBool,

    /// Whether SP-404MKII device is currently detected in PipeWire graph
    pub is_connected: AtomicBool,

    /// Device name / description string
    pub device_name: RwLock<String>,

    /// Sample rate of stream (e.g., 48000)
    pub sample_rate: AtomicU32,

    /// Master output / monitor gain multiplier (f32 bits, e.g. 1.0 = 100%)
    pub monitor_gain: AtomicU32,

    /// Whether local monitor is physically unlinked from local speaker/headphone ports
    /// (Discord still receives audio, but local ears do not)
    pub local_monitor_unlinked: AtomicBool,

    /// Total audio buffer frames processed (monotonically increasing activity counter)
    pub frame_counter: AtomicU32,
}

impl SharedAudioState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            peak_l_db: AtomicU32::new((-60.0f32).to_bits()),
            peak_r_db: AtomicU32::new((-60.0f32).to_bits()),
            rms_l_db: AtomicU32::new((-60.0f32).to_bits()),
            rms_r_db: AtomicU32::new((-60.0f32).to_bits()),
            is_clipped_l: AtomicBool::new(false),
            is_clipped_r: AtomicBool::new(false),
            is_connected: AtomicBool::new(false),
            device_name: RwLock::new("Searching for SP-404MKII...".to_string()),
            sample_rate: AtomicU32::new(48000),
            monitor_gain: AtomicU32::new(1.0f32.to_bits()),
            local_monitor_unlinked: AtomicBool::new(false),
            frame_counter: AtomicU32::new(0),
        })
    }

    #[inline]
    pub fn get_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_db.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn get_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_db.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn set_peak_l(&self, val_db: f32) {
        self.peak_l_db.store(val_db.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn set_peak_r(&self, val_db: f32) {
        self.peak_r_db.store(val_db.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn get_monitor_gain(&self) -> f32 {
        f32::from_bits(self.monitor_gain.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn set_monitor_gain(&self, gain: f32) {
        self.monitor_gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_connected(&self, connected: bool) {
        self.is_connected.store(connected, Ordering::Relaxed);
    }

    #[inline]
    pub fn is_local_monitor_unlinked(&self) -> bool {
        self.local_monitor_unlinked.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set_local_monitor_unlinked(&self, unlinked: bool) {
        self.local_monitor_unlinked.store(unlinked, Ordering::Relaxed);
    }
}
