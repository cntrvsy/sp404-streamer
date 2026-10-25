//! Thread-safe shared audio and UI state.
//!
//! Exposes lock-free atomic meters and minimal RwLocks for communicating
//! audio levels, hotplug state, and active playback sinks between the PipeWire
//! real-time thread, background command worker, and the egui render loop.

use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use super::port_manager::AudioSinkDevice;

/// Shared thread-safe, lock-free audio state between PipeWire processing and egui UI.
pub struct SharedAudioState {
    /// Packed linear stereo peaks (Left in high 32 bits, Right in low 32 bits, as raw f32 bits).
    /// Updated and read in a single 64-bit atomic transaction to eliminate channel tearing.
    packed_peaks: AtomicU64,

    /// Whether SP-404MKII device is currently detected in PipeWire graph
    pub is_connected: AtomicBool,

    /// Device name / description string
    pub device_name: RwLock<String>,

    /// PipeWire node name of the SP-404MKII (e.g. alsa_input.usb-Roland_SP-404MKII...)
    pub sp404_node_name: RwLock<Option<String>>,

    /// Currently selected playback sink node name (None = System Default)
    pub selected_sink_name: RwLock<Option<String>>,

    /// List of available playback sink devices
    pub available_sinks: RwLock<Vec<AudioSinkDevice>>,

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
            packed_peaks: AtomicU64::new(0),
            is_connected: AtomicBool::new(false),
            device_name: RwLock::new("Searching for SP-404MKII...".to_string()),
            sp404_node_name: RwLock::new(None),
            selected_sink_name: RwLock::new(None),
            available_sinks: RwLock::new(Vec::new()),
            sample_rate: AtomicU32::new(48000),
            monitor_gain: AtomicU32::new(1.0f32.to_bits()),
            local_monitor_unlinked: AtomicBool::new(false),
            frame_counter: AtomicU32::new(0),
        })
    }

    /// Atomically updates both Left and Right linear peaks in a single transaction.
    ///
    /// Stores the IEEE 754 bit representations of `linear_l` in the upper 32 bits
    /// and `linear_r` in the lower 32 bits of an `AtomicU64`.
    #[inline]
    pub fn set_peaks(&self, linear_l: f32, linear_r: f32) {
        let l_bits = linear_l.to_bits() as u64;
        let r_bits = linear_r.to_bits() as u64;
        let packed = (l_bits << 32) | r_bits;
        self.packed_peaks.store(packed, Ordering::Relaxed);
    }

    /// Atomically reads both Left and Right linear peaks simultaneously.
    #[inline]
    pub fn get_peaks(&self) -> (f32, f32) {
        let packed = self.packed_peaks.load(Ordering::Relaxed);
        let l_bits = (packed >> 32) as u32;
        let r_bits = (packed & 0xFFFF_FFFF) as u32;
        (f32::from_bits(l_bits), f32::from_bits(r_bits))
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
        self.local_monitor_unlinked
            .store(unlinked, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stereo_peak_bitpacking() {
        let state = SharedAudioState::new();

        // 1. Exact zero
        state.set_peaks(0.0, 0.0);
        assert_eq!(state.get_peaks(), (0.0, 0.0));

        // 2. Typical asymmetric audio levels
        state.set_peaks(0.25, 0.85);
        let (l, r) = state.get_peaks();
        assert!((l - 0.25).abs() < f32::EPSILON);
        assert!((r - 0.85).abs() < f32::EPSILON);

        // 3. Clipping signals (> 1.0)
        state.set_peaks(1.5, 2.0);
        let (l, r) = state.get_peaks();
        assert!((l - 1.5).abs() < f32::EPSILON);
        assert!((r - 2.0).abs() < f32::EPSILON);

        // 4. Subnormal / small signals
        state.set_peaks(1e-6, 1e-7);
        let (l, r) = state.get_peaks();
        assert!((l - 1e-6).abs() < 1e-9);
        assert!((r - 1e-7).abs() < 1e-9);
    }

    #[test]
    fn test_atomic_gain() {
        let state = SharedAudioState::new();
        assert!((state.get_monitor_gain() - 1.0).abs() < f32::EPSILON);

        state.set_monitor_gain(0.5);
        assert!((state.get_monitor_gain() - 0.5).abs() < f32::EPSILON);

        state.set_monitor_gain(1.5);
        assert!((state.get_monitor_gain() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_atomic_connection_and_unlinked_toggles() {
        let state = SharedAudioState::new();
        assert!(!state.is_connected());
        assert!(!state.is_local_monitor_unlinked());

        state.set_connected(true);
        assert!(state.is_connected());

        state.set_local_monitor_unlinked(true);
        assert!(state.is_local_monitor_unlinked());

        state.set_local_monitor_unlinked(false);
        assert!(!state.is_local_monitor_unlinked());
    }
}
