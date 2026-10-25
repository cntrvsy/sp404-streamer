use std::convert::TryInto;
use std::io::Cursor;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use libspa_sys as spa_sys;
use pipewire as pw;
use pw::properties::properties;
use pw::spa::pod::Pod;
use pw::types::ObjectType;
use ringbuf::traits::*;
use ringbuf::HeapRb;
use tracing::{debug, error, info, warn};

use super::port_manager::PortManager;
use super::state::SharedAudioState;

pub const DEFAULT_SAMPLE_RATE: u32 = 48000;
pub const DEFAULT_CHANNELS: u32 = 2;
pub const BUFFER_CAPACITY: usize = 16384; // ~170ms of 48kHz stereo float samples

/// Background PipeWire engine handling SP-404 device lifecycle,
/// audio capture, peak metering, and low-latency Discord-ready playback.
#[allow(dead_code)]
pub struct PipewireClient {
    state: Arc<SharedAudioState>,
    thread_handle: Option<JoinHandle<()>>,
}

impl PipewireClient {
    pub fn start(state: Arc<SharedAudioState>) -> Self {
        let state_clone = Arc::clone(&state);
        let thread_handle = thread::Builder::new()
            .name("sp404-pipewire".into())
            .spawn(move || {
                if let Err(e) = Self::run_loop(state_clone) {
                    error!("PipeWire event loop exited with error: {:?}", e);
                }
            })
            .expect("Failed to spawn PipeWire thread");

        Self {
            state,
            thread_handle: Some(thread_handle),
        }
    }

    fn run_loop(state: Arc<SharedAudioState>) -> Result<(), pw::Error> {
        pw::init();

        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let registry = core.get_registry_rc()?;

        // Lock-free SPSC ring buffer for audio transfer from capture to playback
        let ring_buffer = HeapRb::<f32>::new(BUFFER_CAPACITY);
        let (mut producer, mut consumer) = ring_buffer.split();

        // 1. Create the Playback Stream
        // Discord (Vesktop) detects application audio through this node by matching process ID
        let pid_str = std::process::id().to_string();
        let playback_props = properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::APP_NAME => "SP-404MKII Streamer",
            *pw::keys::APP_ID => "sp404-streamer",
            *pw::keys::NODE_NAME => "sp404_streamer_playback",
            *pw::keys::NODE_DESCRIPTION => "SP-404MKII Discord Stream",
            *pw::keys::NODE_LATENCY => "256/48000",
            *pw::keys::APP_PROCESS_ID => pid_str.as_str(),
        };

        let playback_stream = pw::stream::StreamBox::new(&core, "sp404-playback", playback_props)?;

        let state_for_playback = Arc::clone(&state);
        let _playback_listener = playback_stream
            .add_local_listener::<()>()
            .process(move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };

                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let data = &mut datas[0];
                let stride = mem::size_of::<f32>() * DEFAULT_CHANNELS as usize;

                let gain = state_for_playback.get_monitor_gain();

                if let Some(slice) = data.data() {
                    let n_samples = slice.len() / mem::size_of::<f32>();
                    let n_frames = slice.len() / stride;

                    // Fill buffer from producer or output silence if waiting
                    for i in 0..n_samples {
                        let sample = consumer.try_pop().unwrap_or(0.0);
                        let final_sample = sample * gain;
                        let start = i * mem::size_of::<f32>();
                        let end = start + mem::size_of::<f32>();
                        if end <= slice.len() {
                            slice[start..end].copy_from_slice(&final_sample.to_le_bytes());
                        }
                    }

                    let chunk = data.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = stride as _;
                    *chunk.size_mut() = (stride * n_frames) as _;
                }
            })
            .register()?;

        // Build F32LE Stereo 48kHz format Pod parameter for Playback
        let mut playback_audio_info = pw::spa::param::audio::AudioInfoRaw::new();
        playback_audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
        playback_audio_info.set_rate(DEFAULT_SAMPLE_RATE);
        playback_audio_info.set_channels(DEFAULT_CHANNELS);
        let mut pos = [0; pw::spa::param::audio::MAX_CHANNELS];
        pos[0] = spa_sys::SPA_AUDIO_CHANNEL_FL;
        pos[1] = spa_sys::SPA_AUDIO_CHANNEL_FR;
        playback_audio_info.set_position(pos);

        let playback_pod_obj = pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pw::spa::param::ParamType::EnumFormat.as_raw(),
            properties: playback_audio_info.into(),
        };

        let playback_values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
            Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(playback_pod_obj),
        )
        .unwrap()
        .0
        .into_inner();

        let mut playback_params = [Pod::from_bytes(&playback_values).unwrap()];

        playback_stream.connect(
            pw::spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut playback_params,
        )?;

        info!("Playback stream connected to PipeWire graph.");

        // 2. Set up Device Registry Watcher
        let active_target_id = Arc::new(std::sync::atomic::AtomicU32::new(0));

        let state_for_add = Arc::clone(&state);
        let target_id_for_add = Arc::clone(&active_target_id);

        let state_for_remove = Arc::clone(&state);
        let target_id_for_remove = Arc::clone(&active_target_id);

        let _registry_listener = registry
            .add_listener_local()
            .global(move |obj| {
                if obj.type_ == ObjectType::Node {
                    if let Some(props) = obj.props {
                        let media_class = props.get("media.class").unwrap_or("");
                        let node_name = props.get("node.name").unwrap_or("");
                        let node_desc = props.get("node.description").unwrap_or("");

                        // Look for audio source nodes
                        if media_class == "Audio/Source" {
                            debug!("Audio Source discovered: {} ({})", node_desc, node_name);

                            if PortManager::is_sp404_device(node_name, node_desc) {
                                info!("Found Roland SP-404MKII! Node ID: {}", obj.id);
                                target_id_for_add.store(obj.id, Ordering::Relaxed);
                                state_for_add.set_connected(true);
                                *state_for_add.device_name.write() = node_desc.to_string();
                            }
                        }
                    }
                }
            })
            .global_remove(move |id| {
                if id == target_id_for_remove.load(Ordering::Relaxed) && id != 0 {
                    warn!("SP-404MKII was unplugged/removed (ID: {})", id);
                    target_id_for_remove.store(0, Ordering::Relaxed);
                    state_for_remove.set_connected(false);
                    *state_for_remove.device_name.write() =
                        "Waiting for SP-404MKII...".to_string();
                    state_for_remove.set_peak_l(-60.0);
                    state_for_remove.set_peak_r(-60.0);
                }
            })
            .register();

        // 3. Set up Capture Stream
        let capture_props = properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::NODE_NAME => "sp404_streamer_capture",
            *pw::keys::NODE_LATENCY => "256/48000",
        };

        let capture_stream = pw::stream::StreamBox::new(&core, "sp404-capture", capture_props)?;

        let state_for_capture = Arc::clone(&state);
        let _capture_listener = capture_stream
            .add_local_listener::<()>()
            .process(move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };

                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let data = &mut datas[0];
                if let Some(samples) = data.data() {
                    let n_floats = samples.len() / mem::size_of::<f32>();
                    let n_frames = n_floats / DEFAULT_CHANNELS as usize;

                    let mut peak_l: f32 = 0.0;
                    let mut peak_r: f32 = 0.0;

                    for i in 0..n_frames {
                        let idx_l = (i * 2) * mem::size_of::<f32>();
                        let idx_r = (i * 2 + 1) * mem::size_of::<f32>();

                        if idx_r + mem::size_of::<f32>() <= samples.len() {
                            let val_l = f32::from_le_bytes(
                                samples[idx_l..idx_l + 4].try_into().unwrap_or([0; 4]),
                            );
                            let val_r = f32::from_le_bytes(
                                samples[idx_r..idx_r + 4].try_into().unwrap_or([0; 4]),
                            );

                            peak_l = peak_l.max(val_l.abs());
                            peak_r = peak_r.max(val_r.abs());

                            // Push stereo samples into ring buffer for playback
                            let _ = producer.try_push(val_l);
                            let _ = producer.try_push(val_r);
                        }
                    }

                    // Convert linear amplitude to dBFS
                    let db_l = if peak_l > 0.0001 {
                        (20.0 * peak_l.log10()).clamp(-60.0, 6.0)
                    } else {
                        -60.0
                    };

                    let db_r = if peak_r > 0.0001 {
                        (20.0 * peak_r.log10()).clamp(-60.0, 6.0)
                    } else {
                        -60.0
                    };

                    // Check for clipping (> -0.1 dBFS)
                    if db_l >= -0.1 {
                        state_for_capture.is_clipped_l.store(true, Ordering::Relaxed);
                    }
                    if db_r >= -0.1 {
                        state_for_capture.is_clipped_r.store(true, Ordering::Relaxed);
                    }

                    state_for_capture.set_peak_l(db_l);
                    state_for_capture.set_peak_r(db_r);
                    state_for_capture.frame_counter.fetch_add(1, Ordering::Relaxed);
                }
            })
            .register()?;

        // Connect Capture Stream
        let mut capture_audio_info = pw::spa::param::audio::AudioInfoRaw::new();
        capture_audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
        capture_audio_info.set_rate(DEFAULT_SAMPLE_RATE);
        capture_audio_info.set_channels(DEFAULT_CHANNELS);

        let capture_pod_obj = pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pw::spa::param::ParamType::EnumFormat.as_raw(),
            properties: capture_audio_info.into(),
        };

        let capture_values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
            Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(capture_pod_obj),
        )
        .unwrap()
        .0
        .into_inner();

        let mut capture_params = [Pod::from_bytes(&capture_values).unwrap()];

        capture_stream.connect(
            pw::spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut capture_params,
        )?;

        info!("Capture stream connected to PipeWire graph.");

        // Run the PipeWire event loop
        mainloop.run();

        Ok(())
    }
}
