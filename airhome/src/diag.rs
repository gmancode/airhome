//! "Check connection": quick, read-only checks for the usual reasons why a speaker stays silent
//! (found in the issue trackers of similar apps): Public network profile, unreachable speaker,
//! busy time-sync ports, VPN/virtual adapters, an ad blocker with a network filter, no sound device.
//! Nothing is changed on the PC and nothing is sent anywhere.

use std::io::Read;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Check {
    Network,
    Speaker,
    Ports,
    Adapters,
    Security,
    Audio,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub check: Check,
    pub level: Level,
    /// Extra text: the speaker address, adapter names, the sound device, …
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub items: Vec<Item>,
    pub os: String,
}

impl Report {
    pub fn all_ok(&self) -> bool {
        self.items.iter().all(|i| i.level == Level::Ok)
    }

    /// Plain English text to paste into a GitHub issue.
    pub fn text(&self) -> String {
        let mut s = format!("airhome v{} · {}\n", env!("CARGO_PKG_VERSION"), self.os);
        for it in &self.items {
            let label = match it.check {
                Check::Network => "Network type",
                Check::Speaker => "Speaker",
                Check::Ports => "Time-sync ports 319/320",
                Check::Adapters => "VPN and virtual adapters",
                Check::Security => "Ad blocker / filter",
                Check::Audio => "Sound output",
            };
            let mark = match it.level {
                Level::Ok => "OK  ",
                Level::Warn => "WARN",
                Level::Fail => "FAIL",
            };
            s.push_str(&format!("[{mark}] {label}: {}\n", it.detail));
        }
        s
    }
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Asks Windows (through PowerShell, hidden) about the network profile, adapters and running programs.
const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
'OS=' + [System.Environment]::OSVersion.VersionString
$ip = $env:AIRHOME_IP
$alias = $null
if ($ip) {
  $r = Find-NetRoute -RemoteIPAddress $ip | Where-Object { $_.InterfaceAlias } | Select-Object -First 1
  if ($r) { $alias = $r.InterfaceAlias }
}
if ($alias) { $cp = Get-NetConnectionProfile -InterfaceAlias $alias } else { $cp = Get-NetConnectionProfile }
foreach ($c in $cp) {
  $d = (Get-NetAdapter -InterfaceAlias $c.InterfaceAlias | Select-Object -First 1).InterfaceDescription
  'PROFILE=' + $c.NetworkCategory + '|' + $d
}
Get-NetAdapter | Where-Object { $_.Status -eq 'Up' } | ForEach-Object { 'ADAPTER=' + $_.InterfaceDescription }
Get-Process | ForEach-Object { 'PROC=' + $_.ProcessName }
"#;

#[derive(Default)]
struct Shell {
    ok: bool,
    os: String,
    profiles: Vec<String>,
    adapters: Vec<String>,
    procs: Vec<String>,
}

fn query_shell(ip: Option<SocketAddr>) -> Shell {
    let mut sh = Shell::default();
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(a) = ip {
        cmd.env("AIRHOME_IP", a.ip().to_string());
    }
    let Ok(mut child) = cmd.spawn() else { return sh };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                return sh;
            }
        }
    }
    let mut raw = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut raw);
    }
    let text = String::from_utf8_lossy(&raw);
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(v) = line.strip_prefix("OS=") {
            sh.os = v.to_string();
            sh.ok = true;
        } else if let Some(v) = line.strip_prefix("PROFILE=") {
            sh.profiles.push(v.to_string());
        } else if let Some(v) = line.strip_prefix("ADAPTER=") {
            sh.adapters.push(v.to_string());
        } else if let Some(v) = line.strip_prefix("PROC=") {
            sh.procs.push(v.to_string());
        }
    }
    sh
}

/// VPN tunnels and virtual machine / container adapters.
fn is_virtual(description: &str) -> bool {
    const WORDS: [&str; 15] = [
        "vpn", "tap-windows", "tap windows", "wireguard", "wintun", "tunnel", "openvpn", "nordlynx",
        "tailscale", "zerotier", "hyper-v", "vethernet", "vmware", "virtualbox", "docker",
    ];
    let low = description.to_lowercase();
    WORDS.iter().any(|w| low.contains(w))
}

/// Adapters that are known to confuse local-network discovery.
fn virtual_adapters(adapters: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for a in adapters {
        if is_virtual(a) && !found.contains(a) {
            found.push(a.clone());
        }
    }
    found.truncate(3);
    found
}

fn ports_busy() -> Vec<u16> {
    [319u16, 320]
        .into_iter()
        .filter(|p| UdpSocket::bind(("0.0.0.0", *p)).is_err())
        .collect()
}

/// `speaker`: the chosen speaker's address, if any. `streaming`: airhome itself holds the time-sync ports then.
pub fn run(speaker: Option<SocketAddr>, streaming: bool) -> Report {
    let sh = query_shell(speaker);
    let mut items = Vec::new();

    // "category|adapter description". With a known speaker only the adapter that reaches it is listed;
    // otherwise ignore VPN tunnels (they are usually "Public" and unrelated to the speaker).
    let profiles: Vec<(&str, &str)> = sh
        .profiles
        .iter()
        .map(|p| p.split_once('|').unwrap_or((p.as_str(), "")))
        .filter(|(_, desc)| speaker.is_some() || !is_virtual(desc))
        .collect();
    if let Some((first, _)) = profiles.first() {
        let public = profiles.iter().any(|(c, _)| c.eq_ignore_ascii_case("Public"));
        items.push(Item {
            check: Check::Network,
            level: if public { Level::Warn } else { Level::Ok },
            detail: first.to_string(),
        });
    }

    items.push(match speaker {
        None => Item { check: Check::Speaker, level: Level::Warn, detail: String::new() },
        Some(addr) => {
            let reachable = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok();
            Item {
                check: Check::Speaker,
                level: if reachable { Level::Ok } else { Level::Fail },
                detail: addr.ip().to_string(),
            }
        }
    });

    // While streaming, airhome holds the ports itself, which is normal.
    let busy = if streaming { Vec::new() } else { ports_busy() };
    items.push(Item {
        check: Check::Ports,
        level: if busy.is_empty() { Level::Ok } else { Level::Fail },
        detail: busy.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
    });

    if sh.ok {
        let virt = virtual_adapters(&sh.adapters);
        items.push(Item {
            check: Check::Adapters,
            level: if virt.is_empty() { Level::Ok } else { Level::Warn },
            detail: virt.join(", "),
        });
        let adguard = sh.procs.iter().any(|p| p.to_lowercase().contains("adguard"));
        items.push(Item {
            check: Check::Security,
            level: if adguard { Level::Warn } else { Level::Ok },
            detail: if adguard { "AdGuard".to_string() } else { String::new() },
        });
    }

    let device = cpal::default_host().default_output_device().and_then(|d| d.name().ok());
    items.push(match device {
        Some(name) => Item { check: Check::Audio, level: Level::Ok, detail: name },
        None => Item { check: Check::Audio, level: Level::Fail, detail: String::new() },
    });

    Report { items, os: if sh.os.is_empty() { "Windows".to_string() } else { sh.os } }
}
