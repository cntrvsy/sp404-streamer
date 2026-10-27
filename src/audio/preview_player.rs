//! Background preview playback feeder for auditioning samples in the Crate.
//!
//! Feeds decoded 48kHz audio into the PipeWire monitor playback stream
//! with zero-latency response and instant Spacebar cutoff.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use ringbuf::HeapProd;
use ringbuf::traits::*;

use crate::sample_crate::conformer::DecodedAudio;

enum FeederCommand {
    Play(Arc<DecodedAudio>, usize),
    Stop,
}

/// Controller for auditioning audio samples through the monitor playback stream.
#[derive(Clone)]
pub struct PreviewPlayer {
    cmd_tx: Sender<FeederCommand>,
    is_playing: Arc<AtomicBool>,
    progress_ratio: Arc<AtomicU32>,
}

impl PreviewPlayer {
    /// Creates a new `PreviewPlayer` and spawns its background feeder thread.
    pub fn new(mut producer: HeapProd<f32>) -> Self {
        let (cmd_tx, cmd_rx): (Sender<FeederCommand>, Receiver<FeederCommand>) = mpsc::channel();
        let is_playing = Arc::new(AtomicBool::new(false));
        let is_playing_clone = Arc::clone(&is_playing);
        let progress_ratio = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let progress_clone = Arc::clone(&progress_ratio);

        thread::Builder::new()
            .name("sp404-preview-feeder".into())
            .spawn(move || {
                let mut current_audio: Option<Arc<DecodedAudio>> = None;
                let mut current_frame: usize = 0;

                loop {
                    // Check for incoming commands
                    match cmd_rx.try_recv() {
                        Ok(FeederCommand::Play(audio, start_frame)) => {
                            current_audio = Some(audio);
                            current_frame = start_frame;
                            is_playing_clone.store(true, Ordering::SeqCst);
                        }
                        Ok(FeederCommand::Stop) => {
                            current_audio = None;
                            current_frame = 0;
                            is_playing_clone.store(false, Ordering::SeqCst);
                            progress_clone.store(0.0f32.to_bits(), Ordering::Relaxed);
                        }
                        Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }

                    if let Some(ref audio) = current_audio {
                        let total_frames = if audio.samples.is_empty() {
                            0
                        } else {
                            audio.samples[0].len()
                        };

                        if current_frame >= total_frames {
                            current_audio = None;
                            current_frame = 0;
                            is_playing_clone.store(false, Ordering::SeqCst);
                            progress_clone.store(0.0f32.to_bits(), Ordering::Relaxed);
                            continue;
                        }

                        // Feed frames into ring buffer as long as there is space
                        let free_slots = producer.vacant_len();
                        let max_frames_to_write = free_slots / 2; // Stereo = 2 samples per frame

                        if max_frames_to_write > 0 {
                            let frames_to_write =
                                max_frames_to_write.min(total_frames - current_frame);

                            for _ in 0..frames_to_write {
                                let (sample_l, sample_r) = if audio.channels == 1 {
                                    let s = audio.samples[0][current_frame];
                                    (s, s)
                                } else {
                                    let l = audio.samples[0][current_frame];
                                    let r = if audio.samples.len() > 1 {
                                        audio.samples[1][current_frame]
                                    } else {
                                        l
                                    };
                                    (l, r)
                                };

                                let _ = producer.try_push(sample_l);
                                let _ = producer.try_push(sample_r);
                                current_frame += 1;
                            }

                            if total_frames > 0 {
                                let ratio = (current_frame as f32) / (total_frames as f32);
                                progress_clone.store(ratio.to_bits(), Ordering::Relaxed);
                            }
                        }

                        // Sleep briefly (~5ms) to avoid spinning while buffer is full
                        thread::sleep(Duration::from_millis(5));
                    } else {
                        // When idle, block waiting for next command to save CPU
                        match cmd_rx.recv() {
                            Ok(FeederCommand::Play(audio, start_frame)) => {
                                current_audio = Some(audio);
                                current_frame = start_frame;
                                is_playing_clone.store(true, Ordering::SeqCst);
                            }
                            Ok(FeederCommand::Stop) => {
                                current_audio = None;
                                current_frame = 0;
                                is_playing_clone.store(false, Ordering::SeqCst);
                                progress_clone.store(0.0f32.to_bits(), Ordering::Relaxed);
                            }
                            Err(_) => break,
                        }
                    }
                }
            })
            .expect("Failed to spawn preview feeder thread");

        Self {
            cmd_tx,
            is_playing,
            progress_ratio,
        }
    }

    /// Starts or restarts playback of the given decoded audio sample.
    pub fn play(&self, audio: DecodedAudio) {
        self.play_at(audio, 0.0);
    }

    /// Starts playback of the given decoded audio sample from the specified progress ratio (0.0..=1.0).
    pub fn play_at(&self, audio: DecodedAudio, start_ratio: f32) {
        let mut audio_48k = audio;
        let _ = audio_48k.resample_to_48k();
        let total = if audio_48k.samples.is_empty() {
            0
        } else {
            audio_48k.samples[0].len()
        };
        let start_frame = ((start_ratio.clamp(0.0, 1.0) * total as f32) as usize).min(total);
        let _ = self
            .cmd_tx
            .send(FeederCommand::Play(Arc::new(audio_48k), start_frame));
    }

    /// Stops any currently playing sample preview immediately.
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(FeederCommand::Stop);
    }

    /// Toggles playback: stops if playing, or plays the sample.
    pub fn toggle(&self, audio: DecodedAudio) {
        if self.is_playing() {
            self.stop();
        } else {
            self.play(audio);
        }
    }

    /// Returns whether a sample preview is currently actively playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Returns the current playback position ratio (0.0 to 1.0).
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress_ratio.load(Ordering::Relaxed))
    }
}
