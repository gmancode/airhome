//! Background worker: discovery, connection and streaming. Talks to the UI over channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use airplay_audio::{AlacEncoder, LiveAudioDecoder};
use airplay_client::{AirPlayClient, Connection};
use airplay_core::stream::{PtpMode, StreamType, TimingProtocol};
use airplay_core::{AudioCodec, AudioFormat, Device, StreamConfig};
use anyhow::Result;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::capture::{self, SourceFormat};
use crate::prio;

/// What the UI asks the engine to do.
pub enum Cmd {
    Scan,
    /// `indexes[0]` is the leader; the rest play in sync with it.
    Start { indexes: Vec<usize>, volumes: Vec<f32>, mute_pc: bool },
    Stop,
    /// One volume per speaker, in the same order as `Start::indexes`.
    SetVolume(Vec<f32>),
    /// Turn "mute this PC while streaming" on/off while a stream is running.
    SetMutePc(bool),
    /// Speakers added by IP address (for networks where mDNS discovery does not work). Applied on next scan.
    SetManual(Vec<String>),
    /// Tear everything down and end the worker.
    Quit,
}

#[derive(Clone, Debug)]
pub struct DeviceEntry {
    pub name: String,
    pub model: String,
    pub ip: String,
    pub is_homepod: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Scanning,
    Connecting(String),
    Streaming(String),
    /// The stream dropped; trying to get it back (attempt number, 1-based).
    Reconnecting(String, u32),
    Stopping,
    /// Keep-alive failed repeatedly; the stream was torn down.
    ConnectionLost,
    Error(String),
}

/// What the engine reports back.
pub enum Event {
    Devices(Vec<DeviceEntry>),
    Status(Status),
    /// Source description, e.g. "Speakers (48000 Hz, 2 ch)".
    Source(String),
}

pub struct Engine {
    cmd_tx: UnboundedSender<Cmd>,
    pub events: mpsc::Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn spawn(ctx: egui::Context, manual: Vec<String>) -> Self {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    // The packet pacing runs on these threads: give them real-time-ish scheduling.
                    .on_thread_start(crate::prio::audio_thread)
                    .build()
                    .expect("tokio runtime");
                rt.block_on(worker(cmd_rx, Notifier { tx: ev_tx, ctx }, manual));
            })
            .expect("engine thread");
        Self { cmd_tx, events: ev_rx, thread: Some(thread) }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.cmd_tx.send(cmd);
    }
}

impl Drop for Engine {
    /// Make sure the speaker gets a proper TEARDOWN before the process exits.
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Notifier {
    tx: mpsc::Sender<Event>,
    ctx: egui::Context,
}

impl Notifier {
    fn emit(&self, ev: Event) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
    fn status(&self, s: Status) {
        self.emit(Event::Status(s));
    }
}

/// Owns the capture thread. Dropping it (e.g. when a connection attempt is cancelled midway) tells the
/// thread to stop, so no orphaned capture keeps running.
struct CaptureGuard {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl CaptureGuard {
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    /// The capture thread ended on its own (lost device, stream error): no audio will ever arrive again.
    fn is_dead(&self) -> bool {
        self.handle.as_ref().map_or(false, |h| h.is_finished())
    }
}

/// Mute the PC's own output (so only the speaker plays). Failure is not fatal: the stream still works.
fn engage_mute() -> Option<crate::pcmute::Muter> {
    match crate::pcmute::engage() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("could not mute PC sound: {e}");
            None
        }
    }
}

/// A running stream to one speaker, or to a group of speakers playing in sync.
struct Session {
    /// `conns[0]` is the group leader (it provides the PTP clock).
    conns: Vec<Connection>,
    capture: CaptureGuard,
    feedback_failures: u32,
    /// Present while the PC's output device is muted by us.
    muter: Option<crate::pcmute::Muter>,
}

impl Session {
    async fn shutdown(mut self) {
        // Give the PC its sound back first: it is the user-visible side effect we must never leave behind.
        if let Some(m) = self.muter.take() {
            m.restore();
        }
        self.capture.stop_and_join();
        // Followers first, the clock-providing leader last.
        for conn in self.conns.iter_mut().rev() {
            let _ = conn.stop().await;
            let _ = conn.disconnect().await;
        }
    }

    async fn set_volumes(&mut self, volumes: &[f32]) {
        let fallback = volumes.last().copied().unwrap_or(0.4);
        for (i, c) in self.conns.iter_mut().enumerate() {
            let _ = c.set_volume(volumes.get(i).copied().unwrap_or(fallback)).await;
        }
    }
}

/// Connect to every device, set up timing (leader owns the PTP clock, followers share it).
/// Connections are pushed into `conns` as they come up so the caller can clean up after a failure.
async fn connect_group(conns: &mut Vec<Connection>, devices: &[Device], config: &StreamConfig) -> Result<()> {
    for d in devices {
        conns.push(Connection::connect_auto(d.clone(), config.clone(), "3939").await?);
    }
    conns[0].setup().await?;
    if conns.len() > 1 {
        // Every member must know all the others (and us) to play in sync.
        let mut peers: Vec<String> = devices
            .iter()
            .filter_map(|d| d.addresses.iter().find(|a| a.is_ipv4()).or(d.addresses.first()).map(|a| a.to_string()))
            .collect();
        if let Some(local) = conns[0].local_addr() {
            peers.push(local.ip().to_string());
        }
        conns[0].send_setpeers(&peers).await?;
        let clock = conns[0].ptp_master_clock_id().ok_or_else(|| anyhow::anyhow!("leader has no PTP clock"))?;
        let offset = conns[0].timing_offset().ok_or_else(|| anyhow::anyhow!("leader has no timing offset"))?;
        for i in 1..conns.len() {
            let rx = conns[0].timing_rx().ok_or_else(|| anyhow::anyhow!("leader has no timing channel"))?;
            conns[i].setup_for_group(clock, offset, rx).await?;
            conns[i].send_setpeers(&peers).await?;
        }
    }
    Ok(())
}

async fn start_session(devices: Vec<Device>, volumes: Vec<f32>, src: SourceFormat, mute_pc: bool) -> Result<Session> {
    // PTP + "master" role triggers the BMCA yield to the HomePod clock (NTP does not work on HomePod mini).
    let audio_format = AudioFormat::default();
    let asc = if audio_format.codec == AudioCodec::Alac {
        Some(AlacEncoder::new(audio_format.clone())?.magic_cookie())
    } else {
        None
    };
    let config = StreamConfig {
        stream_type: StreamType::Realtime,
        audio_format,
        timing_protocol: TimingProtocol::Ptp,
        ptp_mode: PtpMode::Master,
        latency_min: 22050,
        latency_max: 88200,
        supports_dynamic_stream_id: true,
        asc,
    };

    let mut conns: Vec<Connection> = Vec::new();
    if let Err(e) = connect_group(&mut conns, &devices, &config).await {
        for c in conns.iter_mut().rev() {
            let _ = c.disconnect().await;
        }
        return Err(e);
    }

    // Set the speakers' volume BEFORE any audio flows. The library re-sends its stored volume (default: 100 %)
    // when streaming starts, so without this every connect would begin with a burst at full volume.
    let fallback = volumes.last().copied().unwrap_or(0.4);
    for (i, c) in conns.iter_mut().enumerate() {
        let _ = c.set_volume(volumes.get(i).copied().unwrap_or(fallback)).await;
    }

    // Start capturing BEFORE the streamers start: they wait for the first frames to fill their buffers.
    // Small queue (~170 ms) per speaker so a slow consumer can't pile up seconds of latency.
    let mut senders = Vec::new();
    let mut decoders = Vec::new();
    for _ in &conns {
        let (sender, decoder) = LiveAudioDecoder::create_pair(src.rate, 2, 8);
        senders.push(sender);
        decoders.push(decoder);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let handle = capture::spawn_loopback_capture(senders, src, stop.clone());
    let mut capture = CaptureGuard { stop, handle: Some(handle) };
    for (conn, decoder) in conns.iter_mut().zip(decoders) {
        if let Err(e) = conn.start_streaming_live(decoder).await {
            capture.stop_and_join();
            for c in conns.iter_mut().rev() {
                let _ = c.disconnect().await;
            }
            return Err(e.into());
        }
    }
    let mut session = Session { conns, capture, feedback_failures: 0, muter: None };
    session.set_volumes(&volumes).await;
    // Only after the stream is really running, so a failed/cancelled connect never silences the PC.
    session.muter = if mute_pc { engage_mute() } else { None };
    Ok(session)
}

/// What woke the worker loop.
enum Wake {
    Cmd(Option<Cmd>),
    Feedback,
}

/// Result of a cancellable connection attempt.
enum Attempt {
    Connected(Session),
    Failed(anyhow::Error),
    Cancelled,
    Quit,
}

/// Connect while still listening for commands, so the attempt can be cancelled (Stop) or abandoned (Quit).
/// Dropping the future aborts the handshake.
async fn connect_cancellable(
    cmd_rx: &mut UnboundedReceiver<Cmd>,
    devices: Vec<Device>,
    volumes: Vec<f32>,
    src: SourceFormat,
    mute_pc: bool,
) -> Attempt {
    let attempt = start_session(devices, volumes, src, mute_pc);
    tokio::pin!(attempt);
    loop {
        tokio::select! {
            res = &mut attempt => return match res {
                Ok(s) => Attempt::Connected(s),
                Err(e) => Attempt::Failed(e),
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Stop) => return Attempt::Cancelled,
                Some(Cmd::Quit) | None => return Attempt::Quit,
                Some(_) => {}
            },
        }
    }
}

/// Wait `d`, but wake early on Stop/Quit. Returns `Some(Attempt::..)` when interrupted.
async fn pause_cancellable(cmd_rx: &mut UnboundedReceiver<Cmd>, d: Duration) -> Option<Attempt> {
    let sleep = tokio::time::sleep(d);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return None,
            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Stop) => return Some(Attempt::Cancelled),
                Some(Cmd::Quit) | None => return Some(Attempt::Quit),
                Some(_) => {}
            },
        }
    }
}

/// Errors worth one quick retry (timeouts), as opposed to real refusals such as a wrong password.
fn is_transient(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("timed out") || m.contains("timeout")
}

/// Delays between reconnect attempts after a dropped stream.
const RECONNECT_DELAYS: [u64; 5] = [2, 4, 8, 12, 20];

/// A speaker given only by IP address: pretend to be what a HomePod advertises. The real capabilities are
/// negotiated during the handshake, so this is enough to connect.
fn manual_device(addr: std::net::SocketAddr) -> Device {
    let ip = addr.ip();
    use airplay_core::{DeviceId, Features, Version};
    let mac = match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            [0x02, 0x00, o[0], o[1], o[2], o[3]] // locally administered, unique per IP
        }
        std::net::IpAddr::V6(v6) => {
            let o = v6.octets();
            [0x02, 0x00, o[12], o[13], o[14], o[15]]
        }
    };
    Device {
        id: DeviceId(mac),
        name: if addr.port() == 7000 { format!("HomePod ({ip})") } else { format!("HomePod ({addr})") },
        model: "AudioAccessory".to_string(),
        manufacturer: Some("manual".to_string()),
        serial_number: None,
        addresses: vec![ip],
        port: addr.port(),
        features: Features::from_txt_value("0x4A7FCA00,0x3C354BD0").unwrap_or_default(),
        required_sender_features: None,
        public_key: None,
        source_version: Version::new(660, 8, 1),
        firmware_version: None,
        os_version: None,
        protocol_version: None,
        requires_password: false,
        status_flags: 0,
        access_control: None,
        pairing_identity: None,
        system_pairing_identity: None,
        bluetooth_address: None,
        homekit_home_id: None,
        group_id: None,
        is_group_leader: false,
        group_public_name: None,
        group_contains_discoverable_leader: false,
        home_group_id: None,
        household_id: None,
        parent_group_id: None,
        parent_group_contains_discoverable_leader: false,
        tight_sync_id: None,
        raop_port: None,
        raop_encryption_types: None,
        raop_codecs: None,
        raop_transport: None,
        raop_metadata_types: None,
        raop_digest_auth: false,
        vodka_version: None,
    }
}

/// "192.168.1.50" (port 7000 assumed) or "192.168.1.50:7000" / "[fe80::1]:7000".
pub fn parse_addr(s: &str) -> Option<std::net::SocketAddr> {
    let s = s.trim();
    if let Ok(a) = s.parse::<std::net::SocketAddr>() {
        return (a.port() != 0).then_some(a);
    }
    s.parse::<std::net::IpAddr>().ok().map(|ip| std::net::SocketAddr::new(ip, 7000))
}

fn parse_manual(list: &[String]) -> Vec<Device> {
    list.iter()
        .filter_map(|s| parse_addr(s))
        .map(manual_device)
        .collect()
}

async fn worker(mut cmd_rx: UnboundedReceiver<Cmd>, n: Notifier, manual: Vec<String>) {
    let mut manual: Vec<Device> = parse_manual(&manual);
    let client = match AirPlayClient::new() {
        Ok(c) => c,
        Err(e) => {
            n.status(Status::Error(format!("mDNS init failed: {e}")));
            return;
        }
    };
    let mut devices: Vec<Device> = Vec::new();
    let mut misses: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut session: Option<Session> = None;
    /// What the current/last stream was started with, so it can be re-established.
    let mut last: Option<(Vec<Device>, Vec<f32>, bool)> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(4));

    if let Ok((name, f)) = capture::default_output_format() {
        let khz = if f.rate % 1000 == 0 { format!("{}", f.rate / 1000) } else { format!("{:.1}", f.rate as f32 / 1000.0) };
        n.emit(Event::Source(format!("{name} · {khz} kHz")));
    }
    scan(&client, &manual, &mut devices, &mut misses, &n).await;

    'main: loop {
        let wake = tokio::select! {
            cmd = cmd_rx.recv() => Wake::Cmd(cmd),
            _ = tick.tick(), if session.is_some() => Wake::Feedback,
        };

        match wake {
            Wake::Cmd(None) | Wake::Cmd(Some(Cmd::Quit)) => break,
            Wake::Cmd(Some(cmd)) => match cmd {
                Cmd::Scan => {
                    if session.is_none() {
                        scan(&client, &manual, &mut devices, &mut misses, &n).await;
                    }
                }
                Cmd::Start { indexes, volumes, mute_pc } => {
                    if session.is_some() {
                        continue;
                    }
                    let group: Vec<Device> = indexes.iter().filter_map(|&i| devices.get(i).cloned()).collect();
                    if group.is_empty() {
                        continue;
                    }
                    let name = group.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(" + ");
                    tracing::info!("start requested: {name}");
                    n.status(Status::Connecting(name.clone()));
                    let src = match capture::default_output_format() {
                        Ok((_, f)) => f,
                        Err(e) => {
                            n.status(Status::Error(e.to_string()));
                            continue;
                        }
                    };

                    last = Some((group.clone(), volumes.clone(), mute_pc));
                    let mut result = connect_cancellable(&mut cmd_rx, group.clone(), volumes.clone(), src, mute_pc).await;
                    // A speaker that was busy or half-asleep often times out once and works right after.
                    if let Attempt::Failed(e) = &result {
                        if is_transient(&format!("{e}")) {
                            tracing::warn!("connect failed ({e}); retrying once");
                            result = match pause_cancellable(&mut cmd_rx, Duration::from_millis(750)).await {
                                Some(interrupted) => interrupted,
                                None => connect_cancellable(&mut cmd_rx, group, volumes, src, mute_pc).await,
                            };
                        }
                    }
                    match result {
                        Attempt::Connected(s) => {
                            session = Some(s);
                            prio::process_boost(true);
                            tracing::info!("streaming: {name}");
                            n.status(Status::Streaming(name));
                        }
                        Attempt::Failed(e) => {
                            tracing::warn!("connect failed: {e}");
                            n.status(Status::Error(format!("{e}")))
                        }
                        Attempt::Cancelled => {
                            tracing::info!("connect cancelled");
                            n.status(Status::Idle)
                        }
                        Attempt::Quit => break 'main,
                    }
                }
                Cmd::Stop => {
                    if let Some(s) = session.take() {
                        tracing::info!("stop requested");
                        n.status(Status::Stopping);
                        s.shutdown().await;
                        prio::process_boost(false);
                        n.status(Status::Idle);
                    }
                }
                Cmd::SetVolume(v) => {
                    if let Some(l) = last.as_mut() {
                        l.1 = v.clone(); // a reconnect must come back at the current volume
                    }
                    if let Some(s) = session.as_mut() {
                        s.set_volumes(&v).await;
                    }
                }
                Cmd::SetManual(list) => manual = parse_manual(&list),
                Cmd::SetMutePc(on) => {
                    if let Some(s) = session.as_mut() {
                        if on && s.muter.is_none() {
                            s.muter = engage_mute();
                        } else if !on {
                            if let Some(m) = s.muter.take() {
                                m.restore();
                            }
                        }
                    }
                }
                Cmd::Quit => break,
            },
            Wake::Feedback => {
                let failed = match session.as_mut() {
                    Some(s) if s.capture.is_dead() => {
                        tracing::warn!("audio capture stopped; rebuilding the stream");
                        true
                    }
                    Some(s) => {
                        // Healthy only if every speaker of the group answers.
                        let mut all_ok = true;
                        for c in &mut s.conns {
                            if let Err(e) = c.send_feedback().await {
                                tracing::warn!("keepalive failed ({}/3): {e}", s.feedback_failures + 1);
                                all_ok = false;
                            }
                        }
                        if all_ok {
                            s.feedback_failures = 0;
                            false
                        } else {
                            s.feedback_failures += 1;
                            s.feedback_failures >= 3
                        }
                    }
                    None => false,
                };
                if failed {
                    tracing::warn!("stream lost; reconnecting");
                    if let Some(s) = session.take() {
                        s.shutdown().await;
                    }
                    // Wi-Fi blip or the speaker restarted: try to bring the stream back by itself.
                    let Some((group, volumes, mute_pc)) = last.clone() else {
                        prio::process_boost(false);
                        n.status(Status::ConnectionLost);
                        continue;
                    };
                    let name = group.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(" + ");
                    let mut outcome: Option<Status> = Some(Status::ConnectionLost);
                    for (i, delay) in RECONNECT_DELAYS.iter().enumerate() {
                        n.status(Status::Reconnecting(name.clone(), i as u32 + 1));
                        match pause_cancellable(&mut cmd_rx, Duration::from_secs(*delay)).await {
                            Some(Attempt::Quit) => break 'main,
                            Some(_) => {
                                outcome = Some(Status::Idle);
                                break;
                            }
                            None => {}
                        }
                        let Ok((_, src)) = capture::default_output_format() else { continue };
                        match connect_cancellable(&mut cmd_rx, group.clone(), volumes.clone(), src, mute_pc).await {
                            Attempt::Connected(s) => {
                                session = Some(s);
                                outcome = None;
                                break;
                            }
                            Attempt::Failed(e) => tracing::warn!("reconnect attempt {} failed: {e}", i + 1),
                            Attempt::Cancelled => {
                                outcome = Some(Status::Idle);
                                break;
                            }
                            Attempt::Quit => break 'main,
                        }
                    }
                    match outcome {
                        None => {
                            tracing::info!("reconnected: {name}");
                            n.status(Status::Streaming(name))
                        }
                        Some(s) => {
                            tracing::warn!("reconnect gave up");
                            prio::process_boost(false);
                            n.status(s);
                        }
                    }
                }
            }
        }
    }

    tracing::info!("engine exiting");
    if let Some(s) = session.take() {
        s.shutdown().await;
    }
    prio::process_boost(false);
}

/// A speaker seen earlier stays in the list for this many consecutive scans that miss it (mDNS replies
/// are best-effort: a busy speaker can easily skip one 4-second window).
const CACHE_MISSES: u32 = 3;

async fn scan(
    client: &AirPlayClient,
    manual: &[Device],
    devices: &mut Vec<Device>,
    misses: &mut std::collections::HashMap<String, u32>,
    n: &Notifier,
) {
    n.status(Status::Scanning);
    match client.discover(Duration::from_secs(4)).await {
        Ok(found) => {
            let mut found: Vec<Device> = found.into_iter().filter(|d| !d.addresses.is_empty()).collect();
            // HomePods first, then by name.
            found.sort_by_key(|d| (!d.model.starts_with("AudioAccessory"), d.name.clone()));
            // Add manually entered speakers that discovery did not find.
            for m in manual {
                if !found.iter().any(|d| d.addresses.iter().any(|a| m.addresses.contains(a))) {
                    found.push(m.clone());
                }
            }
            // Keep speakers that were found before but didn't answer this time (until they miss several scans).
            let seen_now: std::collections::HashSet<String> = found.iter().map(|d| d.name.clone()).collect();
            misses.retain(|name, _| !seen_now.contains(name));
            for old in devices.iter() {
                // manual speakers come from the settings list, never from the cache
                if seen_now.contains(&old.name) || old.manufacturer.as_deref() == Some("manual") {
                    continue;
                }
                let m = misses.entry(old.name.clone()).or_insert(0);
                *m += 1;
                if *m < CACHE_MISSES {
                    found.push(old.clone());
                }
            }
            found.sort_by_key(|d| (!d.model.starts_with("AudioAccessory"), d.name.clone()));
            let entries = found
                .iter()
                .map(|d| DeviceEntry {
                    name: d.name.clone(),
                    model: d.model.clone(),
                    ip: d
                        .addresses
                        .iter()
                        .find(|a| a.is_ipv4())
                        .or(d.addresses.first())
                        .map(|a| a.to_string())
                        .unwrap_or_default(),
                    is_homepod: d.model.starts_with("AudioAccessory"),
                })
                .collect();
            *devices = found;
            n.emit(Event::Devices(entries));
            n.status(Status::Idle);
        }
        Err(e) => n.status(Status::Error(format!("Scan failed: {e}"))),
    }
}
