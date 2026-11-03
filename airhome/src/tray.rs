//! System tray icon + menu (including speaker volume, so the window never has to be opened for it).

use std::sync::mpsc::{channel, Receiver};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayCmd {
    Show,
    Toggle,
    Quit,
    /// Change the speaker volume by this much (fraction of full scale, may be negative).
    VolumeStep(f32),
    /// Set the speaker volume to this value (0.0..=1.0).
    VolumeSet(f32),
}

#[derive(Clone, Copy, PartialEq)]
pub enum TrayState {
    Idle,
    Busy,
    Streaming,
}

/// All user-visible menu texts (so a language change can refresh them in one go).
pub struct TrayLabels<'a> {
    pub show: &'a str,
    pub start: &'a str,
    pub stop: &'a str,
    pub quit: &'a str,
    pub volume: &'a str,
    pub louder: &'a str,
    pub quieter: &'a str,
}

/// Preset volumes in the submenu, in percent.
const PRESETS: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// Step of the "louder / quieter" items.
pub const STEP: f32 = 0.05;

pub struct Tray {
    icon: TrayIcon,
    show_item: MenuItem,
    toggle_item: MenuItem,
    quit_item: MenuItem,
    volume_menu: Submenu,
    louder_item: MenuItem,
    quieter_item: MenuItem,
    presets: Vec<CheckMenuItem>,
    rx: Receiver<TrayCmd>,
    state: TrayState,
    start_label: String,
    stop_label: String,
    volume_label: String,
    /// Text currently shown on the toggle item (to avoid redundant updates).
    toggle_shown: String,
    /// Volume currently reflected in the menu, in whole percent (None = not shown yet).
    volume_shown: Option<u32>,
}

/// Ring colours (start, middle, end of the gradient) for each state.
fn palette(state: TrayState) -> [(f32, f32, f32); 3] {
    match state {
        // the brand gradient: cyan -> blue -> violet
        TrayState::Idle => [(90.0, 200.0, 250.0), (10.0, 132.0, 255.0), (191.0, 90.0, 242.0)],
        TrayState::Busy => [(255.0, 214.0, 10.0), (255.0, 176.0, 20.0), (255.0, 138.0, 20.0)],
        TrayState::Streaming => [(80.0, 230.0, 170.0), (52.0, 211.0, 110.0), (30.0, 170.0, 90.0)],
    }
}

fn lerp(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// Draw the app logo (dark rounded tile, thin outer ring, gradient ring and dot) into an RGBA buffer
/// (straight alpha), `n` x `n` pixels. The ring colours follow `state`. Rendered with 3x3 supersampling.
pub fn render_logo(n: u32, state: TrayState) -> (Vec<u8>, u32, u32) {
    let pal = palette(state);
    let ss = 4u32;
    // rim at least ~1 physical pixel wide, so small sizes stay clean instead of shimmering
    let rim = (1.1 / n as f32).max(0.016);
    let mut px = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let (mut ar, mut ag, mut ab, mut aa) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for sy in 0..ss {
                for sx in 0..ss {
                    let u = (x as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32;
                    // rounded square (half size 0.47, corner radius 0.23)
                    let (qx, qy) = ((u - 0.5).abs() - 0.24, (v - 0.5).abs() - 0.24);
                    let sd = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - 0.23;
                    if sd > 0.0 {
                        continue;
                    }
                    // background: dark, slightly lighter at the top
                    let mut c = lerp((27.0, 32.0, 51.0), (9.0, 11.0, 18.0), v);
                    let d = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
                    let ring_col = {
                        let t = ((u + v) * 0.5).clamp(0.0, 1.0);
                        if t < 0.55 {
                            lerp(pal[0], pal[1], t / 0.55)
                        } else {
                            lerp(pal[1], pal[2], (t - 0.55) / 0.45)
                        }
                    };
                    if (d - 0.36).abs() < 0.015 {
                        c = lerp(c, pal[1], 0.28); // faint outer halo
                    }
                    if sd > -rim {
                        c = lerp(c, (255.0, 255.0, 255.0), 0.18); // thin light rim so the tile reads on dark backgrounds
                    }
                    if (d - 0.27).abs() < 0.045 || d < 0.09 {
                        c = ring_col;
                    }
                    ar += c.0;
                    ag += c.1;
                    ab += c.2;
                    aa += 1.0;
                }
            }
            let i = ((y * n + x) * 4) as usize;
            if aa > 0.0 {
                px[i] = (ar / aa) as u8;
                px[i + 1] = (ag / aa) as u8;
                px[i + 2] = (ab / aa) as u8;
                px[i + 3] = (aa / (ss * ss) as f32 * 255.0) as u8;
            }
        }
    }
    (px, n, n)
}

/// Tray/window icon pixels.
pub fn make_rgba(state: TrayState) -> (Vec<u8>, u32, u32) {
    render_logo(64, state)
}

fn icon(state: TrayState) -> Icon {
    let (rgba, w, h) = make_rgba(state);
    Icon::from_rgba(rgba, w, h).expect("valid icon")
}

impl Tray {
    /// Must be created on the thread that runs the window event loop (the main thread).
    pub fn new(ctx: egui::Context, labels: &TrayLabels, volume: f32) -> anyhow::Result<Self> {
        let menu = Menu::new();
        let show_item = MenuItem::new(labels.show, true, None);
        let toggle_item = MenuItem::new(labels.start, true, None);
        let quit_item = MenuItem::new(labels.quit, true, None);

        let volume_menu = Submenu::new(labels.volume, true);
        let louder_item = MenuItem::new(labels.louder, true, None);
        let quieter_item = MenuItem::new(labels.quieter, true, None);
        let presets: Vec<CheckMenuItem> =
            PRESETS.iter().map(|p| CheckMenuItem::new(format!("{p}%"), true, false, None)).collect();
        volume_menu.append(&louder_item)?;
        volume_menu.append(&quieter_item)?;
        volume_menu.append(&PredefinedMenuItem::separator())?;
        for p in &presets {
            volume_menu.append(p)?;
        }

        menu.append_items(&[
            &show_item,
            &toggle_item,
            &PredefinedMenuItem::separator(),
            &volume_menu,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])?;

        let (tx, rx) = channel();

        let ids: (MenuId, MenuId, MenuId, MenuId, MenuId) = (
            show_item.id().clone(),
            toggle_item.id().clone(),
            quit_item.id().clone(),
            louder_item.id().clone(),
            quieter_item.id().clone(),
        );
        let preset_ids: Vec<(MenuId, f32)> =
            presets.iter().zip(PRESETS).map(|(item, p)| (item.id().clone(), p as f32 / 100.0)).collect();
        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let cmd = if e.id == ids.0 {
                    TrayCmd::Show
                } else if e.id == ids.1 {
                    TrayCmd::Toggle
                } else if e.id == ids.2 {
                    TrayCmd::Quit
                } else if e.id == ids.3 {
                    TrayCmd::VolumeStep(STEP)
                } else if e.id == ids.4 {
                    TrayCmd::VolumeStep(-STEP)
                } else if let Some((_, v)) = preset_ids.iter().find(|(id, _)| *id == e.id) {
                    TrayCmd::VolumeSet(*v)
                } else {
                    return;
                };
                let _ = tx.send(cmd);
                ctx.request_repaint();
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                let show = matches!(
                    e,
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                        | TrayIconEvent::DoubleClick { .. }
                );
                if show {
                    let _ = tx.send(TrayCmd::Show);
                    ctx.request_repaint();
                }
            }));
        }

        let icon_handle = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("airhome")
            .with_icon(icon(TrayState::Idle))
            .build()?;

        let mut tray = Self {
            icon: icon_handle,
            show_item,
            toggle_item,
            quit_item,
            volume_menu,
            louder_item,
            quieter_item,
            presets,
            rx,
            state: TrayState::Idle,
            start_label: labels.start.to_string(),
            stop_label: labels.stop.to_string(),
            volume_label: labels.volume.to_string(),
            toggle_shown: labels.start.to_string(),
            volume_shown: None,
        };
        tray.set_volume(volume);
        Ok(tray)
    }

    pub fn poll(&self) -> Vec<TrayCmd> {
        self.rx.try_iter().collect()
    }

    /// Change the language of the menu.
    pub fn set_labels(&mut self, labels: &TrayLabels) {
        self.show_item.set_text(labels.show);
        self.quit_item.set_text(labels.quit);
        self.louder_item.set_text(labels.louder);
        self.quieter_item.set_text(labels.quieter);
        self.start_label = labels.start.to_string();
        self.stop_label = labels.stop.to_string();
        self.volume_label = labels.volume.to_string();
        self.toggle_shown.clear(); // force refresh in update()
        self.volume_shown = None; // and the volume title
    }

    /// Show the current volume in the menu title ("Volume: 40%") and tick the matching preset.
    pub fn set_volume(&mut self, volume: f32) {
        let pct = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
        if self.volume_shown == Some(pct) {
            return;
        }
        self.volume_shown = Some(pct);
        self.volume_menu.set_text(format!("{}: {pct}%", self.volume_label));
        for (item, p) in self.presets.iter().zip(PRESETS) {
            item.set_checked(p == pct);
        }
    }

    /// `stop_mode`: the toggle item reads "Stop" (streaming, or a connection attempt that can be cancelled).
    pub fn update(&mut self, state: TrayState, tooltip: &str, can_toggle: bool, stop_mode: bool) {
        if state != self.state {
            self.state = state;
            let _ = self.icon.set_icon(Some(icon(state)));
        }
        let want = if stop_mode { &self.stop_label } else { &self.start_label };
        if *want != self.toggle_shown {
            self.toggle_item.set_text(want);
            self.toggle_shown = want.clone();
        }
        self.toggle_item.set_enabled(can_toggle);
        let _ = self.icon.set_tooltip(Some(tooltip));
    }
}
