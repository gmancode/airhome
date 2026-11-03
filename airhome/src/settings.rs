//! Persistent settings (JSON in %APPDATA%\airhome\settings.json).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Name of the last used speaker (names are stable; IPs can change).
    pub device: Option<String>,
    /// HomePod volume, 0.0..=1.0.
    pub volume: f32,
    /// Start streaming to the saved speaker as soon as the app launches.
    pub auto_connect: bool,
    /// UI language code ("en", "ru", ...). `None` = detect from Windows on first run.
    pub language: Option<String>,
    /// The window's close button hides to the tray (true) or quits (false).
    pub close_to_tray: bool,
    /// Pre-fill of the jitter buffer in milliseconds (100..=2000): lower = less delay, higher = steadier.
    pub latency_ms: u32,
    /// Mute this PC's own speakers while streaming, so sound comes only from the HomePod.
    pub mute_pc: bool,
    /// Speakers added by IP address (for networks without working mDNS).
    pub manual_ips: Vec<String>,
    /// Extra speakers (by name) that play together with `device`.
    pub group: Vec<String>,
    /// Per-speaker volume factor (0..=1, default 1) on top of the main volume, by speaker name.
    pub speaker_trim: std::collections::HashMap<String, f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device: None,
            volume: 0.4,
            auto_connect: false,
            language: None,
            close_to_tray: true,
            latency_ms: 200,
            mute_pc: true,
            manual_ips: Vec::new(),
            group: Vec::new(),
            speaker_trim: std::collections::HashMap::new(),
        }
    }
}

impl Settings {
    /// Pre-fill as a percent of the 2000 ms jitter buffer.
    pub fn prefill_percent(&self) -> u32 {
        (self.latency_ms / 20).clamp(5, 100)
    }
}

pub fn dir() -> PathBuf {
    path().parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

pub fn log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("airhome")
}

fn path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("airhome")
        .join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = std::fs::read_to_string(path())
            .ok()
            // Windows editors (Notepad, PowerShell) like to prepend a BOM; don't lose the settings over it.
            .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        s.volume = s.volume.clamp(0.0, 1.0);
        s.latency_ms = s.latency_ms.clamp(100, 2000);
        s
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&p, text) {
                    tracing::error!("cannot save settings: {e}");
                }
            }
            Err(e) => tracing::error!("cannot serialize settings: {e}"),
        }
    }
}
