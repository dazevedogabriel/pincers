//! The three columns of the mock-up: sidebar, list and detail panel, plus the
//! options window. Everything is painted by hand to match its CSS sizes.

use super::theme::*;
use super::{Filter, Item, MapView, PincersApp, Status, open_folder};
use crate::engine::SegmentView;
use crate::{tr, tr_n};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Id, Margin, Painter, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};

// ------------------------------------------------------------ small widgets

#[derive(Clone, Copy)]
enum Btn {
    /// Transparent with a border (Pausar todos, Abrir pasta).
    Outline,
    /// Filled raised surface (the detail panel's primary action).
    Raised,
    /// Orange call to action (Baixar).
    Accent,
    /// Square trash button that turns red on hover.
    Danger,
}

type Icon = fn(&Painter, Rect, Color32);

fn button(ui: &mut Ui, rect: Rect, id: &str, label: &str, icon: Option<Icon>, kind: Btn, font: egui::FontId) -> Response {
    let resp = ui.interact(rect, Id::unique(id), Sense::click());
    let h = resp.hovered();
    let p = ui.painter();
    let (fill, stroke, fg, radius) = match kind {
        Btn::Outline => (if h { HOVER } else { Color32::TRANSPARENT }, TRACK, if h { TEXT } else { TEXT2 }, 8),
        Btn::Raised => (if h { TRACK } else { RAISED }, TRACK, TEXT, 8),
        Btn::Accent => (if h { ACCENT_HOVER } else { ACCENT }, Color32::TRANSPARENT, ON_ACCENT, 10),
        Btn::Danger => (
            if h { DANGER_BG } else { Color32::TRANSPARENT },
            if h { DANGER_LINE } else { TRACK },
            if h { RED } else { MUTED },
            8,
        ),
    };
    p.rect(rect, CornerRadius::same(radius), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let g = line(p, label, font, fg, rect.width(), 0.0);
    let icon_w = if icon.is_some() { if label.is_empty() { 13.0 } else { 18.0 } } else { 0.0 };
    let x0 = rect.center().x - (icon_w + g.size().x) / 2.0;
    if let Some(icon) = icon {
        icon(p, Rect::from_min_size(pos2(x0, rect.center().y - 6.5), vec2(13.0, 13.0)), fg);
    }
    p.galley(pos2(x0 + icon_w, rect.center().y - g.size().y / 2.0), g, fg);
    if h {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn label_width(ui: &Ui, s: &str, font: egui::FontId) -> f32 {
    line(ui.painter(), s, font, TEXT, f32::INFINITY, 0.0).size().x
}

/// Parts bar: one track per part, widths proportional to its length.
fn parts_bar(p: &Painter, rect: Rect, item: &Item, parts: &[SegmentView], track: Color32, radius: u8) {
    let total: u64 = parts.iter().map(|s| s.end.saturating_sub(s.start)).sum::<u64>().max(1);
    let n = parts.len().max(1) as f32;
    let gap = if rect.width() / n > 6.0 { 2.0 } else { 0.0 };
    let usable = rect.width() - gap * (n - 1.0);
    let mut x = rect.left();
    for s in parts {
        let len = s.end.saturating_sub(s.start).max(1);
        let w = usable * len as f32 / total as f32;
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w.max(1.0), rect.height()));
        p.rect_filled(r, CornerRadius::same(radius), track);
        let f = (s.pos.saturating_sub(s.start)) as f32 / len as f32;
        let f = if matches!(item.status, Status::Done(_)) { 1.0 } else { f.clamp(0.0, 1.0) };
        if f > 0.0 {
            let fill = Rect::from_min_size(r.min, vec2(r.width() * f, r.height()));
            p.rect_filled(fill, CornerRadius::same(radius), item.part_color(s));
        }
        x += w + gap;
    }
}

// ------------------------------------------------------------------ sidebar

pub fn sidebar(app: &mut PincersApp, ui: &mut Ui) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.vline(full.right() - 0.5, full.y_range(), Stroke::new(1.0, LINE));
    let left = full.left() + 10.0;
    let width = full.width() - 20.0;

    caps(&p, pos2(left + 10.0, full.top() + 16.0 + 7.0), &tr!("sidebar.downloads"), 11.0, DIM);
    let mut y = full.top() + 16.0 + 14.0 + 8.0;
    for f in Filter::ALL {
        let r = Rect::from_min_size(pos2(left, y), vec2(width, 32.0));
        let resp = ui.interact(r, Id::unique(("filter", f.key())), Sense::click());
        let on = app.filter == f;
        let bg = if on { RAISED } else if resp.hovered() { HOVER } else { Color32::TRANSPARENT };
        p.rect_filled(r, CornerRadius::same(7), bg);
        p.rect_filled(Rect::from_center_size(pos2(r.left() + 13.0, r.center().y), vec2(6.0, 6.0)), CornerRadius::same(2), f.dot());
        text(&p, pos2(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, &f.label(), sans(13.0, W::Regular), if on { TEXT } else { MUTED }, 140.0);
        let count = app.items.iter().filter(|i| f.matches(&i.status)).count();
        text(&p, pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, &count.to_string(), mono(11.0, W::Regular), DIM, 40.0);
        if resp.clicked() {
            app.filter = f;
        }
        y += 34.0;
    }

    // Options row at the very bottom, total-speed card above it.
    let opt = Rect::from_min_size(pos2(left, full.bottom() - 16.0 - 34.0), vec2(width, 34.0));
    let card_h = 12.0 + 14.0 + 10.0 + 24.0 + 10.0 + 28.0 + 10.0 + 14.0 + 12.0;
    let card = Rect::from_min_size(pos2(left, opt.top() - 8.0 - card_h), vec2(width, card_h));
    p.rect(card, CornerRadius::same(10), PANEL, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let cx = card.left() + 12.0;
    let mut cy = card.top() + 12.0;
    text(&p, pos2(cx, cy + 7.0), Align2::LEFT_CENTER, &tr!("sidebar.total_speed"), sans(11.0, W::Regular), MUTED, 180.0);
    cy += 14.0 + 10.0;
    text(&p, pos2(cx, cy + 12.0), Align2::LEFT_CENTER, &format!("{}/s", size(app.total_speed())), mono(20.0, W::Medium), TEXT, 180.0);
    cy += 24.0 + 10.0;
    let hist = Rect::from_min_size(pos2(cx, cy), vec2(card.width() - 24.0, 28.0));
    bars(&p, hist, app.total_hist.iter().copied().collect(), 32, 0.03, tint(ACCENT, 0.75));
    cy += 28.0 + 10.0;
    text(&p, pos2(cx, cy + 7.0), Align2::LEFT_CENTER, &tr_n!("sidebar.active_connections", app.active_connections()), sans(11.0, W::Regular), DIM, 180.0);

    let resp = ui.interact(opt, Id::unique("opt-row"), Sense::click());
    let h = resp.hovered() || app.show_settings;
    p.rect_filled(opt, CornerRadius::same(7), if h { HOVER } else { Color32::TRANSPARENT });
    let c = if h { TEXT } else { MUTED };
    icon_gear(&p, Rect::from_center_size(pos2(opt.left() + 17.5, opt.center().y), vec2(15.0, 15.0)), c);
    text(&p, pos2(opt.left() + 35.0, opt.center().y), Align2::LEFT_CENTER, &tr!("sidebar.options"), sans(13.0, W::Regular), c, 120.0);
    if resp.clicked() {
        app.show_settings = !app.show_settings;
    }
}

/// Bottom-aligned bar chart (speed histories). `min` is the smallest bar as
/// a fraction of the height, like the mock-up's `max(3, v/max*100)%`.
fn bars(p: &Painter, rect: Rect, values: Vec<f64>, count: usize, min: f32, color: Color32) {
    let gap = 2.0;
    let w = (rect.width() - gap * (count as f32 - 1.0)) / count as f32;
    let max = values.iter().copied().fold(1.0_f64, f64::max);
    let pad = count.saturating_sub(values.len());
    for i in 0..count {
        let v = if i < pad { 0.0 } else { values[i - pad] };
        let h = ((v / max) as f32).max(min) * rect.height();
        let x = rect.left() + i as f32 * (w + gap);
        let r = Rect::from_min_max(pos2(x, rect.bottom() - h), pos2(x + w, rect.bottom()));
        p.rect_filled(r, CornerRadius { nw: 1, ne: 1, sw: 0, se: 0 }, color);
    }
}

// --------------------------------------------------------------------- main

struct Cols {
    x: Vec<(f32, f32)>,
    stats: bool,
}

/// `grid-template-columns` of the mock-up, resolved for a given inner width.
fn columns(left: f32, width: f32, stats: bool) -> Cols {
    let gap = 14.0;
    let (fixed, fr, mins): (Vec<f32>, [f32; 2], [f32; 2]) = if stats {
        (vec![78.0, 64.0, 92.0, 28.0], [1.5, 1.0], [140.0, 110.0])
    } else {
        (vec![92.0, 28.0], [1.4, 1.0], [120.0, 90.0])
    };
    let n = fixed.len() + 2;
    let free = width - fixed.iter().sum::<f32>() - gap * (n as f32 - 1.0);
    let unit = free / (fr[0] + fr[1]);
    let mut widths = vec![(fr[0] * unit).max(mins[0]), (fr[1] * unit).max(mins[1])];
    widths.extend(fixed);
    let mut x = left;
    let cols = widths
        .into_iter()
        .map(|w| {
            let c = (x, w);
            x += w + gap;
            c
        })
        .collect();
    Cols { x: cols, stats }
}

pub fn main(app: &mut PincersApp, ui: &mut Ui, narrow: bool) {
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    let p = ui.painter().clone();

    // ---- add-URL bar (padding 16 20, 40 px controls)
    let bar = Rect::from_min_size(full.min, vec2(full.width(), 72.0));
    p.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, LINE));
    let cy = bar.center().y;
    let btn_font = sans(13.0, W::Semi);
    let download_label = tr!("bar.download");
    let btn_w = 16.0 + 14.0 + 8.0 + label_width(ui, &download_label, btn_font.clone()) + 16.0;
    let btn = Rect::from_min_size(pos2(bar.right() - 20.0 - btn_w, cy - 20.0), vec2(btn_w, 40.0));
    let opts = [4usize, 8, 16, 32];
    let parts_label = tr!("bar.parts");
    let parts_label_w = label_width(ui, &parts_label, sans(11.0, W::Regular)) + 12.0;
    let parts_w = 3.0 + parts_label_w + opts.len() as f32 * 28.0 + (opts.len() as f32 - 1.0) * 2.0 + 2.0 + 3.0;
    let parts = Rect::from_min_size(pos2(btn.left() - 10.0 - parts_w, cy - 20.0), vec2(parts_w, 40.0));
    let field = Rect::from_min_max(pos2(bar.left() + 20.0, cy - 20.0), pos2(parts.left() - 10.0, cy + 20.0));

    let url_id = Id::unique("url-input");
    let focused = ctx.memory(|m| m.has_focus(url_id));
    p.rect(field, CornerRadius::same(10), INPUT, Stroke::new(1.0, if focused { ACCENT } else { TRACK }), StrokeKind::Inside);
    icon_link(&p, Rect::from_center_size(pos2(field.left() + 14.0 + 7.5, cy), vec2(15.0, 15.0)), DIM);
    let edit_rect = Rect::from_min_max(pos2(field.left() + 39.0, cy - 10.0), pos2(field.right() - 8.0, cy + 10.0));
    let edit = ui.put(
        edit_rect,
        egui::TextEdit::singleline(&mut app.url_input)
            .id(url_id)
            .frame(egui::Frame::NONE)
            .font(mono(12.5, W::Regular))
            .text_color(TEXT)
            .hint_text(RichText::new(tr!("bar.url_hint")).color(DIM).font(mono(12.5, W::Regular)))
            .align(egui::Align2::LEFT_CENTER),
    );
    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

    p.rect(parts, CornerRadius::same(7), INPUT, Stroke::new(1.0, TRACK), StrokeKind::Inside);
    text(&p, pos2(parts.left() + 3.0 + 6.0, cy), Align2::LEFT_CENTER, &parts_label, sans(11.0, W::Regular), DIM, 80.0);
    let mut x = parts.left() + 3.0 + parts_label_w;
    for n in opts {
        let r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(28.0, 24.0));
        let resp = ui.interact(r, Id::unique(("parts", n)), Sense::click());
        let on = app.settings.connections == n;
        if on {
            p.rect_filled(r, CornerRadius::same(5), TRACK);
        }
        let c = if on || resp.hovered() { TEXT } else { DIM };
        text(&p, r.center(), Align2::CENTER_CENTER, &n.to_string(), mono(11.5, W::Regular), c, 28.0);
        if resp.clicked() {
            app.settings.connections = n;
            app.mark_dirty();
        }
        resp.on_hover_text(tr_n!("bar.parts_tip", n));
        x += 30.0;
    }

    let clicked = button(ui, btn, "baixar", &download_label, Some(icon_download), Btn::Accent, btn_font).clicked();
    if clicked || enter {
        let url = std::mem::take(&mut app.url_input);
        app.add_url(url, &ctx);
        if enter {
            ctx.memory_mut(|m| m.request_focus(url_id));
        }
    }

    // ---- list toolbar (padding 14 20 10)
    let tb = Rect::from_min_size(pos2(full.left(), bar.bottom()), vec2(full.width(), 54.0));
    let ty = tb.top() + 14.0 + 15.0;
    let list: Vec<u64> = app.items.iter().filter(|i| app.filter.matches(&i.status)).map(|i| i.id).collect();
    let title = text(&p, pos2(tb.left() + 20.0, ty), Align2::LEFT_CENTER, &app.filter.label(), sans(15.0, W::Semi), TEXT, 200.0);
    let count = tr_n!("list.count", list.len());
    text(&p, pos2(title.right() + 10.0, ty + 1.0), Align2::LEFT_CENTER, &count, sans(12.0, W::Regular), DIM, 100.0);

    let f12 = sans(12.0, W::Regular);
    let (resume_label, pause_label) = (tr!("list.resume_all"), tr!("list.pause_all"));
    let rw = 12.0 + 18.0 + label_width(ui, &resume_label, f12.clone()) + 12.0;
    let pw = 12.0 + 18.0 + label_width(ui, &pause_label, f12.clone()) + 12.0;
    let resume = Rect::from_min_size(pos2(tb.right() - 20.0 - rw, ty - 15.0), vec2(rw, 30.0));
    let pause = Rect::from_min_size(pos2(resume.left() - 6.0 - pw, ty - 15.0), vec2(pw, 30.0));
    if button(ui, pause, "pause-all", &pause_label, Some(icon_pause), Btn::Outline, f12.clone()).clicked() {
        app.pause_all();
    }
    if button(ui, resume, "resume-all", &resume_label, Some(icon_play), Btn::Outline, f12.clone()).clicked() {
        app.resume_all(&ctx);
    }
    // Without the sidebar (narrow window) filters and options move here.
    if narrow {
        let gear = Rect::from_min_size(pos2(pause.left() - 6.0 - 30.0, ty - 15.0), vec2(30.0, 30.0));
        if button(ui, gear, "gear", "", Some(icon_gear), Btn::Outline, f12.clone()).on_hover_text(tr!("sidebar.options")).clicked() {
            app.show_settings = !app.show_settings;
        }
        let combo = Rect::from_min_size(pos2(gear.left() - 6.0 - 130.0, ty - 13.0), vec2(130.0, 26.0));
        let mut filter = app.filter;
        ui.put(combo, |ui: &mut Ui| {
            egui::ComboBox::from_id_salt("filter-combo")
                .selected_text(filter.label())
                .width(124.0)
                .show_ui(ui, |ui| {
                    for f in Filter::ALL {
                        ui.selectable_value(&mut filter, f, f.label());
                    }
                })
                .response
        });
        app.filter = filter;
    }

    // ---- column header (padding 0 20 8 32)
    let stats = full.width() >= 700.0;
    let cols = columns(full.left() + 32.0, full.width() - 32.0 - 20.0, stats);
    let hy = tb.bottom() + 6.0;
    let mut heads = vec![(tr!("col.file"), false), (tr!("col.parts"), false)];
    if stats {
        heads.extend([(tr!("col.speed"), true), (tr!("col.remaining"), true)]);
    }
    heads.push((tr!("col.status"), false));
    for ((label, right), (x, w)) in heads.into_iter().zip(cols.x.iter()) {
        let g = line(&p, &label.to_uppercase(), sans(11.0, W::Regular), DIM, f32::INFINITY, 0.66);
        let gx = if right { x + w - g.size().x } else { *x };
        p.galley(pos2(gx, hy - g.size().y / 2.0), g, DIM);
    }

    // ---- rows (padding 0 12 16)
    let list_rect = Rect::from_min_max(pos2(full.left(), hy + 14.0), full.max);
    let mut actions: Vec<(u64, &'static str)> = Vec::new();
    let mut ui_list = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut ui_list, |ui| {
        let w = ui.available_width();
        if list.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                if app.items.is_empty() {
                    ui.add(egui::Image::from_bytes("bytes://logo.svg", super::LOGO_SVG).fit_to_exact_size(vec2(96.0, 80.0)).tint(tint(Color32::WHITE, 0.35)));
                    ui.add_space(14.0);
                }
                ui.label(RichText::new(tr!("list.empty")).font(sans(13.0, W::Regular)).color(DIM));
            });
            return;
        }
        for id in &list {
            let Some(item) = app.items.iter().find(|i| i.id == *id) else { continue };
            let (outer, _) = ui.allocate_exact_size(vec2(w, 60.0), Sense::hover());
            let row = Rect::from_min_max(pos2(outer.left() + 12.0, outer.top()), pos2(outer.right() - 12.0, outer.bottom() - 2.0));
            let cols = columns(row.left() + 20.0, row.width() - 28.0, cols.stats);
            if let Some(a) = draw_row(ui, row, &cols, item, app.selected == Some(item.id)) {
                actions.push((item.id, a));
            }
        }
        ui.add_space(16.0);
    });
    for (id, a) in actions {
        match a {
            "select" => app.selected = Some(id),
            "folder" => {
                // The icon was "pause" until the download finished: ignore a
                // click aimed at the old button.
                if let Some(item) = app.items.iter().find(|i| i.id == id && i.settled()) {
                    open_folder(item);
                }
            }
            _ => app.toggle(id, &ctx),
        }
    }
}

fn draw_row(ui: &mut Ui, row: Rect, cols: &Cols, item: &Item, selected: bool) -> Option<&'static str> {
    let resp = ui.interact(row, Id::unique(("row", item.id)), Sense::click());
    let p = ui.painter().clone();
    let (bg, border) = if selected {
        (HOVER, TRACK)
    } else if resp.hovered() {
        (INPUT, Color32::TRANSPARENT)
    } else {
        (Color32::TRANSPARENT, Color32::TRANSPARENT)
    };
    p.rect(row, CornerRadius::same(10), bg, Stroke::new(1.0, border), StrokeKind::Inside);
    let cy = row.center().y;

    // Arquivo: extension badge + name + size.
    let (x0, w0) = cols.x[0];
    let badge = Rect::from_min_size(pos2(x0, cy - 17.0), vec2(34.0, 34.0));
    p.rect(badge, CornerRadius::same(8), RAISED, Stroke::new(1.0, TRACK), StrokeKind::Inside);
    text(&p, badge.center(), Align2::CENTER_CENTER, &item.ext(), mono(9.5, W::Medium), MUTED, 32.0);
    let tx = badge.right() + 12.0;
    text(&p, pos2(tx, cy - 8.5), Align2::LEFT_CENTER, &item.name(), sans(13.0, W::Medium), TEXT, x0 + w0 - tx);
    text(&p, pos2(tx, cy + 9.5), Align2::LEFT_CENTER, &item.size_label(), mono(11.0, W::Regular), DIM, x0 + w0 - tx);

    // Partes: bar + percentage.
    let (x1, w1) = cols.x[1];
    parts_bar(&p, Rect::from_min_size(pos2(x1, cy - 10.0), vec2(w1, 6.0)), item, &item.parts(), TRACK, 1);
    let pct = format!("{:.1}%", item.fraction() * 100.0);
    let sub = if cols.stats || item.status != Status::Running {
        format!("{pct} · {}", item.parts_label())
    } else {
        format!("{pct} · {} · {}", item.speed_label(), item.eta_label())
    };
    text(&p, pos2(x1, cy + 8.0), Align2::LEFT_CENTER, &sub, mono(11.0, W::Regular), MUTED, w1);

    let mut c = 2;
    if cols.stats {
        let (x, w) = cols.x[2];
        text(&p, pos2(x + w, cy), Align2::RIGHT_CENTER, &item.speed_label(), mono(12.0, W::Regular), TEXT2, w);
        let (x, w) = cols.x[3];
        text(&p, pos2(x + w, cy), Align2::RIGHT_CENTER, &item.eta_label(), mono(12.0, W::Regular), MUTED, w);
        c = 4;
    }
    let (xs, _) = cols.x[c];
    let pill_rect = pill(&p, pos2(xs, cy), &item.status.label(), item.status.color());
    if let Status::Failed(e) = &item.status {
        ui.interact(pill_rect, Id::unique(("pill", item.id)), Sense::hover()).on_hover_text(e);
    }

    // Action: pause / resume / open folder.
    let (xa, _) = cols.x[c + 1];
    let ar = Rect::from_min_size(pos2(xa, cy - 14.0), vec2(28.0, 28.0));
    let act = ui.interact(ar, Id::unique(("act", item.id)), Sense::click());
    if act.hovered() {
        p.rect_filled(ar, CornerRadius::same(7), TRACK);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fg = if act.hovered() { TEXT } else { MUTED };
    let ir = Rect::from_center_size(ar.center(), vec2(11.0, 11.0));
    match item.status {
        Status::Running => icon_pause(&p, ir, fg),
        Status::Done(_) => icon_folder(&p, Rect::from_center_size(ar.center(), vec2(13.0, 13.0)), fg),
        _ => icon_play(&p, ir, fg),
    }
    let tip = tr!(match item.status {
        Status::Running => "action.pause",
        Status::Done(_) => "action.open_folder",
        Status::Failed(_) => "action.retry",
        Status::Paused => "action.resume",
    });
    if act.on_hover_text(tip).clicked() {
        return Some(if matches!(item.status, Status::Done(_)) { "folder" } else { "toggle" });
    }
    resp.clicked().then_some("select")
}

// ------------------------------------------------------------------- detail

pub fn detail(app: &mut PincersApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    ui.painter().vline(full.left() + 0.5, full.y_range(), Stroke::new(1.0, LINE));
    let Some(item) = app.items.iter().find(|i| Some(i.id) == app.selected) else { return };
    let mut action = None;

    // ---- header
    egui::Panel::top("detail-head")
        .frame(egui::Frame::NONE.inner_margin(Margin { left: 20, right: 20, top: 20, bottom: 16 }))
        .show_separator_line(false)
        .show(ui, |ui| {
            let w = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
            let p = ui.painter();
            pill(p, pos2(r.left(), r.center().y), &item.status.label(), item.status.color());
            text(p, r.right_center(), Align2::RIGHT_CENTER, &when(item.added_ts), sans(11.0, W::Regular), DIM, 160.0);
            ui.add_space(12.0);
            // word-break: break-all
            let mut job = egui::text::LayoutJob::single_section(
                item.name(),
                egui::TextFormat { font_id: sans(17.0, W::Semi), color: TEXT, ..Default::default() },
            );
            job.wrap.max_width = w;
            job.wrap.break_anywhere = true;
            let g = ui.painter().layout_job(job);
            let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
            ui.painter().galley(r.min, g, TEXT);
            ui.add_space(12.0);
            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
            let p = ui.painter();
            p.rect(r, CornerRadius::same(7), BG, Stroke::new(1.0, LINE), StrokeKind::Inside);
            text(p, pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, &item.url, mono(11.0, W::Regular), MUTED, w - 20.0);
            if resp.on_hover_text(tr!("detail.copy_url_tip")).clicked() {
                ctx.copy_text(item.url.clone());
            }
            if let Status::Failed(e) = &item.status {
                ui.add_space(8.0);
                ui.label(RichText::new(e).font(sans(11.5, W::Regular)).color(RED));
            }
        });
    let head_bottom = ui.min_rect().bottom();
    ui.painter().hline(full.x_range(), head_bottom, Stroke::new(1.0, LINE));

    // ---- footer actions
    egui::Panel::bottom("detail-foot")
        .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(20, 14)))
        .show_separator_line(false)
        .show(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
            ui.painter().hline(full.x_range(), r.top() - 14.0, Stroke::new(1.0, LINE));
            let f = sans(12.5, W::Regular);
            let folder_label = tr!("action.open_folder");
            let folder_w = 24.0 + label_width(ui, &folder_label, f.clone());
            let trash = Rect::from_min_size(pos2(r.right() - 34.0, r.top()), vec2(34.0, 34.0));
            let folder = Rect::from_min_size(pos2(trash.left() - 8.0 - folder_w, r.top()), vec2(folder_w, 34.0));
            let primary = Rect::from_min_max(r.min, pos2(folder.left() - 8.0, r.bottom()));
            let label = tr!(match item.status {
                Status::Running => "action.pause",
                Status::Paused => "action.resume",
                Status::Failed(_) => "action.retry",
                Status::Done(_) => "action.open_file",
            });
            if button(ui, primary, "primary", &label, None, Btn::Raised, sans(12.5, W::Medium)).clicked() {
                action = Some("toggle");
            }
            if button(ui, folder, "folder", &folder_label, None, Btn::Outline, f).clicked() {
                action = Some("folder");
            }
            if button(ui, trash, "trash", "", Some(icon_trash), Btn::Danger, sans(12.0, W::Regular))
                .on_hover_text(tr!("action.remove_tip"))
                .clicked()
            {
                action = Some("remove");
            }
        });

    // ---- scrollable body (padding 16 20)
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(Margin::symmetric(20, 16)).show(ui, |ui| {
            let w = ui.available_width();
            stats_grid(ui, w, item);
            ui.add_space(20.0);
            file_map(ui, w, item, app.settings.map_view);
            ui.add_space(20.0);
            speed_history(ui, w, item);
            ui.add_space(20.0);
            parts_table(ui, w, item);
        });
    });

    let id = item.id;
    match action {
        Some("toggle") => app.toggle(id, &ctx),
        Some("folder") => {
            if let Some(item) = app.items.iter().find(|i| i.id == id) {
                open_folder(item);
            }
        }
        Some("remove") => app.remove(id),
        _ => {}
    }
}

/// 2×2 grid with 1 px hairlines (`gap:1px` over a border-coloured
/// background); values wrap like the mock-up's "5.80 GB / 5.80 GB".
fn stats_grid(ui: &mut Ui, w: f32, item: &Item) {
    let cells = [
        (tr!("stat.downloaded"), item.size_label_full()),
        (tr!("stat.progress"), format!("{:.1}%", item.fraction() * 100.0)),
        (tr!("stat.speed"), item.speed_label()),
        (tr!("stat.remaining"), item.eta_label()),
    ];
    let cw = (w - 3.0) / 2.0;
    let galleys: Vec<_> = cells
        .iter()
        .map(|(_, v)| {
            let mut job = egui::text::LayoutJob::single_section(
                v.clone(),
                egui::TextFormat { font_id: mono(13.0, W::Regular), color: TEXT, ..Default::default() },
            );
            job.wrap.max_width = cw - 24.0;
            job.wrap.max_rows = 2;
            ui.painter().layout_job(job)
        })
        .collect();
    let row_h = |a: usize| 10.0 + 13.0 + 4.0 + galleys[a].size().y.max(galleys[a + 1].size().y) + 10.0;
    let (h0, h1) = (row_h(0), row_h(2));
    let (r, _) = ui.allocate_exact_size(vec2(w, h0 + h1 + 3.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(10), LINE);
    for (i, ((label, _), g)) in cells.iter().zip(galleys).enumerate() {
        let (col, row) = (i % 2, i / 2);
        let cell = Rect::from_min_size(
            pos2(r.left() + 1.0 + col as f32 * (cw + 1.0), r.top() + 1.0 + row as f32 * (h0 + 1.0)),
            vec2(cw, if row == 0 { h0 } else { h1 }),
        );
        let radius = match i {
            0 => CornerRadius { nw: 9, ..Default::default() },
            1 => CornerRadius { ne: 9, ..Default::default() },
            2 => CornerRadius { sw: 9, ..Default::default() },
            _ => CornerRadius { se: 9, ..Default::default() },
        };
        p.rect_filled(cell, radius, PANEL);
        text(p, pos2(cell.left() + 12.0, cell.top() + 10.0 + 6.5), Align2::LEFT_CENTER, label, sans(11.0, W::Regular), DIM, cw - 24.0);
        p.galley(pos2(cell.left() + 12.0, cell.top() + 10.0 + 13.0 + 4.0), g, TEXT);
    }
}

impl Item {
    fn size_label_full(&self) -> String {
        let total = self.last.size.map(|s| size(s as f64)).unwrap_or_else(|| "?".into());
        let done = if matches!(self.status, Status::Done(_)) { self.last.size.unwrap_or(self.last.downloaded) } else { self.last.downloaded };
        format!("{} / {total}", size(done as f64))
    }
}

fn file_map(ui: &mut Ui, w: f32, item: &Item, view: MapView) {
    let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
    let p = ui.painter();
    text(p, r.left_center(), Align2::LEFT_CENTER, &tr!("map.title"), sans(12.0, W::Medium), TEXT, w / 2.0);
    text(p, r.right_center(), Align2::RIGHT_CENTER, &item.parts_label(), sans(11.0, W::Regular), DIM, w / 2.0);
    ui.add_space(10.0);
    let parts = item.parts();
    match view {
        MapView::Barras => {
            let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
            parts_bar(ui.painter(), r, item, &parts, RAISED, 3);
        }
        MapView::Grade => {
            let (cols, rows, gap) = (24usize, 5usize, 2.0);
            let cell = (w - gap * (cols as f32 - 1.0)) / cols as f32;
            let (r, _) = ui.allocate_exact_size(vec2(w, rows as f32 * cell + (rows as f32 - 1.0) * gap), Sense::hover());
            let size = parts.iter().map(|s| s.end).max().unwrap_or(1).max(1) as f64;
            let n = (cols * rows) as f64;
            let done_all = matches!(item.status, Status::Done(_));
            for i in 0..cols * rows {
                let pos = (i as f64 + 0.5) / n * size;
                let s = parts.iter().find(|s| pos >= s.start as f64 && pos < s.end as f64).or(parts.last());
                let color = match s {
                    _ if done_all => GREEN,
                    Some(s) if pos < s.pos as f64 => item.part_color(s),
                    Some(s) if s.failed && pos < s.pos as f64 + size / 60.0 => RED,
                    Some(s) if item.status == Status::Running && s.active && pos < s.pos as f64 + size / n => tint(ACCENT, 0.4),
                    _ => LINE,
                };
                let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
                let cr = Rect::from_min_size(pos2(r.left() + cx * (cell + gap), r.top() + cy * (cell + gap)), vec2(cell, cell));
                ui.painter().rect_filled(cr, CornerRadius::same(2), color);
            }
        }
    }
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
    let p = ui.painter();
    let mut x = r.left();
    for (c, l) in [(ACCENT, tr!("legend.downloading")), (GREEN, tr!("legend.done")), (TRACK, tr!("legend.pending"))] {
        p.rect_filled(Rect::from_min_size(pos2(x, r.center().y - 4.0), vec2(8.0, 8.0)), CornerRadius::same(2), c);
        let t = text(p, pos2(x + 14.0, r.center().y), Align2::LEFT_CENTER, &l, sans(11.0, W::Regular), MUTED, 100.0);
        x = t.right() + 14.0;
    }
}

fn speed_history(ui: &mut Ui, w: f32, item: &Item) {
    let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
    text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!("chart.speed"), sans(12.0, W::Medium), TEXT, w);
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 45.0), Sense::hover());
    let chart = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - 1.0));
    bars(ui.painter(), chart, item.hist.iter().copied().collect(), 48, 0.02, tint(ACCENT, 0.8));
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, LINE));
}

fn parts_table(ui: &mut Ui, w: f32, item: &Item) {
    // grid-template-columns: 22px minmax(0,1fr) 70px 64px; gap 10px
    let gap = 10.0;
    let col = |x0: f32| {
        let mid = w - 22.0 - 70.0 - 64.0 - 3.0 * gap;
        [x0, x0 + 22.0 + gap, x0 + 22.0 + gap + mid + gap, x0 + 22.0 + gap + mid + gap + 70.0 + gap, mid]
    };
    let (h, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    let c = col(h.left());
    let p = ui.painter();
    for (x, label, right) in [
        (c[0], tr!("parts.col_n"), false),
        (c[1], tr!("parts.col_range"), false),
        (c[2] + 70.0, tr!("parts.col_downloaded"), true),
        (c[3], tr!("parts.col_state"), false),
    ] {
        let g = line(p, &label.to_uppercase(), sans(10.5, W::Regular), DIM, f32::INFINITY, 0.63);
        let gx = if right { x - g.size().x } else { x };
        p.galley(pos2(gx, h.top()), g, DIM);
    }
    for (n, s) in item.parts().iter().enumerate() {
        let (r, _) = ui.allocate_exact_size(vec2(w, 7.0 + 13.0 + 5.0 + 3.0 + 7.0 + 2.0), Sense::hover());
        let p = ui.painter();
        p.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, LINE_SOFT));
        let c = col(r.left());
        let cy = r.top() + 7.0 + 6.5;
        let color = item.part_color(s);
        text(p, pos2(c[0], cy), Align2::LEFT_CENTER, &(n + 1).to_string(), mono(11.0, W::Regular), DIM, 22.0);
        let range = format!("{} – {}", size(s.start as f64), size(s.end as f64));
        text(p, pos2(c[1], cy), Align2::LEFT_CENTER, &range, mono(11.0, W::Regular), MUTED, c[4]);
        let track = Rect::from_min_size(pos2(c[1], cy + 6.5 + 5.0), vec2(c[4], 3.0));
        p.rect_filled(track, CornerRadius::same(2), TRACK);
        let len = s.end.saturating_sub(s.start).max(1);
        let got = s.pos.saturating_sub(s.start);
        let f = if matches!(item.status, Status::Done(_)) { 1.0 } else { (got as f32 / len as f32).clamp(0.0, 1.0) };
        p.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * f, 3.0)), CornerRadius::same(2), color);
        let got = if matches!(item.status, Status::Done(_)) { len } else { got };
        text(p, pos2(c[2] + 70.0, r.center().y), Align2::RIGHT_CENTER, &size(got as f64), mono(11.0, W::Regular), TEXT2, 70.0);
        let state = tr!(if s.failed {
            "part.error"
        } else if s.pos >= s.end || matches!(item.status, Status::Done(_)) {
            "part.done"
        } else {
            match item.status {
                Status::Running if s.active => "part.active",
                Status::Running => "part.waiting",
                Status::Paused => "part.paused",
                _ => "part.stopped",
            }
        });
        text(p, pos2(c[3], r.center().y), Align2::LEFT_CENTER, &state, sans(11.0, W::Regular), color, 64.0);
    }
}

// ------------------------------------------------------------------ options

pub fn settings_window(app: &mut PincersApp, ctx: &egui::Context) {
    let mut open = app.show_settings;
    let mut changed = false;
    egui::Window::new(RichText::new(tr!("options.title")).font(sans(14.0, W::Semi)))
        .id(Id::unique("options-window"))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(PANEL)
                .stroke(Stroke::new(1.0, TRACK))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 12.0);
            let label = |key: &str| RichText::new(tr!(key)).font(sans(12.5, W::Regular)).color(MUTED);
            egui::Grid::new("opts").num_columns(2).spacing([16.0, 14.0]).show(ui, |ui| {
                ui.label(label("options.language"));
                changed |= language_picker(app, ui);
                ui.end_row();

                ui.label(label("options.connections"));
                changed |= ui
                    .add(egui::Slider::new(&mut app.settings.connections, 1..=32))
                    .on_hover_text(tr!("options.connections_tip"))
                    .changed();
                ui.end_row();

                ui.label(label("options.save_to"));
                ui.horizontal(|ui| {
                    let mut dir = app.settings.out_dir.display().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut dir).font(mono(11.5, W::Regular)).desired_width(240.0)).changed() {
                        app.settings.out_dir = dir.into();
                        changed = true;
                    }
                    if ui.button(tr!("options.browse")).clicked()
                        && let Some(d) = rfd::FileDialog::new().set_directory(&app.settings.out_dir).pick_folder()
                    {
                        app.settings.out_dir = d;
                        changed = true;
                    }
                });
                ui.end_row();

                changed |= ui.checkbox(&mut app.settings.limit_enabled, label("options.limit")).changed();
                changed |= ui
                    .add_enabled(
                        app.settings.limit_enabled,
                        egui::DragValue::new(&mut app.settings.limit_kbps).range(1..=10_000_000).suffix(" KB/s"),
                    )
                    .changed();
                ui.end_row();

                ui.label(label("options.map"));
                ui.horizontal(|ui| {
                    changed |= ui.selectable_value(&mut app.settings.map_view, MapView::Grade, tr!("options.map_grid")).changed();
                    changed |= ui.selectable_value(&mut app.settings.map_view, MapView::Barras, tr!("options.map_bars")).changed();
                });
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr!("options.note"))
                    .font(sans(11.0, W::Regular))
                    .color(DIM),
            );
        });
    if changed || open != app.show_settings {
        app.mark_dirty();
    }
    app.show_settings = open;
}

/// Dropdown with "Automatic (system language)" plus every loaded language
/// (built-in or dropped into a `locales` folder). Applies at once.
fn language_picker(app: &mut PincersApp, ui: &mut Ui) -> bool {
    let languages = crate::i18n::languages();
    let name_of = |code: &str| languages.iter().find(|l| l.code == code).map(|l| l.name.clone()).unwrap_or_else(|| code.to_owned());
    let auto = tr!("options.language_auto", language = name_of(&crate::i18n::system_choice()));
    let selected_text = match &app.settings.language {
        None => auto.clone(),
        Some(code) => name_of(code),
    };
    let mut choice = app.settings.language.clone();
    egui::ComboBox::from_id_salt("language")
        .selected_text(selected_text)
        .width(240.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut choice, None, auto);
            for l in &languages {
                ui.selectable_value(&mut choice, Some(l.code.clone()), l.name.clone());
            }
        });
    if choice == app.settings.language {
        return false;
    }
    app.settings.language = choice;
    crate::i18n::set_language(app.settings.language.as_deref());
    true
}
