//! Palette, fonts, icons and formatting from the Claude Design mock-up
//! (`Pincers.dc.html`). Colours are copied 1:1 from its CSS.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect,
    Shape, Stroke, StrokeKind, Vec2, pos2, text::LayoutJob, vec2,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x0d, 0x0e, 0x10);
pub const CHROME: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x0e);
pub const PANEL: Color32 = Color32::from_rgb(0x10, 0x11, 0x14);
pub const INPUT: Color32 = Color32::from_rgb(0x14, 0x15, 0x18);
pub const HOVER: Color32 = Color32::from_rgb(0x16, 0x18, 0x1b);
pub const RAISED: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x21);
pub const LINE: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x23);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x17, 0x19, 0x1c);
pub const TRACK: Color32 = Color32::from_rgb(0x25, 0x28, 0x2d);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xeb, 0xe8);
pub const TEXT2: Color32 = Color32::from_rgb(0xc9, 0xc8, 0xc4);
pub const MUTED: Color32 = Color32::from_rgb(0x8d, 0x90, 0x97);
pub const DIM: Color32 = Color32::from_rgb(0x5c, 0x5f, 0x66);
pub const ACCENT: Color32 = Color32::from_rgb(0xf0, 0x64, 0x3a);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xff, 0x75, 0x47);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x16, 0x0a, 0x05);
pub const GREEN: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x8a);
pub const AMBER: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x4a);
pub const RED: Color32 = Color32::from_rgb(0xf0, 0x60, 0x5d);
pub const CLOSE_RED: Color32 = Color32::from_rgb(0xc4, 0x2b, 0x1c);
pub const DANGER_BG: Color32 = Color32::from_rgb(0x2a, 0x15, 0x14);
pub const DANGER_LINE: Color32 = Color32::from_rgb(0x3d, 0x1d, 0x1b);

/// `rgba(r,g,b,.12)` pill backgrounds.
pub fn tint(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8)
}

#[derive(Clone, Copy)]
pub enum W {
    Regular,
    Medium,
    Semi,
}

pub fn sans(size: f32, w: W) -> FontId {
    let fam = match w {
        W::Regular => FontFamily::Proportional,
        W::Medium => FontFamily::Name("medium".into()),
        W::Semi => FontFamily::Name("semi".into()),
    };
    FontId::new(size, fam)
}

pub fn mono(size: f32, w: W) -> FontId {
    match w {
        W::Regular => FontId::monospace(size),
        _ => FontId::new(size, FontFamily::Name("mono-medium".into())),
    }
}

/// Geist + Geist Mono (SIL OFL, `assets/fonts`), embedded in the binary.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, data: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
    };
    add(&mut fonts, "geist", include_bytes!("../../assets/fonts/Geist-Regular.ttf"));
    add(&mut fonts, "geist-medium", include_bytes!("../../assets/fonts/Geist-Medium.ttf"));
    add(&mut fonts, "geist-semi", include_bytes!("../../assets/fonts/Geist-SemiBold.ttf"));
    add(&mut fonts, "geist-mono", include_bytes!("../../assets/fonts/GeistMono-Regular.ttf"));
    add(&mut fonts, "geist-mono-medium", include_bytes!("../../assets/fonts/GeistMono-Medium.ttf"));

    // Keep egui's bundled fonts as fallbacks for glyphs Geist lacks.
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |first: &str| {
        let mut v = vec![first.to_owned()];
        v.extend(fallback.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, family("geist"));
    fonts.families.insert(FontFamily::Monospace, family("geist-mono"));
    fonts.families.insert(FontFamily::Name("medium".into()), family("geist-medium"));
    fonts.families.insert(FontFamily::Name("semi".into()), family("geist-semi"));
    fonts.families.insert(FontFamily::Name("mono-medium".into()), family("geist-mono-medium"));
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, TRACK);
    v.window_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = HOVER;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = tint(ACCENT, 0.45);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, PANEL),
        (&mut v.widgets.inactive, RAISED),
        (&mut v.widgets.hovered, TRACK),
        (&mut v.widgets.active, TRACK),
        (&mut v.widgets.open, RAISED),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = CornerRadius::same(7);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, TRACK);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, DIM);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT2);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    // Pin the dark theme: by default egui follows the system theme, and a
    // light Windows would give light text fields, sliders and windows.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(egui::TextStyle::Body, sans(13.0, W::Regular));
        s.text_styles.insert(egui::TextStyle::Button, sans(12.5, W::Regular));
        s.text_styles.insert(egui::TextStyle::Small, sans(11.0, W::Regular));
        s.text_styles.insert(egui::TextStyle::Monospace, mono(12.0, W::Regular));
        s.text_styles.insert(egui::TextStyle::Heading, sans(15.0, W::Semi));
    });
}

// ------------------------------------------------------------- text helpers

/// Lays out one line, ellipsised to `max_w`, with optional letter spacing.
pub fn line(painter: &Painter, s: &str, font: FontId, color: Color32, max_w: f32, spacing: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(
        s.to_owned(),
        egui::TextFormat { font_id: font, color, extra_letter_spacing: spacing, ..Default::default() },
    );
    job.wrap.max_width = max_w.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    painter.layout_job(job)
}

/// Paints a single line anchored at `pos` with alignment `align`; returns its rect.
pub fn text(painter: &Painter, pos: Pos2, align: Align2, s: &str, font: FontId, color: Color32, max_w: f32) -> Rect {
    let g = line(painter, s, font, color, max_w, 0.0);
    let rect = align.anchor_size(pos, g.size());
    painter.galley(rect.min, g, color);
    rect
}

pub fn caps(painter: &Painter, pos: Pos2, s: &str, size: f32, color: Color32) {
    let g = line(painter, &s.to_uppercase(), sans(size, W::Medium), color, f32::INFINITY, size * 0.07);
    let rect = Align2::LEFT_CENTER.anchor_size(pos, g.size());
    painter.galley(rect.min, g, color);
}

/// Status pill: `● Concluído` on a 12 % tint.
pub fn pill(painter: &Painter, left_center: Pos2, label: &str, color: Color32) -> Rect {
    let g = line(painter, label, sans(11.5, W::Medium), color, f32::INFINITY, 0.0);
    let w = 8.0 + 5.0 + 6.0 + g.size().x + 8.0;
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - 11.0), vec2(w, 22.0));
    painter.rect_filled(rect, CornerRadius::same(6), tint(color, 0.12));
    painter.circle_filled(pos2(rect.left() + 10.5, rect.center().y), 2.5, color);
    painter.galley(pos2(rect.left() + 19.0, rect.center().y - g.size().y / 2.0), g, color);
    rect
}

// ------------------------------------------------------------------ icons
// Hand-drawn versions of the mock-up's inline SVGs (16×16 or 12×12 grids).

fn map(r: Rect, grid: f32) -> impl Fn(f32, f32) -> Pos2 {
    let s = r.width().min(r.height()) / grid;
    let o = r.center() - Vec2::splat(grid * s / 2.0);
    move |x, y| pos2(o.x + x * s, o.y + y * s)
}

pub fn icon_pause(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 12.0);
    p.rect_filled(Rect::from_min_max(m(2.0, 1.5), m(4.6, 10.5)), CornerRadius::same(1), c);
    p.rect_filled(Rect::from_min_max(m(7.4, 1.5), m(10.0, 10.5)), CornerRadius::same(1), c);
}

pub fn icon_play(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 12.0);
    p.add(Shape::convex_polygon(vec![m(3.0, 1.5), m(10.6, 6.0), m(3.0, 10.5)], c, Stroke::NONE));
}

pub fn icon_folder(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 16.0);
    let pts = vec![m(2.0, 4.5), m(6.0, 4.5), m(7.5, 6.0), m(14.0, 6.0), m(14.0, 13.5), m(2.0, 13.5)];
    p.add(Shape::closed_line(pts, Stroke::new(1.3, c)));
}

pub fn icon_download(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 16.0);
    let s = Stroke::new(1.8, c);
    p.line_segment([m(8.0, 2.0), m(8.0, 11.0)], s);
    p.add(Shape::line(vec![m(4.0, 7.5), m(8.0, 11.5), m(12.0, 7.5)], s));
    p.line_segment([m(3.0, 14.0), m(13.0, 14.0)], s);
}

pub fn icon_link(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 16.0);
    let s = Stroke::new(1.5, c);
    p.line_segment([m(6.5, 9.5), m(9.5, 6.5)], s);
    p.circle_stroke(m(10.6, 5.4), 2.6, s);
    p.circle_stroke(m(5.4, 10.6), 2.6, s);
}

pub fn icon_gear(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 16.0);
    let s = Stroke::new(1.4, c);
    p.circle_stroke(m(8.0, 8.0), 2.2 * r.width() / 16.0, s);
    for (a, b) in [
        ((8.0, 1.5), (8.0, 3.5)),
        ((8.0, 12.5), (8.0, 14.5)),
        ((1.5, 8.0), (3.5, 8.0)),
        ((12.5, 8.0), (14.5, 8.0)),
        ((3.4, 3.4), (4.8, 4.8)),
        ((11.2, 11.2), (12.6, 12.6)),
        ((3.4, 12.6), (4.8, 11.2)),
        ((11.2, 4.8), (12.6, 3.4)),
    ] {
        p.line_segment([m(a.0, a.1), m(b.0, b.1)], s);
    }
}

pub fn icon_trash(p: &Painter, r: Rect, c: Color32) {
    let m = map(r, 16.0);
    let s = Stroke::new(1.4, c);
    p.line_segment([m(2.5, 4.0), m(13.5, 4.0)], s);
    p.add(Shape::line(vec![m(6.0, 4.0), m(6.0, 2.5), m(10.0, 2.5), m(10.0, 4.0)], s));
    p.add(Shape::line(vec![m(4.0, 4.0), m(4.7, 13.5), m(11.3, 13.5), m(12.0, 4.0)], s));
}

pub fn icon_min(p: &Painter, r: Rect, c: Color32) {
    p.line_segment([r.center() - vec2(5.0, 0.0), r.center() + vec2(5.0, 0.0)], Stroke::new(1.0, c));
}

pub fn icon_max(p: &Painter, r: Rect, c: Color32, restore: bool) {
    let s = Stroke::new(1.0, c);
    if restore {
        let a = Rect::from_center_size(r.center() + vec2(-1.0, 1.0), Vec2::splat(8.0));
        p.rect_stroke(a, CornerRadius::ZERO, s, StrokeKind::Inside);
        p.add(Shape::line(
            vec![a.left_top() + vec2(2.0, -2.0), a.right_top() + vec2(2.0, -2.0), a.right_bottom() + vec2(2.0, -2.0)],
            s,
        ));
    } else {
        p.rect_stroke(Rect::from_center_size(r.center(), Vec2::splat(10.0)), CornerRadius::ZERO, s, StrokeKind::Inside);
    }
}

pub fn icon_close(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.0, c);
    let h = 5.0;
    p.line_segment([r.center() + vec2(-h, -h), r.center() + vec2(h, h)], s);
    p.line_segment([r.center() + vec2(h, -h), r.center() + vec2(-h, h)], s);
}

// ------------------------------------------------------------- formatting

const KB: f64 = 1024.0;
const MB: f64 = KB * 1024.0;
const GB: f64 = MB * 1024.0;

/// Same rules as the mock-up's `fmt()`: `812 B`, `811.9 KB`, `342.0 MB`, `5.80 GB`.
pub fn size(b: f64) -> String {
    // `max` also turns the `-0.0` of an empty float sum into `0.0`.
    let b = b.max(0.0);
    if b < KB {
        format!("{b:.0} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.2} GB", b / GB)
    }
}

pub fn eta(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "—".into();
    }
    let s = secs.round() as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, s / 60 % 60)
    }
}

/// `Hoje, 17:52` / `Ontem, 22:08` / `12/10/2026 09:15`, in local time.
pub fn when(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts, 0).single().filter(|_| ts > 0) else { return String::new() };
    let today = Local::now().date_naive();
    let d = t.date_naive();
    let hm = t.format("%H:%M");
    if d == today {
        crate::tr!("time.today", time = hm)
    } else if Some(d) == today.pred_opt() {
        crate::tr!("time.yesterday", time = hm)
    } else {
        // The pattern itself is translatable (day/month order differs).
        let pattern = crate::tr!("time.date_format");
        let mut out = String::new();
        use std::fmt::Write;
        match write!(out, "{}", t.format(&pattern)) {
            Ok(()) => out,
            Err(_) => t.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}
