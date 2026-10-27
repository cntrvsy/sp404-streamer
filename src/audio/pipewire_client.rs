//! Low-latency PipeWire audio engine and real-time processing streams.
//!
//! Spawns the dedicated PipeWire event loop thread and manages:
//! - An input capture stream unlinked from default microphones and dynamically linked to the SP-404.
//! - An output playback stream registered with application credentials for Discord window capture.
//! - A lock-free SPSC ring buffer for inter-stream audio transfer with jitter pre-buffering.

use std::convert::TryInto;
use std::io::Cursor;
use std::mem;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use libspa_sys as spa_sys;
use pipewire as pw;
use pw::properties::properties;
use pw::spa::pod::Pod;
use pw::types::ObjectType;
use ringbuf::traits::*;
use ringbuf::{HeapCons, HeapRb};
use tracing::{debug, error, info, warn};

use super::command::AudioCommand;
use super::port_manager::PortManager;
use super::preview_player::PreviewPlayer;
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
    worker_handle: Option<JoinHandle<()>>,
}

impl PipewireClient {
    pub fn start(
        state: Arc<SharedAudioState>,
    ) -> (Self, mpsc::Sender<AudioCommand>, PreviewPlayer) {
        let (command_tx, command_rx) = mpsc::channel::<AudioCommand>();

        // Preview ring buffer for Crate sample auditioning
        let preview_rb = HeapRb::<f32>::new(BUFFER_CAPACITY);
        let (preview_prod, preview_cons) = preview_rb.split();
        let preview_player = PreviewPlayer::new(preview_prod);

        // 1. Dedicated Command Worker Thread (processes UI/registry commands in FIFO order)
        let state_for_worker = Arc::clone(&state);
        let worker_handle = thread::Builder::new()
            .name("sp404-cmd-worker".into())
            .spawn(move || {
                while let Ok(cmd) = command_rx.recv() {
                    match cmd {
                        AudioCommand::RoutePlayback { sink_name } => {
                            *state_for_worker.selected_sink_name.write() = sink_name.clone();
                            PortManager::route_playback_to_sink(
                                "sp404_streamer_playback",
                                sink_name.as_deref(),
                            );
                        }
                        AudioCommand::LinkCapture { sp404_node } => {
                            PortManager::link_sp404_capture_with_retry(
                                &sp404_node,
                                "sp404_streamer_capture",
                                6,
                                std::time::Duration::from_millis(40),
                            );
                        }
                        AudioCommand::UnlinkCapture => {
                            PortManager::unlink_capture_stream("sp404_streamer_capture");
                        }
                        AudioCommand::SetLocalMonitorMute(mute) => {
                            state_for_worker.set_local_monitor_unlinked(mute);
                            if mute {
                                PortManager::unlink_local_monitor("sp404_streamer_playback");
                            } else {
                                let target_sink =
                                    state_for_worker.selected_sink_name.read().clone();
                                PortManager::route_playback_to_sink(
                                    "sp404_streamer_playback",
                                    target_sink.as_deref(),
                                );
                            }
                        }
                        AudioCommand::RefreshSinks => {
                            let sinks = PortManager::get_available_sinks();
                            *state_for_worker.available_sinks.write() = sinks;
                        }
                    }
                }
            })
            .expect("Failed to spawn audio command worker thread");

        // 2. Dedicated PipeWire Event Loop Thread
        let state_clone = Arc::clone(&state);
        let cmd_tx_for_pw = command_tx.clone();
        let thread_handle = thread::Builder::new()
            .name("sp404-pipewire".into())
            .spawn(move || {
                if let Err(e) = Self::run_loop(&state_clone, cmd_tx_for_pw, preview_cons) {
                    error!("PipeWire event loop exited with error: {:?}", e);
                }
            })
            .expect("Failed to spawn PipeWire thread");

        (
            Self {
                state,
                thread_handle: Some(thread_handle),
                worker_handle: Some(worker_handle),
            },
            command_tx,
            preview_player,
        )
    }

    fn run_loop(
        state: &Arc<SharedAudioState>,
        command_tx: mpsc::Sender<AudioCommand>,
        mut preview_consumer: HeapCons<f32>,
    ) -> Result<(), pw::Error> {
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

        let mut is_primed = false;
        let state_for_playback = Arc::clone(state);
        let _playback_listener = playback_stream
            .add_local_listener::<()>()
            .process(move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };

                let req = buffer.requested() as usize;

                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let data = &mut datas[0];
                let stride = mem::size_of::<f32>() * DEFAULT_CHANNELS as usize;
                let gain = state_for_playback.get_monitor_gain();

                if let Some(slice) = data.data() {
                    let max_frames = slice.len() / stride;
                    let n_frames = if req > 0 {
                        req.min(max_frames)
                    } else {
                        256.min(max_frames)
                    };

                    let occupied = consumer.occupied_len();

                    // Pre-buffering / jitter prevention:
                    // Wait until at least 512 samples (~5.3ms) are buffered before starting playback
                    if !is_primed && occupied >= 512 {
                        is_primed = true;
                    }

                    // Drop stale samples if buffer grows too large (> 4096 samples = ~42ms)
                    if occupied > 4096 {
                        let to_drop = occupied - 1024;
                        for _ in 0..to_drop {
                            let _ = consumer.try_pop();
                        }
                    }

                    if is_primed {
                        for i in 0..n_frames {
                            let stream_l = consumer.try_pop().unwrap_or_else(|| {
                                is_primed = false;
                                0.0
                            });
                            let stream_r = consumer.try_pop().unwrap_or_else(|| {
                                is_primed = false;
                                0.0
                            });

                            let prev_l = preview_consumer.try_pop().unwrap_or(0.0);
                            let prev_r = preview_consumer.try_pop().unwrap_or(0.0);

                            let final_l = ((stream_l + prev_l) * gain).clamp(-1.0, 1.0);
                            let final_r = ((stream_r + prev_r) * gain).clamp(-1.0, 1.0);

                            let start_l = i * stride;
                            let start_r = start_l + mem::size_of::<f32>();

                            slice[start_l..start_l + 4].copy_from_slice(&final_l.to_le_bytes());
                            slice[start_r..start_r + 4].copy_from_slice(&final_r.to_le_bytes());
                        }
                    } else {
                        // When stream is not primed, output preview samples if active, else silence
                        for i in 0..n_frames {
                            let prev_l = preview_consumer.try_pop().unwrap_or(0.0);
                            let prev_r = preview_consumer.try_pop().unwrap_or(0.0);

                            let final_l = (prev_l * gain).clamp(-1.0, 1.0);
                            let final_r = (prev_r * gain).clamp(-1.0, 1.0);

                            let start_l = i * stride;
                            let start_r = start_l + mem::size_of::<f32>();

                            slice[start_l..start_l + 4].copy_from_slice(&final_l.to_le_bytes());
                            slice[start_r..start_r + 4].copy_from_slice(&final_r.to_le_bytes());
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

        let playback_values = serialize_spa_pod(playback_pod_obj)?;
        let playback_pod = Pod::from_bytes(&playback_values).ok_or(pw::Error::CreationFailed)?;
        let mut playback_params = [playback_pod];

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

        let state_for_add = Arc::clone(state);
        let target_id_for_add = Arc::clone(&active_target_id);
        let cmd_tx_for_add = command_tx.clone();

        let state_for_remove = Arc::clone(state);
        let target_id_for_remove = Arc::clone(&active_target_id);
        let cmd_tx_for_remove = command_tx;

        let _registry_listener = registry
            .add_listener_local()
            .global(move |obj| {
                if obj.type_ != ObjectType::Node {
                    return;
                }
                let Some(props) = obj.props else {
                    return;
                };

                let media_class = props.get("media.class").unwrap_or("");
                let node_name = props.get("node.name").unwrap_or("");
                let node_desc = props.get("node.description").unwrap_or("");

                // Look for audio source nodes
                if media_class == "Audio/Source" {
                    debug!("Audio Source discovered: {} ({})", node_desc, node_name);

                    if PortManager::is_sp404_device(node_name, node_desc) {
                        info!(
                            "Found Roland SP-404MKII! Node ID: {}, Node Name: {}",
                            obj.id, node_name
                        );
                        target_id_for_add.store(obj.id, Ordering::Relaxed);
                        state_for_add.set_connected(true);
                        *state_for_add.device_name.write() = node_desc.to_string();
                        *state_for_add.sp404_node_name.write() = Some(node_name.to_string());

                        let _ = cmd_tx_for_add.send(AudioCommand::LinkCapture {
                            sp404_node: node_name.to_string(),
                        });
                    }
                } else if media_class == "Audio/Sink" {
                    // Reactively refresh available sinks when a new sink appears
                    let _ = cmd_tx_for_add.send(AudioCommand::RefreshSinks);
                }
            })
            .global_remove(move |id| {
                if id == target_id_for_remove.load(Ordering::Relaxed) && id != 0 {
                    warn!("SP-404MKII was unplugged/removed (ID: {})", id);
                    target_id_for_remove.store(0, Ordering::Relaxed);
                    state_for_remove.set_connected(false);
                    *state_for_remove.device_name.write() = "Waiting for SP-404MKII...".to_string();
                    *state_for_remove.sp404_node_name.write() = None;
                    state_for_remove.set_peaks(0.0, 0.0);

                    let _ = cmd_tx_for_remove.send(AudioCommand::UnlinkCapture);
                }
                // Reactively refresh sinks when a node disappears
                let _ = cmd_tx_for_remove.send(AudioCommand::RefreshSinks);
            })
            .register();

        // 3. Set up Capture Stream (explicitly unlinked from default system mic)
        let capture_props = properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::NODE_NAME => "sp404_streamer_capture",
            *pw::keys::NODE_LATENCY => "256/48000",
            *pw::keys::NODE_AUTOCONNECT => "false",
        };

        let capture_stream = pw::stream::StreamBox::new(&core, "sp404-capture", capture_props)?;

        let state_for_capture = Arc::clone(state);
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
                let chunk_size = data.chunk().size() as usize;
                let chunk_offset = data.chunk().offset() as usize;

                if chunk_size == 0 {
                    return;
                }

                if let Some(all_bytes) = data.data() {
                    if chunk_offset + chunk_size > all_bytes.len() {
                        return;
                    }
                    let valid_bytes = &all_bytes[chunk_offset..chunk_offset + chunk_size];
                    let stride = mem::size_of::<f32>() * DEFAULT_CHANNELS as usize;
                    let n_frames = chunk_size / stride;

                    let mut peak_l: f32 = 0.0;
                    let mut peak_r: f32 = 0.0;

                    for i in 0..n_frames {
                        let idx_l = i * stride;
                        let idx_r = idx_l + mem::size_of::<f32>();

                        if idx_r + mem::size_of::<f32>() <= valid_bytes.len() {
                            let val_l = f32::from_le_bytes(
                                valid_bytes[idx_l..idx_l + 4].try_into().unwrap_or([0; 4]),
                            );
                            let val_r = f32::from_le_bytes(
                                valid_bytes[idx_r..idx_r + 4].try_into().unwrap_or([0; 4]),
                            );

                            peak_l = peak_l.max(val_l.abs());
                            peak_r = peak_r.max(val_r.abs());

                            // Push stereo samples into ring buffer for playback
                            let _ = producer.try_push(val_l);
                            let _ = producer.try_push(val_r);
                        }
                    }

                    state_for_capture.set_peaks(peak_l, peak_r);
                    state_for_capture
                        .frame_counter
                        .fetch_add(1, Ordering::Relaxed);
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

        let capture_values = serialize_spa_pod(capture_pod_obj)?;
        let capture_pod = Pod::from_bytes(&capture_values).ok_or(pw::Error::CreationFailed)?;
        let mut capture_params = [capture_pod];

        capture_stream.connect(
            pw::spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
            &mut capture_params,
        )?;

        info!("Capture stream connected to PipeWire graph.");

        // Run the PipeWire event loop
        mainloop.run();

        Ok(())
    }
}

/// Helper function to safely serialize an SPA POD object without panicking.
fn serialize_spa_pod(pod_obj: pw::spa::pod::Object) -> Result<Vec<u8>, pw::Error> {
    pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pod_obj),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|_| pw::Error::CreationFailed)
}
