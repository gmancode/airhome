//! Small design system: colours, typography and hand-drawn widgets.
//!
//! All icons are drawn with vector shapes (no Unicode symbols), so they can never show up as "tofu"
//! boxes whatever fonts the machine has. Spacing follows a 4 px grid.

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

// ------------------------------------------------------------------ palette (dark)

pub const BG: Color32 = Color32::from_rgb(15, 16, 20);
pub const CARD: Color32 = Color32::from_rgb(25, 27, 33);
pub const TEXT: Color32 = Color32::from_rgb(240, 242, 246);
pub const TEXT2: Color32 = Color32::from_rgb(142, 149, 162);
pub const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
pub const GOOD: Color32 = Color32::from_rgb(48, 209, 88);
pub const WARN: Color32 = Color32::from_rgb(255, 159, 10);
pub const BAD: Color32 = Color32::from_rgb(255, 69, 58);

pub fn border() -> Color32 {
    Color32::from_white_alpha(18)
}
fn divider_color() -> Color32 {
    Color32::from_white_alpha(14)
}
fn hover_fill() -> Color32 {
    Color32::from_white_alpha(10)
}

// ------------------------------------------------------------------ typography

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn pointer(ui: &Ui, hovered: bool) {
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

// ------------------------------------------------------------------ containers

/// A rounded surface. `pad` is the inner margin (use 4 for lists of rows, 16 for free content).
pub fn card<R>(ui: &mut Ui, pad: i8, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_with(ui, pad, border(), add)
}

/// Like [`card`] with a custom outline colour (used to tint the hero card by state).
pub fn card_with<R>(ui: &mut Ui, pad: i8, outline: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, outline))
        .corner_radius(16)
        .inner_margin(Margin::same(pad))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = if pad <= 6 { 0.0 } else { 6.0 };
            add(ui)
        })
        .inner
}

/// Small grey caption above a card.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(text).font(bold(12.5)).color(TEXT2));
    });
    ui.add_space(1.0);
}

/// Hairline between rows inside a card.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(rect.left() + 14.0, rect.center().y), pos2(rect.right() - 14.0, rect.center().y)],
        Stroke::new(1.0, divider_color()),
    );
}

// ------------------------------------------------------------------ vector icons

pub fn icon_play(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    let h = size * 0.5;
    // Optical centring: a triangle looks left-heavy, so nudge it right a little.
    let c = pos2(c.x + size * 0.06, c.y);
    p.add(Shape::convex_polygon(
        vec![pos2(c.x - h * 0.8, c.y - h), pos2(c.x - h * 0.8, c.y + h), pos2(c.x + h, c.y)],
        color,
        Stroke::NONE,
    ));
}

pub fn icon_stop(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    p.rect_filled(Rect::from_center_size(c, vec2(size, size)), CornerRadius::same((size * 0.2) as u8), color);
}

pub fn icon_check(p: &egui::Painter, c: Pos2, color: Color32) {
    p.add(Shape::line(
        vec![pos2(c.x - 5.0, c.y + 0.5), pos2(c.x - 1.5, c.y + 4.0), pos2(c.x + 5.5, c.y - 4.0)],
        Stroke::new(2.2, color),
    ));
}

pub fn icon_chevron(p: &egui::Painter, c: Pos2, color: Color32) {
    p.add(Shape::line(
        vec![pos2(c.x - 2.5, c.y - 5.0), pos2(c.x + 2.5, c.y), pos2(c.x - 2.5, c.y + 5.0)],
        Stroke::new(1.8, color),
    ));
}

/// Circular "refresh" arrow: a ~315° arc with a solid arrow head at its clockwise end.
pub fn icon_refresh(p: &egui::Painter, c: Pos2, color: Color32) {
    use std::f32::consts::PI;
    let r = 7.0;
    // Screen coordinates: angles grow clockwise. Start at the upper right, end at the top.
    let (a0, a1) = (-0.30 * PI, 1.50 * PI);
    let n = 28;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    let end = *pts.last().unwrap();
    p.add(Shape::line(pts, Stroke::new(1.9, color)));
    // Filled head pointing along the direction of travel.
    let tangent = vec2(-a1.sin(), a1.cos());
    let normal = vec2(a1.cos(), a1.sin());
    let tip = end + tangent * 5.5;
    let b1 = end + normal * 4.2 - tangent * 0.5;
    let b2 = end - normal * 4.2 - tangent * 0.5;
    p.add(Shape::convex_polygon(vec![tip, b1, b2], color, Stroke::NONE));
}

/// Indeterminate spinner: a 270° arc that rotates with time.
pub fn icon_spinner(p: &egui::Painter, c: Pos2, radius: f32, time: f64, color: Color32) {
    use std::f32::consts::PI;
    let start = (time * 5.0) as f32;
    let n = 26;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = start + 1.5 * PI * i as f32 / n as f32;
            pos2(c.x + radius * a.cos(), c.y + radius * a.sin())
        })
        .collect();
    p.add(Shape::line(pts, Stroke::new(3.0, color)));
}

/// Speaker / display tile used in the device list.
fn device_tile(p: &egui::Painter, rect: Rect, is_homepod: bool, enabled: bool) {
    let a = if enabled { 1.0 } else { 0.5 };
    let (fill, glyph) = if is_homepod {
        // same look as the app logo: dark tile, light rim, glowing ring
        (Color32::from_rgb(24, 29, 46).gamma_multiply(a), Color32::from_rgb(64, 170, 255).gamma_multiply(a))
    } else {
        (Color32::from_gray(58).gamma_multiply(a), Color32::from_gray(230).gamma_multiply(a))
    };
    p.rect_filled(rect, CornerRadius::same(11), fill);
    if is_homepod {
        p.rect_stroke(rect, CornerRadius::same(11), Stroke::new(1.0, Color32::from_white_alpha((46.0 * a) as u8)), StrokeKind::Inside);
    }
    let c = rect.center();
    if is_homepod {
        p.circle_stroke(c, 8.5, Stroke::new(2.2, glyph));
        p.circle_filled(c, 3.2, glyph);
    } else {
        // a tiny display on a stand
        let screen = Rect::from_center_size(pos2(c.x, c.y - 2.0), vec2(18.0, 12.0));
        p.rect_stroke(screen, CornerRadius::same(2), Stroke::new(1.8, glyph), StrokeKind::Inside);
        p.line_segment([pos2(c.x - 4.5, c.y + 8.0), pos2(c.x + 4.5, c.y + 8.0)], Stroke::new(1.8, glyph));
    }
}

/// App mark, drawn from the same renderer as the tray and window icons so they always match.
/// It is rendered at exactly the on-screen pixel size and snapped to the pixel grid, so edges stay crisp.
pub fn logo(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let ppp = ui.ctx().pixels_per_point();
    let n = ((size * ppp).round() as u32).max(16);
    let id = egui::Id::new(("airhome-logo-texture", n));
    let tex: egui::TextureHandle = match ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        Some(t) => t,
        None => {
            let (rgba, w, h) = crate::tray::render_logo(n, crate::tray::TrayState::Idle);
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            let t = ui.ctx().load_texture(format!("airhome-logo-{n}"), img, egui::TextureOptions::NEAREST);
            ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    };
    // snap to whole physical pixels: one texel = one screen pixel
    let min = pos2((rect.min.x * ppp).round() / ppp, (rect.min.y * ppp).round() / ppp);
    let snapped = Rect::from_min_size(min, vec2(n as f32 / ppp, n as f32 / ppp));
    ui.painter().image(tex.id(), snapped, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

// ------------------------------------------------------------------ buttons

#[derive(Clone, Copy, PartialEq)]
pub enum ActionKind {
    Play,
    Stop,
    /// Something is in progress: shows a spinner and ignores clicks.
    Busy,
    /// A connection attempt that can still be cancelled: spinner ring around a stop square, clickable.
    Cancel,
}

/// Round primary button with a play / stop glyph or a spinner.
pub fn round_action(ui: &mut Ui, diameter: f32, kind: ActionKind, color: Color32, enabled: bool) -> bool {
    let clickable = enabled && kind != ActionKind::Busy;
    let (rect, resp) = ui.allocate_exact_size(vec2(diameter, diameter), if clickable { Sense::click() } else { Sense::hover() });
    let hovered = clickable && resp.hovered();
    pointer(ui, hovered);
    let t = ui.ctx().animate_bool_with_time(resp.id, hovered, 0.12);
    let base = if enabled || kind == ActionKind::Busy { color } else { Color32::from_gray(60) };
    let base = if kind == ActionKind::Busy { Color32::from_gray(62) } else { base };
    let fill = base.lerp_to_gamma(Color32::WHITE, 0.10 * t);
    let r = diameter * 0.5 + 1.0 * t;
    let p = ui.painter();
    p.circle_filled(rect.center(), r, fill);
    let glyph = if enabled { Color32::WHITE } else { Color32::from_gray(130) };
    match kind {
        ActionKind::Stop => icon_stop(p, rect.center(), diameter * 0.30, glyph),
        ActionKind::Play => icon_play(p, rect.center(), diameter * 0.34, glyph),
        ActionKind::Busy => {
            let time = ui.input(|i| i.time);
            icon_spinner(p, rect.center(), diameter * 0.22, time, WARN);
        }
        ActionKind::Cancel => {
            let time = ui.input(|i| i.time);
            icon_stop(p, rect.center(), diameter * 0.26, glyph);
            icon_spinner(p, rect.center(), diameter * 0.40, time, Color32::from_white_alpha(210));
        }
    }
    clickable && resp.clicked()
}

/// A section caption with room for a control on the right (e.g. a refresh button); both are centred on a
/// fixed 32 px row so they line up exactly.
pub fn section_row(ui: &mut Ui, text: &str, right: impl FnOnce(&mut Ui)) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.add_space(6.0);
        ui.label(egui::RichText::new(text).font(bold(12.5)).color(TEXT2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// 32 px round icon button with the refresh arrow.
pub fn refresh_button(ui: &mut Ui, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = enabled && resp.hovered();
    pointer(ui, hovered);
    let p = ui.painter();
    if hovered {
        p.circle_filled(rect.center(), 16.0, hover_fill());
    }
    icon_refresh(p, rect.center(), if enabled { TEXT } else { Color32::from_gray(90) });
    enabled && resp.clicked()
}

/// Full-width pill button (secondary actions).
pub fn pill_button(ui: &mut Ui, text: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    pointer(ui, resp.hovered());
    let fill = if resp.hovered() { Color32::from_white_alpha(22) } else { Color32::from_white_alpha(14) };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), fill);
    p.text(rect.center(), Align2::CENTER_CENTER, text, regular(14.0), TEXT);
    resp.clicked()
}

// ------------------------------------------------------------------ rows

/// What the user did with a speaker row.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RowAction {
    None,
    /// Row body clicked: make this the (only) main speaker.
    Select,
    /// The round "+" / check button clicked: add to / remove from the group playing together.
    ToggleMember,
}

/// Speaker row (inside a card with `pad = 4`). `selected` = main speaker, `member` = additional group member.
pub fn device_row(
    ui: &mut Ui,
    name: &str,
    sub: &str,
    is_homepod: bool,
    selected: bool,
    member: bool,
    enabled: bool,
) -> RowAction {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 50.0), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = enabled && resp.hovered();
    pointer(ui, hovered);

    let p = ui.painter().clone();
    let cr = CornerRadius::same(12);
    if selected {
        p.rect_filled(rect, cr, ACCENT.gamma_multiply(0.16));
    } else if member {
        p.rect_filled(rect, cr, ACCENT.gamma_multiply(0.09));
    } else if hovered {
        p.rect_filled(rect, cr, hover_fill());
    }

    let tile = Rect::from_center_size(pos2(rect.left() + 12.0 + 17.0, rect.center().y), vec2(34.0, 34.0));
    device_tile(&p, tile, is_homepod, enabled);

    let text_left = tile.right() + 12.0;
    let text_right = rect.right() - 14.0 - 24.0 - 8.0;
    let clip = Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_right, rect.bottom()));
    let pc = p.with_clip_rect(clip);
    let name_col = if enabled { TEXT } else { TEXT2 };
    pc.text(pos2(text_left, rect.center().y - 8.0), Align2::LEFT_CENTER, name, bold(14.5), name_col);
    pc.text(pos2(text_left, rect.center().y + 9.0), Align2::LEFT_CENTER, sub, regular(11.5), TEXT2);

    let mark = pos2(rect.right() - 14.0 - 12.0, rect.center().y);
    let mut toggled = false;
    if selected {
        icon_check(&p, mark, ACCENT);
    } else {
        // Round "+" button: add this speaker to the group (or remove it again).
        let hit = Rect::from_center_size(mark, vec2(34.0, 34.0));
        let tr = ui.interact(hit, resp.id.with("member"), if enabled { Sense::click() } else { Sense::hover() });
        let hot = enabled && tr.hovered();
        pointer(ui, hot || hovered && false);
        if member {
            p.circle_filled(mark, 12.0, ACCENT);
            icon_check(&p, mark, Color32::WHITE);
        } else {
            let col = if hot { TEXT } else { TEXT2 };
            p.circle_stroke(mark, 11.5, Stroke::new(1.4, col));
            p.line_segment([pos2(mark.x - 4.5, mark.y), pos2(mark.x + 4.5, mark.y)], Stroke::new(1.6, col));
            p.line_segment([pos2(mark.x, mark.y - 4.5), pos2(mark.x, mark.y + 4.5)], Stroke::new(1.6, col));
        }
        toggled = enabled && tr.clicked();
    }
    if toggled {
        RowAction::ToggleMember
    } else if enabled && resp.clicked() {
        RowAction::Select
    } else {
        RowAction::None
    }
}

/// Label on the left, iOS-style switch on the right. Long labels wrap. `sub` is an optional second line
/// in smaller grey type (used to say *which* speaker "connect on launch" refers to).
pub fn toggle_row(ui: &mut Ui, label: &str, sub: Option<&str>, value: &mut bool) -> bool {
    let w = ui.available_width();
    let (sw_w, sw_h, pad) = (46.0, 28.0, 14.0);
    let wrap = w - pad * 2.0 - sw_w - 16.0;
    let galley = ui.painter().layout(label.to_owned(), regular(14.5), TEXT, wrap);
    let sub_galley = sub.map(|s| ui.painter().layout(s.to_owned(), regular(12.5), TEXT2, wrap));
    let text_h = galley.size().y + sub_galley.as_ref().map(|g| g.size().y + 2.0).unwrap_or(0.0);
    let h = (text_h + 20.0).max(44.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    pointer(ui, resp.hovered());
    let mut changed = false;
    if resp.clicked() {
        *value = !*value;
        changed = true;
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *value, 0.14);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(12), hover_fill());
    }
    let top = rect.center().y - text_h * 0.5;
    let label_h = galley.size().y;
    p.galley(pos2(rect.left() + pad, top), galley, TEXT);
    if let Some(g) = sub_galley {
        p.galley(pos2(rect.left() + pad, top + label_h + 2.0), g, TEXT2);
    }

    let track = Rect::from_center_size(pos2(rect.right() - pad - sw_w * 0.5, rect.center().y), vec2(sw_w, sw_h));
    let off = Color32::from_white_alpha(38);
    p.rect_filled(track, CornerRadius::same((sw_h * 0.5) as u8), off.lerp_to_gamma(GOOD, t));
    let r = sw_h * 0.5 - 2.5;
    let x = egui::lerp((track.left() + sw_h * 0.5)..=(track.right() - sw_h * 0.5), t);
    p.circle_filled(pos2(x, track.center().y + 0.8), r + 0.8, Color32::from_black_alpha(50));
    p.circle_filled(pos2(x, track.center().y), r, Color32::WHITE);
    changed
}

/// Label with a chevron on the right (opens something).
pub fn nav_row(ui: &mut Ui, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    pointer(ui, resp.hovered());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(12), hover_fill());
    }
    p.text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, label, regular(14.5), TEXT);
    icon_chevron(p, pos2(rect.right() - 20.0, rect.center().y), TEXT2);
    resp.clicked()
}

// ------------------------------------------------------------------ controls

/// Segmented control. Returns true when the selection changed.
pub fn segmented(ui: &mut Ui, id: &str, labels: &[&str], selected: &mut usize) -> bool {
    let n = labels.len().max(1);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let inner = rect.shrink(3.0);
    let seg_w = inner.width() / n as f32;

    let mut changed = false;
    let mut hovered_idx = None;
    for i in 0..n {
        let seg = Rect::from_min_size(pos2(inner.left() + seg_w * i as f32, inner.top()), vec2(seg_w, inner.height()));
        let resp = ui.interact(seg, ui.id().with((id, i)), Sense::click());
        pointer(ui, resp.hovered());
        if resp.hovered() {
            hovered_idx = Some(i);
        }
        if resp.clicked() && *selected != i {
            *selected = i;
            changed = true;
        }
    }

    let anim = ui.ctx().animate_value_with_time(ui.id().with((id, "pill")), *selected as f32, 0.16);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(13), Color32::from_white_alpha(12));
    let pill = Rect::from_min_size(pos2(inner.left() + seg_w * anim, inner.top()), vec2(seg_w, inner.height()));
    p.rect_filled(pill, CornerRadius::same(10), Color32::from_gray(74));
    for (i, label) in labels.iter().enumerate() {
        let center = pos2(inner.left() + seg_w * (i as f32 + 0.5), inner.center().y);
        let active = *selected == i;
        let col = if active {
            TEXT
        } else if hovered_idx == Some(i) {
            Color32::from_gray(205)
        } else {
            TEXT2
        };
        let font = if active { bold(14.0) } else { regular(14.0) };
        p.with_clip_rect(Rect::from_center_size(center, vec2(seg_w - 6.0, inner.height())))
            .text(center, Align2::CENTER_CENTER, *label, font, col);
    }
    changed
}

pub struct SliderState {
    pub changed: bool,
    pub released: bool,
}

/// Thin track, round thumb. `value` is 0..=1.
pub fn slider(ui: &mut Ui, id: &str, value: &mut f32) -> SliderState {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let (left, right) = (rect.left() + 12.0, rect.right() - 12.0);
    let mut st = SliderState { changed: false, released: false };

    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let v = ((pos.x - left) / (right - left)).clamp(0.0, 1.0);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                st.changed = true;
            }
        }
    }
    st.released = resp.drag_stopped() || resp.clicked();

    let hot = resp.hovered() || resp.dragged();
    pointer(ui, hot);
    let t = ui.ctx().animate_bool_with_time(resp.id, hot, 0.12);
    let cy = rect.center().y;
    let x = egui::lerp(left..=right, value.clamp(0.0, 1.0));
    let p = ui.painter();
    let track = Rect::from_min_max(pos2(left, cy - 3.0), pos2(right, cy + 3.0));
    p.rect_filled(track, CornerRadius::same(3), Color32::from_white_alpha(34));
    p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), CornerRadius::same(3), ACCENT);
    let r = 9.5 + 1.5 * t;
    p.circle_filled(pos2(x, cy + 1.0), r + 1.0, Color32::from_black_alpha(70));
    p.circle_filled(pos2(x, cy), r, Color32::WHITE);
    st
}

/// Smooth level meter, 6 px high.
pub fn level_bar(ui: &mut Ui, level: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), Color32::from_white_alpha(26));
    let level = level.clamp(0.0, 1.0);
    let mut fill = rect;
    fill.set_width((rect.width() * level).max(0.0));
    if fill.width() > 1.0 {
        let color = if level < 0.7 {
            GOOD
        } else if level < 0.9 {
            WARN
        } else {
            BAD
        };
        p.rect_filled(fill, CornerRadius::same(3), color);
    }
}

/// Selectable chip used for the language grid. Draws into `rect`.
pub fn chip(ui: &mut Ui, id: &str, rect: Rect, text: &str, selected: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click());
    pointer(ui, resp.hovered());
    let p = ui.painter();
    let cr = CornerRadius::same(11);
    let fill = if selected {
        ACCENT.gamma_multiply(0.22)
    } else if resp.hovered() {
        Color32::from_white_alpha(20)
    } else {
        Color32::from_white_alpha(11)
    };
    p.rect_filled(rect, cr, fill);
    if selected {
        p.rect_stroke(rect, cr, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    }
    let font = if selected { bold(14.0) } else { regular(14.0) };
    p.with_clip_rect(rect.shrink2(vec2(6.0, 0.0)))
        .text(rect.center(), Align2::CENTER_CENTER, text, font, if selected { TEXT } else { Color32::from_gray(210) });
    resp.clicked()
}

/// Dropdown selector: a full-width button showing the current choice; clicking opens a list.
/// `options` are display names; returns `Some(index)` when the user picks a different one.
pub fn dropdown(ui: &mut Ui, id: &str, options: &[&str], selected: usize) -> Option<usize> {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
    pointer(ui, resp.hovered());
    let open_id = ui.id().with(("dropdown-open", id));
    let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
    if resp.clicked() {
        open = !open;
    }
    let p = ui.painter();
    let cr = CornerRadius::same(11);
    let fill = if resp.hovered() { Color32::from_white_alpha(20) } else { Color32::from_white_alpha(11) };
    p.rect_filled(rect, cr, fill);
    p.text(
        pos2(rect.left() + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        options.get(selected).copied().unwrap_or(""),
        regular(14.5),
        TEXT,
    );
    // down chevron
    let c = pos2(rect.right() - 22.0, rect.center().y);
    let col = if open { TEXT } else { TEXT2 };
    p.add(Shape::line(
        vec![pos2(c.x - 5.0, c.y - 2.5), pos2(c.x, c.y + 2.5), pos2(c.x + 5.0, c.y - 2.5)],
        Stroke::new(1.8, col),
    ));

    let mut picked = None;
    let popup = egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .width(w)
        .gap(6.0)
        .show(|ui| {
            ui.set_min_width(w - 16.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            for (i, name) in options.iter().enumerate() {
                let (r, rr) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                pointer(ui, rr.hovered());
                let p = ui.painter();
                let is_sel = i == selected;
                if rr.hovered() {
                    p.rect_filled(r, CornerRadius::same(8), Color32::from_white_alpha(20));
                } else if is_sel {
                    p.rect_filled(r, CornerRadius::same(8), ACCENT.gamma_multiply(0.18));
                }
                p.text(
                    pos2(r.left() + 12.0, r.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    if is_sel { bold(14.0) } else { regular(14.0) },
                    TEXT,
                );
                if is_sel {
                    icon_check(p, pos2(r.right() - 20.0, r.center().y), ACCENT);
                }
                if rr.clicked() && !is_sel {
                    picked = Some(i);
                }
            }
        });
    // popup.is_none() means it is closed (clicked an option / outside / Esc)
    ui.data_mut(|d| d.insert_temp(open_id, popup.is_some()));
    picked
}

/// Round status dot.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

// ------------------------------------------------------------------ main screen (redesign)

/// Settings gear: a ring with eight teeth around a small hub.
pub fn icon_gear(p: &egui::Painter, c: Pos2, color: Color32) {
    p.circle_stroke(c, 5.6, Stroke::new(1.8, color));
    p.circle_stroke(c, 2.0, Stroke::new(1.6, color));
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let (s, co) = a.sin_cos();
        p.line_segment([pos2(c.x + co * 6.6, c.y + s * 6.6), pos2(c.x + co * 9.0, c.y + s * 9.0)], Stroke::new(2.4, color));
    }
}

/// Chevron pointing left ("back").
pub fn icon_back(p: &egui::Painter, c: Pos2, color: Color32) {
    p.add(Shape::line(
        vec![pos2(c.x + 2.5, c.y - 6.0), pos2(c.x - 3.0, c.y), pos2(c.x + 2.5, c.y + 6.0)],
        Stroke::new(2.0, color),
    ));
}

/// Chevron pointing down (an open list).
fn icon_chevron_down(p: &egui::Painter, c: Pos2, color: Color32) {
    p.add(Shape::line(
        vec![pos2(c.x - 5.0, c.y - 2.5), pos2(c.x, c.y + 2.5), pos2(c.x + 5.0, c.y - 2.5)],
        Stroke::new(1.8, color),
    ));
}

/// 34 px round icon button; `back` draws a left chevron, otherwise the settings gear.
pub fn header_icon_button(ui: &mut Ui, back: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let hovered = resp.hovered();
    pointer(ui, hovered);
    let p = ui.painter();
    if hovered {
        p.circle_filled(rect.center(), 17.0, hover_fill());
    }
    let col = if hovered { TEXT } else { TEXT2 };
    if back {
        icon_back(p, rect.center(), col);
    } else {
        icon_gear(p, rect.center(), col);
    }
    resp.clicked()
}

/// Header of the speaker card: caption, the chosen speaker's name, an optional refresh button and (when the
/// list can be opened) a chevron. Returns (header clicked, refresh clicked).
pub fn speaker_header(
    ui: &mut Ui,
    caption: &str,
    name: &str,
    is_homepod: bool,
    chevron: Option<bool>,
    refresh: Option<bool>,
) -> (bool, bool) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 58.0), if chevron.is_some() { Sense::click() } else { Sense::hover() });
    let mut right = rect.right() - 12.0;

    let mut refresh_clicked = false;
    let mut refresh_hot = false;
    let mut chev_center = None;
    if chevron.is_some() {
        chev_center = Some(pos2(right - 6.0, rect.center().y));
        right -= 30.0;
    }
    let mut refresh_center = None;
    if let Some(enabled) = refresh {
        let center = pos2(right - 14.0, rect.center().y);
        let hit = Rect::from_center_size(center, vec2(34.0, 34.0));
        let r = ui.interact(hit, resp.id.with("refresh"), if enabled { Sense::click() } else { Sense::hover() });
        refresh_hot = enabled && r.hovered();
        pointer(ui, refresh_hot);
        refresh_clicked = enabled && r.clicked();
        refresh_center = Some((center, enabled));
        right -= 38.0;
    }

    let hovered = chevron.is_some() && resp.hovered() && !refresh_hot;
    pointer(ui, hovered);
    let p = ui.painter().clone();
    if hovered {
        p.rect_filled(rect, CornerRadius::same(12), hover_fill());
    }
    if let Some((center, enabled)) = refresh_center {
        if refresh_hot {
            p.circle_filled(center, 16.0, hover_fill());
        }
        icon_refresh(&p, center, if enabled { TEXT } else { Color32::from_gray(90) });
    }
    if let (Some(open), Some(c)) = (chevron, chev_center) {
        if open {
            icon_chevron_down(&p, c, TEXT2);
        } else {
            icon_chevron(&p, c, TEXT2);
        }
    }

    let tile = Rect::from_center_size(pos2(rect.left() + 12.0 + 17.0, rect.center().y), vec2(34.0, 34.0));
    device_tile(&p, tile, is_homepod, true);
    let text_left = tile.right() + 12.0;
    let clip = Rect::from_min_max(pos2(text_left, rect.top()), pos2(right.max(text_left + 20.0), rect.bottom()));
    let pc = p.with_clip_rect(clip);
    pc.text(pos2(text_left, rect.center().y - 9.0), Align2::LEFT_CENTER, caption, regular(11.5), TEXT2);
    pc.text(pos2(text_left, rect.center().y + 8.0), Align2::LEFT_CENTER, name, bold(15.0), TEXT);
    (chevron.is_some() && resp.clicked() && !refresh_clicked, refresh_clicked)
}

/// One speaker in the list: a round mark when exactly one speaker plays, a checkbox when several play together.
/// `badge` is a small tag after the name (the main speaker of a group). Returns true when the row was clicked.
pub fn speaker_row(
    ui: &mut Ui,
    name: &str,
    is_homepod: bool,
    checked: bool,
    multi: bool,
    badge: Option<&str>,
    enabled: bool,
) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = enabled && resp.hovered();
    pointer(ui, hovered);
    let p = ui.painter().clone();
    let cr = CornerRadius::same(12);
    if checked {
        p.rect_filled(rect, cr, ACCENT.gamma_multiply(0.16));
    } else if hovered {
        p.rect_filled(rect, cr, hover_fill());
    }
    let tile = Rect::from_center_size(pos2(rect.left() + 12.0 + 15.0, rect.center().y), vec2(30.0, 30.0));
    device_tile(&p, tile, is_homepod, enabled);

    let text_left = tile.right() + 12.0;
    let col = if enabled { TEXT } else { TEXT2 };
    let g = p.layout_no_wrap(name.to_owned(), bold(14.5), col);
    p.galley(pos2(text_left, rect.center().y - g.size().y * 0.5), g.clone(), col);
    if let Some(b) = badge {
        let bg = p.layout_no_wrap(b.to_owned(), regular(11.0), ACCENT);
        let brect = Rect::from_min_size(
            pos2(text_left + g.size().x + 8.0, rect.center().y - (bg.size().y + 4.0) * 0.5),
            vec2(bg.size().x + 12.0, bg.size().y + 4.0),
        );
        p.rect_stroke(brect, CornerRadius::same(9), Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Inside);
        p.galley(pos2(brect.left() + 6.0, brect.top() + 2.0), bg, ACCENT);
    }

    let mark = pos2(rect.right() - 14.0 - 9.0, rect.center().y);
    if multi {
        let r = Rect::from_center_size(mark, vec2(18.0, 18.0));
        if checked {
            p.rect_filled(r, CornerRadius::same(5), ACCENT);
            icon_check(&p, mark, Color32::WHITE);
        } else {
            p.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.6, TEXT2), StrokeKind::Inside);
        }
    } else {
        p.circle_stroke(mark, 9.0, Stroke::new(1.6, if checked { ACCENT } else { TEXT2 }));
        if checked {
            p.circle_filled(mark, 4.5, ACCENT);
        }
    }
    enabled && resp.clicked()
}
