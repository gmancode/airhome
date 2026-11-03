// No console window in release builds (logs go to %LOCALAPPDATA%\airhome\airhome.log).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! airhome: stream Windows system audio (WASAPI loopback) to a HomePod over AirPlay 2.

mod autostart;
mod capture;
mod engine;
mod i18n;
mod notify;
mod pcmute;
mod prio;
mod settings;
mod single;
mod tray;
mod widgets;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, Margin, Rect, RichText, Sense};

use engine::{Cmd, DeviceEntry, Engine, Event, Status};
use i18n::{t, tf, Lang, K};
use settings::Settings;
use tray::{Tray, TrayCmd, TrayLabels, TrayState};
use widgets::*;

/// How many times to retry finding the saved speaker at startup (network may still be coming up).
const AUTO_CONNECT_ATTEMPTS: u32 = 6;
const AUTO_CONNECT_RETRY: Duration = Duration::from_secs(8);

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period_ms: u32) -> u32;
}

/// Log writer that starts a new file each day (`airhome-YYYY-MM-DD.log`) and keeps the last week.
struct DailyLog {
    dir: std::path::PathBuf,
    day: String,
    file: Option<std::fs::File>,
}

const LOG_DAYS_KEPT: usize = 7;

fn today() -> String {
    #[cfg(windows)]
    unsafe {
        let t = windows::Win32::System::SystemInformation::GetLocalTime();
        format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
    }
    #[cfg(not(windows))]
    {
        String::from("log")
    }
}

impl DailyLog {
    fn new(dir: std::path::PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let mut log = Self { dir, day: String::new(), file: None };
        log.roll();
        log
    }

    /// Open today's file (append) if the date changed, and delete files older than the retention window.
    fn roll(&mut self) {
        let day = today();
        if day == self.day && self.file.is_some() {
            return;
        }
        self.day = day.clone();
        self.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(format!("airhome-{day}.log")))
            .ok();
        // Retention: file names sort by date, so drop everything but the newest few.
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            let mut logs: Vec<std::path::PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("airhome-") && n.ends_with(".log"))
                        .unwrap_or(false)
                })
                .collect();
            logs.sort();
            while logs.len() > LOG_DAYS_KEPT {
                let _ = std::fs::remove_file(logs.remove(0));
            }
        }
        // the old single-file log from earlier versions
        let _ = std::fs::remove_file(self.dir.join("airhome.log"));
    }
}

impl std::io::Write for DailyLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.roll();
        match self.file.as_mut() {
            Some(f) => f.write(buf),
            None => Ok(buf.len()), // can't open the log: drop the line rather than fail the program
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// Made-up speakers for documentation screenshots.
fn demo_device_list() -> Vec<DeviceEntry> {
    let d = |name: &str, model: &str, ip: &str, is_homepod: bool| DeviceEntry {
        name: name.into(),
        model: model.into(),
        ip: ip.into(),
        is_homepod,
    };
    vec![
        d("Living Room", "AudioAccessory5,1", "192.168.1.20", true),
        d("Kitchen", "AudioAccessory5,1", "192.168.1.21", true),
        d("Bedroom", "AppleTV6,2", "192.168.1.30", false),
        d("TV", "QN90B", "192.168.1.40", false),
    ]
}

fn tray_labels(l: Lang) -> TrayLabels<'static> {
    TrayLabels {
        show: t(l, K::ShowWindow),
        start: t(l, K::Start),
        stop: t(l, K::Stop),
        quit: t(l, K::Quit),
        volume: t(l, K::VolShort),
        louder: t(l, K::VolUp),
        quieter: t(l, K::VolDown),
    }
}

fn init_logging() {
    // Library warnings (per-packet jitter) are very chatty; keep only real problems by default.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "warn,airplay_audio=error".into());

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(Mutex::new(DailyLog::new(settings::log_dir())))
        .init();
    tracing::warn!("---- airhome v{} started ----", env!("CARGO_PKG_VERSION"));
}

/// Developer switches: `--hidden`, `--page=settings`, `--lang=ja` (never saved to the settings file).
#[derive(Default, Clone)]
struct Args {
    hidden: bool,
    page: Option<Page>,
    lang: Option<Lang>,
    height: Option<f32>,
    /// Fake a state (`streaming`, `connecting`, `error`, `lost`) to review the look without a speaker.
    demo: Option<&'static str>,
    /// Developer switch: allow several instances at once (screenshots, tests).
    multi: bool,
    /// Developer switch: show a test notification and exit.
    toast_test: bool,
    /// Developer switch: show made-up speakers (for screenshots without private names and addresses).
    demo_devices: bool,
}

fn parse_args() -> Args {
    let mut a = Args::default();
    for arg in std::env::args().skip(1) {
        if arg == "--hidden" {
            a.hidden = true;
        } else if arg == "--multi" {
            a.multi = true;
        } else if arg == "--demo-devices" {
            a.demo_devices = true;
        } else if arg == "--toast-test" {
            a.toast_test = true;
        } else if let Some(p) = arg.strip_prefix("--page=") {
            a.page = match p {
                "settings" => Some(Page::Settings),
                "main" => Some(Page::Main),
                _ => None,
            };
        } else if let Some(l) = arg.strip_prefix("--lang=") {
            a.lang = Lang::from_code(l);
        } else if let Some(h) = arg.strip_prefix("--height=") {
            a.height = h.parse().ok();
        } else if let Some(d) = arg.strip_prefix("--demo=") {
            a.demo = ["streaming", "connecting", "error", "lost"].into_iter().find(|s| *s == d);
        }
    }
    a
}

fn main() -> eframe::Result {
    // Windows timers tick every 15.6 ms by default, which wrecks the 8 ms packet pacing.
    #[cfg(windows)]
    unsafe {
        timeBeginPeriod(1);
    }
    let args = parse_args();
    if args.toast_test {
        let r = notify::toast_blocking("airhome", "Test: ошибка подключения к колонке");
        if let Ok(path) = std::env::var("AIRHOME_TOAST_RESULT") {
            let _ = std::fs::write(path, format!("{r:?}"));
        }
        std::thread::sleep(Duration::from_secs(6));
        return Ok(());
    }

    // Only one copy may run: a second launch just asks the first one to show its window.
    // (Checked before touching the log file, which the running copy owns.)
    if !args.multi {
        if let single::Role::Second = single::acquire(args.hidden) {
            return Ok(());
        }
    }
    init_logging();
    // Make a crash leave a trace in the log (the PC's sound is restored on the next start if needed).
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("panic: {info}");
    }));
    // If a previous run died while the PC's sound was muted, give it back now.
    pcmute::recover();
    tracing::debug!(
        "started with args: {:?} (parsed: hidden={}, page_settings={}, lang={:?})",
        std::env::args().skip(1).collect::<Vec<_>>(),
        args.hidden,
        args.page == Some(Page::Settings),
        args.lang.map(|l| l.code())
    );

    let (rgba, w, h) = tray::render_logo(128, TrayState::Idle);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([440.0, args.height.unwrap_or(720.0)])
            .with_min_inner_size([400.0, 560.0])
            .with_visible(!args.hidden)
            .with_icon(egui::IconData { rgba, width: w, height: h }),
        ..Default::default()
    };
    eframe::run_native("airhome", options, Box::new(move |cc| Ok(Box::new(App::new(cc, args)))))
}

// ---------------------------------------------------------------- look & feel

fn apply_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(12, 13, 16);
    v.faint_bg_color = Color32::from_rgb(30, 33, 40);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = ACCENT;
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.text_styles.insert(egui::TextStyle::Heading, bold(22.0));
        s.text_styles.insert(egui::TextStyle::Body, regular(14.5));
        s.text_styles.insert(egui::TextStyle::Button, regular(14.5));
        s.text_styles.insert(egui::TextStyle::Small, regular(12.0));
        s.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.5));
    });
}

/// Typography: Segoe UI (ships with Windows) as the main face, Segoe UI Semibold for emphasis, and the
/// system CJK fonts as fallbacks (egui's built-in fonts have no CJK glyphs). Chinese and Japanese share
/// ideographs but draw them differently, so the matching font goes first.
fn apply_fonts(ctx: &egui::Context, lang: Lang) {
    use egui::{FontData, FontFamily};

    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts");
    // First existing file of the list, with its name.
    let load_named = |names: &[&str]| {
        names
            .iter()
            .find_map(|f| std::fs::read(dir.join(f)).ok().map(|b| (f.to_string(), b)))
    };
    let load = |names: &[&str]| load_named(names).map(|(_, b)| b);

    let base: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();

    // For Chinese/Japanese the CJK face leads for *all* text. Mixing it with Segoe UI inside one line gives
    // mismatched baselines and glyph sizes, so native apps use one family per line. (The CJK fonts are big,
    // so they are loaded only when the UI language needs them.)
    let (cjk_regular, cjk_bold) = match lang {
        Lang::Ja => (
            load_named(&["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"]),
            load_named(&["YuGothB.ttc", "meiryob.ttc"]),
        ),
        Lang::Zh => (
            load_named(&["msyh.ttc", "msyh.ttf", "simsun.ttc"]),
            load_named(&["msyhbd.ttc", "msyhbd.ttf"]),
        ),
        _ => (None, None),
    };

    // Yu Gothic's default face draws Cyrillic/Latin full-width; the collection also holds the proportional
    // "Yu Gothic UI" face (index 1) made for interfaces. Use it when it really is a 2+ face collection.
    let font_data = |name: &str, bytes: Vec<u8>| -> Arc<FontData> {
        let faces = if bytes.len() > 12 && &bytes[0..4] == b"ttcf" {
            u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]])
        } else {
            0
        };
        let mut data = FontData::from_owned(bytes);
        if name.starts_with("YuGoth") && faces >= 2 {
            data.index = 1;
        }
        Arc::new(data)
    };

    let mut regular_list: Vec<String> = Vec::new();
    let mut bold_list: Vec<String> = Vec::new();

    if let Some((name, bytes)) = cjk_regular {
        fonts.font_data.insert("cjk".into(), font_data(&name, bytes));
        regular_list.push("cjk".into());
        bold_list.push("cjk".into()); // fallback if no bold face exists
    }
    if let Some((name, bytes)) = cjk_bold {
        fonts.font_data.insert("cjk_bold".into(), font_data(&name, bytes));
        bold_list.insert(0, "cjk_bold".into());
    }

    if let Some(bytes) = load(&["segoeui.ttf"]) {
        fonts.font_data.insert("ui".into(), Arc::new(FontData::from_owned(bytes)));
        regular_list.push("ui".into());
    }
    if let Some(bytes) = load(&["seguisb.ttf", "segoeuib.ttf"]) {
        fonts.font_data.insert("ui_bold".into(), Arc::new(FontData::from_owned(bytes)));
        bold_list.push("ui_bold".into());
    } else if fonts.font_data.contains_key("ui") {
        bold_list.push("ui".into());
    }
    regular_list.extend(base.iter().cloned());
    bold_list.extend(base.iter().cloned());

    // The language picker shows "中文" and "日本語" whatever the UI language is, so every other language
    // also gets one CJK face — but only as a last-resort fallback for those few glyphs.
    if !matches!(lang, Lang::Zh | Lang::Ja) {
        if let Some((name, bytes)) = load_named(&["YuGothR.ttc", "meiryo.ttc", "msyh.ttc", "msgothic.ttc"]) {
            fonts.font_data.insert("cjk_fallback".into(), font_data(&name, bytes));
            regular_list.push("cjk_fallback".into());
            bold_list.push("cjk_fallback".into());
        }
    }

    fonts.families.insert(FontFamily::Proportional, regular_list);
    fonts.families.insert(FontFamily::Name("bold".into()), bold_list);
    ctx.set_fonts(fonts);
}

fn open_folder(path: PathBuf) {
    let _ = std::fs::create_dir_all(&path);
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

// ---------------------------------------------------------------- app

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Settings,
}

struct App {
    engine: Engine,
    tray: Option<Tray>,
    devices: Vec<DeviceEntry>,
    selected: Option<usize>,
    status: Status,
    source: String,
    volume: f32,
    /// Smoothed 0..1 output level for the meter.
    level: f32,
    /// Last volume actually sent to the speaker.
    sent_volume: f32,
    /// Set when the user chose "Quit" so that closing the window really exits.
    quitting: bool,
    window_hidden: bool,
    settings: Settings,
    lang: Lang,
    page: Page,
    /// Mirrors the registry "Run" entry.
    autostart: bool,
    /// Still waiting to auto-connect to the saved speaker after launch.
    auto_pending: bool,
    /// Error shown before a background scan began; put back when the scan ends.
    held_error: Option<Status>,
    /// The user picked another speaker (or changed the group) mid-stream: restart after the stop completes.
    restart_pending: bool,
    scan_attempts: u32,
    next_scan: Option<Instant>,
    demo: Option<&'static str>,
    demo_devices: bool,
    /// Fired by a second launch of the program ("show your window").
    show_rx: std::sync::mpsc::Receiver<()>,
    /// Text field for adding a speaker by IP, and whether the last entry was rejected.
    manual_input: String,
    manual_port: String,
    manual_invalid: bool,
    manual_open: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let mut settings = Settings::load();
        let saved_lang = settings.language.as_deref().and_then(Lang::from_code);
        if saved_lang.is_none() {
            let detected = Lang::detect();
            settings.language = Some(detected.code().to_string());
            settings.save();
        }
        let lang = args
            .lang
            .or(saved_lang)
            .or_else(|| settings.language.as_deref().and_then(Lang::from_code))
            .unwrap_or(Lang::En);

        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        apply_style(&cc.egui_ctx);
        apply_fonts(&cc.egui_ctx, lang);
        autostart::refresh();

        let tray = match Tray::new(cc.egui_ctx.clone(), &tray_labels(lang), settings.volume) {
            Ok(tr) => Some(tr),
            Err(e) => {
                tracing::error!("tray icon failed: {e}");
                None
            }
        };
        let (show_tx, show_rx) = std::sync::mpsc::channel();
        {
            let ctx = cc.egui_ctx.clone();
            single::watch(move || {
                let _ = show_tx.send(());
                ctx.request_repaint();
            });
        }
        Self {
            engine: Engine::spawn(cc.egui_ctx.clone(), settings.manual_ips.clone()),
            tray,
            devices: Vec::new(),
            selected: None,
            status: Status::Scanning,
            source: String::new(),
            volume: settings.volume,
            level: 0.0,
            sent_volume: settings.volume,
            quitting: false,
            window_hidden: args.hidden,
            lang,
            page: args.page.unwrap_or(Page::Main),
            autostart: autostart::is_enabled(),
            auto_pending: settings.auto_connect && settings.device.is_some(),
            held_error: None,
            restart_pending: false,
            scan_attempts: 0,
            next_scan: None,
            demo: args.demo,
            demo_devices: args.demo_devices,
            show_rx,
            manual_input: String::new(),
            manual_port: String::new(),
            manual_invalid: false,
            manual_open: false,
            settings,
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.window_hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn connecting(&self) -> bool {
        matches!(self.status, Status::Connecting(_) | Status::Reconnecting(..))
    }

    fn save_settings(&mut self) {
        self.settings.volume = self.volume;
        self.settings.save();
    }

    fn streaming(&self) -> bool {
        matches!(self.status, Status::Streaming(_))
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Scanning | Status::Connecting(_) | Status::Reconnecting(..) | Status::Stopping)
    }

    fn poll_events(&mut self) {
        while let Ok(ev) = self.engine.events.try_recv() {
            match ev {
                Event::Devices(list) => {
                    // Prefer: current selection, then the saved speaker, then the first HomePod.
                    let prev = self.selected.and_then(|i| self.devices.get(i)).map(|d| d.name.clone());
                    let list = if self.demo_devices { demo_device_list() } else { list };
                    self.devices = list;
                    let by_name = |name: &Option<String>, devs: &[DeviceEntry]| {
                        name.as_ref().and_then(|n| devs.iter().position(|d| &d.name == n))
                    };
                    let saved_idx = by_name(&self.settings.device, &self.devices);
                    self.selected = by_name(&prev, &self.devices)
                        .or(saved_idx)
                        .or_else(|| self.devices.iter().position(|d| d.is_homepod))
                        .or(if self.devices.is_empty() { None } else { Some(0) });

                    if self.auto_pending {
                        self.scan_attempts += 1;
                        if let Some(i) = saved_idx {
                            self.auto_pending = false;
                            self.selected = Some(i);
                            self.start();
                        } else if self.scan_attempts < AUTO_CONNECT_ATTEMPTS {
                            self.next_scan = Some(Instant::now() + AUTO_CONNECT_RETRY);
                        } else {
                            self.auto_pending = false; // give up, the user can press Start
                        }
                    }
                }
                Event::Status(s) => {
                    // Nobody is looking at the window: tell the user about problems with a Windows toast.
                    if self.window_hidden && s != self.status {
                        let l = self.lang;
                        match &s {
                            Status::ConnectionLost => notify::toast("airhome", t(l, K::ConnectionLost)),
                            Status::Error(e) => {
                                notify::toast("airhome", &format!("{}: {}", t(l, K::Error), notify::short(e)))
                            }
                            _ => {}
                        }
                    }
                    match &s {
                        // An error stays on screen until the user does something: a scan finishing
                        // in the background must not wipe it.
                        Status::Scanning if matches!(self.status, Status::Error(_) | Status::ConnectionLost) => {
                            self.held_error = Some(self.status.clone());
                        }
                        Status::Idle if self.held_error.is_some() => {
                            self.status = self.held_error.take().unwrap_or(Status::Idle);
                            continue;
                        }
                        Status::Error(_) | Status::ConnectionLost => {
                            self.held_error = None;
                            self.restart_pending = false;
                        }
                        Status::Connecting(_) | Status::Streaming(_) | Status::Reconnecting(..) => {
                            self.held_error = None;
                        }
                        _ => {}
                    }
                    self.status = s;
                    // Switching speakers while streaming: stopped, now start on the new choice.
                    if self.status == Status::Idle && self.restart_pending {
                        self.restart_pending = false;
                        self.start();
                    }
                }
                Event::Source(s) => {
                    self.source = if self.demo_devices { "Speakers (High Definition Audio) · 48 kHz".to_string() } else { s };
                }
            }
        }
    }

    /// Names of the speakers that play: the main one first, then the group members.
    fn group_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(d) = self.selected.and_then(|i| self.devices.get(i)) {
            names.push(d.name.clone());
        }
        names.extend(self.member_indices().into_iter().filter_map(|i| self.devices.get(i)).map(|d| d.name.clone()));
        names
    }

    /// Volume to send to each speaker: the main volume times that speaker's own factor.
    fn effective_volumes(&self) -> Vec<f32> {
        self.group_names()
            .iter()
            .map(|n| (self.volume * self.settings.speaker_trim.get(n).copied().unwrap_or(1.0)).clamp(0.0, 1.0))
            .collect()
    }

    /// Push the current volumes to the speakers (only matters while streaming).
    fn push_volumes(&mut self) {
        if self.streaming() {
            self.engine.send(Cmd::SetVolume(self.effective_volumes()));
            self.sent_volume = self.volume;
        }
    }

    /// Indexes (in the current list) of the extra speakers that play together with the main one.
    fn member_indices(&self) -> Vec<usize> {
        let main = self.selected;
        self.settings
            .group
            .iter()
            .filter_map(|n| self.devices.iter().position(|d| &d.name == n))
            .filter(|i| Some(*i) != main)
            .collect()
    }

    fn start(&mut self) {
        if let Some(index) = self.selected {
            self.sent_volume = self.volume;
            self.settings.device = self.devices.get(index).map(|d| d.name.clone());
            self.save_settings();
            // Latency mode takes effect at stream start.
            airplay_audio::LIVE_PREFILL_PERCENT.store(self.settings.prefill_percent(), Ordering::Relaxed);
            let mut indexes = vec![index];
            indexes.extend(self.member_indices());
            let volumes = self.effective_volumes();
            self.engine.send(Cmd::Start { indexes, volumes, mute_pc: self.settings.mute_pc });
        }
    }

    /// Volume changed from the tray menu: same effect as moving the slider in the window.
    fn set_volume_from_tray(&mut self, v: f32) {
        self.volume = ((v.clamp(0.0, 1.0) * 100.0).round() / 100.0).clamp(0.0, 1.0);
        self.save_settings();
        self.push_volumes();
    }

    /// Start, or stop — a connection attempt in progress counts as "running" and can be cancelled.
    fn toggle(&mut self) {
        if self.streaming() || self.connecting() {
            self.engine.send(Cmd::Stop);
        } else if !self.busy() {
            self.start();
        }
    }

    fn set_language(&mut self, ctx: &egui::Context, lang: Lang) {
        self.lang = lang;
        self.settings.language = Some(lang.code().to_string());
        self.save_settings();
        apply_fonts(ctx, lang);
        if let Some(tr) = self.tray.as_mut() {
            tr.set_labels(&tray_labels(lang));
        }
    }

    /// Localized status line and its colour.
    fn status_line(&self) -> (String, Color32) {
        let l = self.lang;
        match &self.status {
            Status::Idle => (t(l, K::Ready).to_string(), TEXT2),
            Status::Scanning => (t(l, K::StatusSearching).to_string(), TEXT2),
            Status::Connecting(n) => (tf(l, K::Connecting, n), WARN),
            Status::Streaming(n) => (tf(l, K::Streaming, n), GOOD),
            Status::Reconnecting(n, _) => (tf(l, K::Reconnecting, n), WARN),
            Status::Stopping => (t(l, K::Stopping).to_string(), TEXT2),
            Status::ConnectionLost => (format!("{}: {}", t(l, K::Error), t(l, K::ConnectionLost)), BAD),
            Status::Error(e) => (format!("{}: {e}", t(l, K::Error)), BAD),
        }
    }

    fn sync_tray(&mut self) {
        let (text, _) = self.status_line();
        let state = match &self.status {
            Status::Streaming(_) => TrayState::Streaming,
            Status::Connecting(_) | Status::Reconnecting(..) | Status::Scanning | Status::Stopping => TrayState::Busy,
            _ => TrayState::Idle,
        };
        let stop_mode = self.streaming() || self.connecting();
        let can_toggle = stop_mode || (!self.busy() && self.selected.is_some());
        if let Some(tr) = self.tray.as_mut() {
            tr.update(state, &format!("airhome — {text}"), can_toggle, stop_mode);
            tr.set_volume(self.volume);
        }
    }

    // ------------------------------------------------------------ pages

    /// Top card: status, the chosen speaker and the big play/stop button.
    fn hero(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let (status_text, status_color) = self.status_line();
        let streaming = self.streaming();
        let busy = self.busy();
        let members = self.member_indices();
        let (name, sub) = match self.selected.and_then(|i| self.devices.get(i)) {
            Some(d) if members.is_empty() => (d.name.clone(), format!("{} · {}", d.model, d.ip)),
            Some(d) => {
                let mut names = vec![d.name.as_str()];
                names.extend(members.iter().filter_map(|&i| self.devices.get(i)).map(|x| x.name.as_str()));
                (tf(l, K::SpeakersCount, &(members.len() + 1).to_string()), names.join(" + "))
            }
            None => (t(l, K::NoSpeaker).to_string(), String::new()),
        };
        let connecting = self.connecting();
        let can_act = streaming || connecting || (!busy && self.selected.is_some());

        let outline = if streaming { GOOD.gamma_multiply(0.55) } else { border() };
        card_with(ui, 12, outline, |ui| {
            let btn_d = 42.0;
            let width = ui.available_width();
            let text_w = (width - btn_d - 16.0).max(60.0);

            // Wrapped status text decides the height.
            let status_g = ui.painter().layout(status_text, regular(13.0), status_color, text_w - 16.0);
            let line_h = 13.0 * 1.35;
            let h = (status_g.size().y + 4.0 + 24.0 + 2.0 + 14.0).max(btn_d + 8.0);
            let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());

            let top = rect.top() + (h - (status_g.size().y + 4.0 + 24.0 + 2.0 + 14.0)).max(0.0) * 0.5;
            let p = ui.painter().clone();
            p.circle_filled(pos2(rect.left() + 4.5, top + line_h * 0.5), 4.0, status_color);
            let status_h = status_g.size().y;
            p.galley(pos2(rect.left() + 16.0, top), status_g, status_color);

            let clip = Rect::from_min_size(rect.min, vec2(text_w, h));
            let pc = p.with_clip_rect(clip);
            pc.text(pos2(rect.left(), top + status_h + 4.0 + 12.0), Align2::LEFT_CENTER, name, bold(19.0), TEXT);
            pc.text(pos2(rect.left(), top + status_h + 4.0 + 24.0 + 2.0 + 7.0), Align2::LEFT_CENTER, sub, regular(12.0), TEXT2);

            let btn_rect = Rect::from_center_size(pos2(rect.right() - btn_d * 0.5, rect.center().y), vec2(btn_d, btn_d));
            let color = if streaming || connecting { BAD } else { GOOD };
            let kind = if connecting {
                ActionKind::Cancel
            } else if busy {
                ActionKind::Busy
            } else if streaming {
                ActionKind::Stop
            } else {
                ActionKind::Play
            };
            let clicked = ui
                .scope_builder(egui::UiBuilder::new().max_rect(btn_rect), |ui| round_action(ui, btn_d, kind, color, can_act))
                .inner;
            if clicked {
                self.toggle();
            }
        });
    }

    fn page_main(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let streaming = self.streaming();
        let busy = self.busy();

        self.hero(ui);
        ui.add_space(10.0);

        // ---- speakers ----
        section_row(ui, t(l, K::Speakers), |ui| {
            if refresh_button(ui, !streaming && !busy) {
                self.engine.send(Cmd::Scan);
            }
        });
        // Rows stay clickable while streaming: choosing another speaker restarts the stream on it.
        let enabled = !busy;
        card(ui, 4, |ui| {
            if self.devices.is_empty() {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
                let msg = if busy { t(l, K::Searching) } else { t(l, K::NoDevices) };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, regular(14.0), TEXT2);
            }
            let n = self.devices.len();
            for i in 0..n {
                let d = &self.devices[i];
                let sub = format!("{} · {}", d.model, d.ip);
                let is_main = self.selected == Some(i);
                let is_member = !is_main && self.settings.group.contains(&d.name);
                let dname = d.name.clone();
                match device_row(ui, &d.name, &sub, d.is_homepod, is_main, is_member, enabled) {
                    RowAction::Select => {
                        self.selected = Some(i);
                        self.settings.group.retain(|n| n != &dname); // the main speaker is not also a member
                        self.save_settings();
                        if streaming && !is_main {
                            self.restart_pending = true;
                            self.engine.send(Cmd::Stop);
                        }
                    }
                    RowAction::ToggleMember => {
                        if is_member {
                            self.settings.group.retain(|n| n != &dname);
                        } else {
                            self.settings.group.push(dname);
                        }
                        self.save_settings();
                        if streaming {
                            self.restart_pending = true; // the group changed: reconnect with the new members
                            self.engine.send(Cmd::Stop);
                        }
                    }
                    RowAction::None => {}
                }
                if i + 1 < n {
                    divider(ui);
                }
            }
        });
        if self.devices.len() > 1 && self.settings.group.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(t(l, K::GroupHint)).font(regular(12.0)).color(TEXT2));
        }
        ui.add_space(10.0);

        // ---- volume + PC level (one card) ----
        card(ui, 14, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(l, K::Volume)).font(regular(14.5)).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.0}%", self.volume * 100.0)).font(bold(14.5)).color(ACCENT));
                });
            });
            let st = slider(ui, "volume", &mut self.volume);
            if st.released {
                self.save_settings();
                self.push_volumes();
            }
            // Several speakers: each one also gets its own level (on top of the main volume above).
            let names = self.group_names();
            if names.len() > 1 {
                for name in names {
                    let mut trim = self.settings.speaker_trim.get(&name).copied().unwrap_or(1.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&name).font(regular(12.5)).color(TEXT2));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{:.0}%", trim * 100.0)).font(regular(12.5)).color(TEXT2));
                        });
                    });
                    let st = slider(ui, &format!("trim-{name}"), &mut trim);
                    if st.changed {
                        self.settings.speaker_trim.insert(name.clone(), trim);
                    }
                    if st.released {
                        self.save_settings();
                        self.push_volumes();
                    }
                }
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(l, K::PcLevel)).font(regular(12.5)).color(TEXT2));
            });
            level_bar(ui, self.level);
            if !self.source.is_empty() {
                ui.label(RichText::new(format!("{}: {}", t(l, K::Source), self.source)).font(regular(11.5)).color(TEXT2));
            }
        });
    }

    fn page_settings(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let ctx = ui.ctx().clone();

        // ---- general ----
        card(ui, 4, |ui| {
            // language: label on the left, compact dropdown on the right
            egui::Frame::new().inner_margin(Margin::symmetric(14, 5)).show(ui, |ui| {
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(RichText::new(t(l, K::Language)).font(regular(14.5)).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.allocate_ui_with_layout(vec2(150.0, 36.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            let names: Vec<&str> = Lang::ALL.iter().map(|c| c.native_name()).collect();
                            let cur = Lang::ALL.iter().position(|c| *c == self.lang).unwrap_or(0);
                            if let Some(i) = dropdown(ui, "lang", &names, cur) {
                                self.set_language(&ctx, Lang::ALL[i]);
                            }
                        });
                    });
                });
            });
            divider(ui);
            if toggle_row(ui, t(l, K::AutoStart), None, &mut self.autostart) {
                if let Err(e) = autostart::set(self.autostart) {
                    tracing::error!("autostart change failed: {e}");
                    self.autostart = autostart::is_enabled();
                }
            }
            divider(ui);
            // Say *which* speaker this refers to, so it can never be a surprise.
            let saved = self.settings.device.clone();
            if toggle_row(ui, t(l, K::AutoConnect), saved.as_deref(), &mut self.settings.auto_connect) {
                self.save_settings();
            }
            divider(ui);
            if toggle_row(ui, t(l, K::MutePc), None, &mut self.settings.mute_pc) {
                self.save_settings();
                if self.streaming() {
                    self.engine.send(Cmd::SetMutePc(self.settings.mute_pc)); // takes effect immediately
                }
            }
            divider(ui);
            if toggle_row(ui, t(l, K::CloseToTray), None, &mut self.settings.close_to_tray) {
                self.save_settings();
            }
        });
        ui.add_space(10.0);

        // ---- speakers added by IP ----
        card(ui, 4, |ui| {
            let title = if self.settings.manual_ips.is_empty() {
                t(l, K::ManualTitle).to_string()
            } else {
                format!("{} ({})", t(l, K::ManualTitle), self.settings.manual_ips.len())
            };
            if nav_row(ui, &title) {
                self.manual_open = !self.manual_open;
            }
            if !self.manual_open {
                return;
            }
            divider(ui);
            egui::Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new(t(l, K::ManualHint)).font(regular(12.5)).color(TEXT2));
            ui.add_space(8.0);
            let mut submit = false;
            ui.horizontal(|ui| {
                let port_w = 84.0;
                let gap = ui.spacing().item_spacing.x;
                let hint = |s: &str| RichText::new(s.to_owned()).color(TEXT2.gamma_multiply(0.55));
                let ip_edit = egui::TextEdit::singleline(&mut self.manual_input)
                    .hint_text(hint(t(l, K::ManualPlaceholder)))
                    .font(regular(14.5))
                    .margin(Margin::symmetric(12, 10))
                    .desired_width(ui.available_width() - port_w - gap - 24.0);
                let r1 = ui.add(ip_edit);
                let port_edit = egui::TextEdit::singleline(&mut self.manual_port)
                    .hint_text(hint("7000"))
                    .font(regular(14.5))
                    .margin(Margin::symmetric(12, 10))
                    .char_limit(5)
                    .desired_width(port_w - 24.0);
                let r2 = ui.add(port_edit);
                // keep digits only
                self.manual_port.retain(|c| c.is_ascii_digit());
                if (r1.lost_focus() || r2.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });
            ui.add_space(8.0);
            if pill_button(ui, t(l, K::ManualAdd)) {
                submit = true;
            }
            if submit {
                // Accept "ip" or "ip:port" in the first field, or a separate port in the second.
                let text = self.manual_input.trim().to_string();
                let port: Option<u16> = if self.manual_port.is_empty() {
                    Some(7000)
                } else {
                    self.manual_port.parse().ok().filter(|p| *p != 0)
                };
                let parsed = engine::parse_addr(&text).filter(|_| !text.is_empty()).map(|mut a| {
                    // the port field applies only when the IP field has no explicit port
                    if text.parse::<std::net::SocketAddr>().is_err() {
                        if let Some(p) = port {
                            a.set_port(p);
                        }
                    }
                    a
                });
                match (parsed, port) {
                    (Some(addr), Some(_)) => {
                        let exists = self.settings.manual_ips.iter().any(|e| engine::parse_addr(e) == Some(addr));
                        if !exists {
                            self.settings.manual_ips.push(addr.to_string());
                            self.save_settings();
                            self.engine.send(Cmd::SetManual(self.settings.manual_ips.clone()));
                            self.engine.send(Cmd::Scan);
                        }
                        self.manual_input.clear();
                        self.manual_port.clear();
                        self.manual_invalid = false;
                    }
                    _ => self.manual_invalid = true,
                }
            }
            if self.manual_invalid {
                ui.add_space(6.0);
                ui.label(RichText::new(t(l, K::ManualInvalid)).font(regular(12.5)).color(BAD));
            }
            let mut remove = None;
            for (i, ip) in self.settings.manual_ips.iter().enumerate() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ip).font(regular(14.0)).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Painted cross: no dependence on font glyphs.
                        let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                        let col = if resp.hovered() { BAD } else { TEXT2 };
                        let c = r.center();
                        let st = egui::Stroke::new(1.8, col);
                        let p = ui.painter();
                        p.line_segment([pos2(c.x - 4.5, c.y - 4.5), pos2(c.x + 4.5, c.y + 4.5)], st);
                        p.line_segment([pos2(c.x - 4.5, c.y + 4.5), pos2(c.x + 4.5, c.y - 4.5)], st);
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                self.settings.manual_ips.remove(i);
                self.save_settings();
                self.engine.send(Cmd::SetManual(self.settings.manual_ips.clone()));
                self.engine.send(Cmd::Scan);
            }
            });
        });
        ui.add_space(10.0);

        // ---- latency ----
        card(ui, 12, |ui| {
            const MIN_MS: f32 = 100.0;
            const MAX_MS: f32 = 2000.0;
            let ms = self.settings.latency_ms as f32;
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(l, K::LatBuffer)).font(regular(14.5)).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} ms", self.settings.latency_ms)).font(bold(14.5)).color(ACCENT));
                });
            });
            let mut v = (ms - MIN_MS) / (MAX_MS - MIN_MS);
            let st = slider(ui, "latency_ms", &mut v);
            if st.changed {
                // 50 ms steps keep the saved value tidy
                let raw = MIN_MS + v * (MAX_MS - MIN_MS);
                self.settings.latency_ms = ((raw / 50.0).round() * 50.0).clamp(MIN_MS, MAX_MS) as u32;
            }
            if st.released {
                self.save_settings();
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(l, K::LatLow)).font(regular(12.0)).color(TEXT2));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(t(l, K::LatStable)).font(regular(12.0)).color(TEXT2));
                });
            });
            if self.streaming() {
                ui.label(RichText::new(t(l, K::LatHint)).font(regular(12.0)).color(TEXT2.gamma_multiply(0.8)));
            }
        });
        ui.add_space(10.0);

        // ---- files + about (one card) ----
        card(ui, 4, |ui| {
            if nav_row(ui, t(l, K::OpenLogs)) {
                open_folder(settings::log_dir());
            }
            divider(ui);
            if nav_row(ui, t(l, K::OpenSettings)) {
                open_folder(settings::dir());
            }
            divider(ui);
            egui::Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.label(
                    RichText::new(format!("airhome v{} · {}", env!("CARGO_PKG_VERSION"), t(l, K::License)))
                        .font(regular(12.0))
                        .color(TEXT2),
                );
            });
        });
        ui.add_space(8.0);
    }
}

impl eframe::App for App {
    /// Runs even while the window is hidden, so the tray keeps working.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();

        let cmds = self.tray.as_ref().map(|tr| tr.poll()).unwrap_or_default();
        for cmd in cmds {
            match cmd {
                TrayCmd::Show => self.show_window(ctx),
                TrayCmd::Toggle => self.toggle(),
                TrayCmd::VolumeStep(d) => self.set_volume_from_tray(self.volume + d),
                TrayCmd::VolumeSet(v) => self.set_volume_from_tray(v),
                TrayCmd::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        // Another launch of the program asked us to show our window.
        if self.show_rx.try_iter().count() > 0 {
            self.show_window(ctx);
        }

        // Retry discovery while waiting to auto-connect (e.g. Wi-Fi not up yet right after login).
        if let Some(when) = self.next_scan {
            if Instant::now() >= when {
                self.next_scan = None;
                if !self.busy() && !self.streaming() {
                    self.engine.send(Cmd::Scan);
                }
            } else {
                ctx.request_repaint_after(when - Instant::now());
            }
        }

        // The window's X button hides it to the tray (or really quits, if the user prefers).
        if !self.quitting && self.settings.close_to_tray && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_hidden = true;
        }

        self.sync_tray();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Level meter: fast attack, slow release.
        let peak = (capture::PEAK.swap(0, Ordering::Relaxed) as f32 / 32767.0).clamp(0.0, 1.0);
        self.level = if peak > self.level { peak } else { self.level * 0.85 };

        // Demo mode (developer switch): pretend to be in a given state.
        if let Some(demo) = self.demo {
            let name = self.devices.first().map(|d| d.name.clone()).unwrap_or_else(|| "HomePod".into());
            self.status = match demo {
                "streaming" => Status::Streaming(name),
                "connecting" => Status::Connecting(name),
                "lost" => Status::ConnectionLost,
                _ => Status::Error("Timeout".into()),
            };
            self.level = 0.55;
        }
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(16)); // smooth spinner
        } else if self.streaming() {
            ui.ctx().request_repaint_after(Duration::from_millis(50)); // level meter
        }

        let l = self.lang;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin { left: 20, right: 20, top: 18, bottom: 0 }))
            .show(ui, |ui| {
                // ---- header ----
                ui.horizontal(|ui| {
                    logo(ui, 30.0);
                    ui.label(RichText::new("airhome").font(bold(21.0)).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).font(regular(12.0)).color(TEXT2));
                    });
                });
                ui.add_space(6.0);

                // ---- tabs ----
                let mut tab = if self.page == Page::Main { 0 } else { 1 };
                if segmented(ui, "tabs", &[t(l, K::TabMain), t(l, K::TabSettings)], &mut tab) {
                    self.page = if tab == 0 { Page::Main } else { Page::Settings };
                }
                ui.add_space(10.0);

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.page {
                        Page::Main => self.page_main(ui),
                        Page::Settings => self.page_settings(ui),
                    }
                    ui.add_space(20.0);
                });
            });
    }
}
