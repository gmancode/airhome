//! WASAPI loopback capture (via cpal) that feeds a `LiveFrameSender`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use airplay_audio::{LiveFrameSender, LivePcmFrame};
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The library's sinc resampler only accepts blocks of exactly this many frames.
const CHUNK_FRAMES: usize = 1024;

/// Peak absolute sample (0..=32767) seen since the UI last read it.
pub static PEAK: AtomicU64 = AtomicU64::new(0);

/// Samples quieter than this (0..=32767, about -60 dBFS) do not count as sound.
const SOUND_PEAK: u64 = 32;

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// When (ms since program start) sound was last heard on the default output.
static LAST_SOUND_MS: AtomicU64 = AtomicU64::new(0);

/// Count "now" as the last time sound was heard (a stream just started).
pub fn mark_sound() {
    LAST_SOUND_MS.store(now_ms(), Ordering::Relaxed);
}

/// How long the PC has been silent.
pub fn silent_for() -> Duration {
    Duration::from_millis(now_ms().saturating_sub(LAST_SOUND_MS.load(Ordering::Relaxed)))
}

/// Current time on the same clock as [`sound_since`], for "has there been sound since X".
pub fn clock_ms() -> u64 {
    now_ms()
}

/// Has sound been heard after the moment `since_ms` (taken from [`clock_ms`])?
pub fn sound_since(since_ms: u64) -> bool {
    LAST_SOUND_MS.load(Ordering::Relaxed) > since_ms
}

/// Source format of the default output device.
#[derive(Clone, Copy, Debug)]
pub struct SourceFormat {
    pub rate: u32,
    pub channels: u16,
    pub sample_format: cpal::SampleFormat,
}

pub fn default_output_format() -> Result<(String, SourceFormat)> {
    let host = cpal::default_host();
    let dev = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default audio output device"))?;
    let cfg = dev.default_output_config()?;
    Ok((
        dev.name().unwrap_or_else(|_| "unknown".into()),
        SourceFormat {
            rate: cfg.sample_rate().0,
            channels: cfg.channels(),
            sample_format: cfg.sample_format(),
        },
    ))
}

/// Collects captured stereo samples and forwards them in fixed `CHUNK_FRAMES` blocks.
struct Chunker {
    senders: Vec<LiveFrameSender>,
    rate: u32,
    pending: Vec<i16>,
}

impl Chunker {
    fn new(senders: Vec<LiveFrameSender>, rate: u32) -> Self {
        Self { senders, rate, pending: Vec::with_capacity(CHUNK_FRAMES * 4) }
    }

    fn push(&mut self, stereo: Vec<i16>) {
        self.pending.extend(stereo);
        let block = CHUNK_FRAMES * 2;
        while self.pending.len() >= block {
            let samples: Vec<i16> = self.pending.drain(..block).collect();
            // Every speaker of a group gets the same audio.
            for s in &self.senders {
                s.try_send(LivePcmFrame { samples: samples.clone(), channels: 2, sample_rate: self.rate });
            }
        }
    }
}

/// State shared between the WASAPI callback and the silence-filler loop.
struct Shared {
    chunker: Chunker,
    /// When the last real audio buffer arrived.
    last_real: Instant,
    /// Up to when we have already emitted silence (to avoid double-filling).
    silence_upto: Instant,
}

impl Shared {
    fn push_real(&mut self, stereo: Vec<i16>) {
        let peak = stereo.iter().map(|s| s.unsigned_abs() as u64).max().unwrap_or(0);
        PEAK.fetch_max(peak, Ordering::Relaxed);
        if peak > SOUND_PEAK {
            mark_sound();
        }
        self.last_real = Instant::now();
        self.silence_upto = self.last_real;
        self.chunker.push(stereo);
    }

    /// If no real audio arrived for >30 ms, emit zeros covering the elapsed gap.
    fn fill_silence(&mut self, rate: u32) {
        let now = Instant::now();
        if now.duration_since(self.last_real) <= Duration::from_millis(30) {
            return;
        }
        let from = self.silence_upto.max(self.last_real);
        let frames = (now.duration_since(from).as_secs_f64() * rate as f64) as usize;
        if frames == 0 {
            return;
        }
        // Advance only by the time the emitted frames really cover. Setting this to `now` threw away the
        // fraction of a frame on every 5 ms tick, so during long silence the stream fell behind the wall clock
        // (~0.2 %: half a second after 5 minutes). The speaker then got packets whose play time had passed.
        self.silence_upto = from + Duration::from_secs_f64(frames as f64 / rate as f64);
        self.chunker.push(vec![0i16; frames * 2]);
    }
}

/// Reduce interleaved N-channel samples to interleaved stereo (first two channels).
fn to_stereo_i16(samples: impl Iterator<Item = i16>, channels: usize) -> Vec<i16> {
    let all: Vec<i16> = samples.collect();
    match channels {
        2 => all,
        1 => all.iter().flat_map(|&s| [s, s]).collect(),
        n => all.chunks_exact(n).flat_map(|f| [f[0], f[1]]).collect(),
    }
}

/// Capture the default output device in loopback mode on a dedicated thread.
/// (cpal streams are not `Send` on every platform, so the stream lives on its own thread.)
pub fn spawn_loopback_capture(
    senders: Vec<LiveFrameSender>,
    fmt: SourceFormat,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        crate::prio::audio_thread();
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            tracing::warn!("capture: no default output device");
            return;
        };
        let config = cpal::StreamConfig {
            channels: fmt.channels,
            sample_rate: cpal::SampleRate(fmt.rate),
            buffer_size: cpal::BufferSize::Default,
        };
        // A capture stream that errors (device unplugged, format changed, driver reset) never recovers by itself.
        // Remember it so the thread ends and the engine can rebuild the whole session.
        let broken = Arc::new(AtomicBool::new(false));
        let err_fn = {
            let broken = broken.clone();
            move |e| {
                tracing::warn!("capture stream error: {e}");
                broken.store(true, Ordering::Relaxed);
            }
        };
        let device_name = device.name().unwrap_or_default();

        let shared = Arc::new(Mutex::new(Shared {
            chunker: Chunker::new(senders, fmt.rate),
            last_real: Instant::now(),
            silence_upto: Instant::now(),
        }));
        let ch = fmt.channels as usize;

        // Building an *input* stream on an *output* device = WASAPI loopback.
        let stream = match fmt.sample_format {
            cpal::SampleFormat::F32 => {
                let shared = shared.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _| {
                        let pcm = to_stereo_i16(data.iter().map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16), ch);
                        shared.lock().unwrap().push_real(pcm);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let shared = shared.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        let pcm = to_stereo_i16(data.iter().copied(), ch);
                        shared.lock().unwrap().push_real(pcm);
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                tracing::warn!("capture: unsupported sample format {other:?}");
                return;
            }
        };

        match stream {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    tracing::warn!("capture: play failed: {e}");
                    return;
                }
                // WASAPI loopback delivers nothing while the PC is silent. A real-time AirPlay
                // stream must never stall, so fill the gaps with digital silence.
                let mut last_check = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                    shared.lock().unwrap().fill_silence(fmt.rate);
                    if broken.load(Ordering::Relaxed) {
                        break;
                    }
                    // Windows switched the default output (monitor/headset slept or woke, device replaced):
                    // the loopback we hold is now attached to a device nobody plays on.
                    if last_check.elapsed() >= Duration::from_secs(2) {
                        last_check = Instant::now();
                        let now = host.default_output_device().and_then(|d| d.name().ok()).unwrap_or_default();
                        if now != device_name {
                            tracing::warn!("default output changed ({device_name} -> {now}); capture ends");
                            break;
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("capture: build stream failed: {e}"),
        }
    })
}
