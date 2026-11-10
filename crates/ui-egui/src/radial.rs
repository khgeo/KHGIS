//! KHGIS's radial (pie) menu on the map: right-click opens eight actions round the pointer.
//! Flick: press the right button, move towards an action and let go, and it runs without the
//! menu ever needing a second click. A plain right-click leaves the menu open for a click.
//! Shift + right-click, or the "More…" slice, opens the full classic menu.

use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};
use rustgis_engine::geom::Coord;
use serde_json::json;

use crate::RustGisApp;
use crate::theme::{Tokens, semibold};

/// The ring's inner and outer radii, in points.
const INNER: f32 = 34.0;
const OUTER: f32 = 122.0;
/// How far a flick must travel before letting go picks a slice.
const FLICK: f32 = 26.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    ZoomIn,
    Center,
    Buffer,
    Copy,
    ZoomOut,
    Directions,
    Measure,
    More,
}

const ACTIONS: [(Action, &str, &str); 8] = [
    (Action::ZoomIn, "zoom_in", "Zoom In Here"),
    (Action::Center, "goto_xy", "Center Here"),
    (Action::Buffer, "buffer", "Buffer 1 km"),
    (Action::Copy, "copy", "Copy Coordinates"),
    (Action::ZoomOut, "zoom_out", "Zoom Out Here"),
    (Action::Directions, "directions", "Directions To Here"),
    (Action::Measure, "measure", "Measure"),
    (Action::More, "more", "More…"),
];

#[derive(Clone, Debug, Default)]
pub struct RadialState {
    /// The radial menu is used for right-clicks (View › Radial Menu, `ui.radialMenu`).
    pub disabled: bool,
    /// Open at this screen point, for this place; `held` while the right button is still down.
    open: Option<(Pos2, Coord, bool)>,
    /// The classic menu, opened from "More…", at this point for this place.
    classic: Option<(Pos2, Coord)>,
}

impl RadialState {
    pub fn is_open(&self) -> bool {
        self.open.is_some() || self.classic.is_some()
    }
}

/// The slice under `p`, if it is in the ring (or past it, when flicking).
fn slice_at(center: Pos2, p: Pos2, flicking: bool) -> Option<usize> {
    let d = p - center;
    let r = d.length();
    if r < if flicking { FLICK } else { INNER } || (!flicking && r > OUTER + 40.0) {
        return None;
    }
    // Slice 0 is straight up; they go clockwise, 45° each.
    let a = d.x.atan2(-d.y).to_degrees();
    let a = (a + 360.0 + 22.5) % 360.0;
    Some((a / 45.0) as usize % 8)
}

/// Starts the menu on a right-press over the map; returns whether it took the press (the classic
/// context menu then stays shut).
pub fn press(app: &mut RustGisApp, ui: &Ui, rect: Rect) -> bool {
    if app.ws.radial.disabled {
        return false;
    }
    let (pressed, pos, shift) = ui.input(|i| (i.pointer.button_pressed(egui::PointerButton::Secondary), i.pointer.press_origin(), i.modifiers.shift));
    if !pressed || shift {
        return false;
    }
    let (Some(pos), Some(c)) = (pos.filter(|p| rect.contains(*p)), app.map.pointer_lonlat) else { return false };
    app.ws.radial.open = Some((pos, c, true));
    app.ws.radial.classic = None;
    true
}

/// Escape closes the menu; checked early in the frame, before other handlers take the key.
pub fn escape(app: &mut RustGisApp, ui: &Ui) {
    if app.ws.radial.is_open() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ws.radial.open = None;
        app.ws.radial.classic = None;
    }
}

/// Draws the open menu and runs what is picked.
pub fn show(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens) {
    if let Some((at, c)) = app.ws.radial.classic {
        classic(app, ui, at, c);
    }
    let Some((center, c, held)) = app.ws.radial.open else { return };
    let (hover, secondary_down, released, primary_clicked, escape) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.button_down(egui::PointerButton::Secondary),
            i.pointer.button_released(egui::PointerButton::Secondary),
            i.pointer.button_clicked(egui::PointerButton::Primary),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if escape {
        app.ws.radial.open = None;
        return;
    }
    let flicking = held && secondary_down;
    let picked = hover.and_then(|p| slice_at(center, p, flicking || held));
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("khgis_radial")));
    paint(&p, t, center, picked);
    let mut chosen = None;
    if held && released {
        // A flick picks the slice it went towards; a plain right-click leaves the menu open.
        let moved = hover.map(|h| (h - center).length()).unwrap_or(0.0);
        if moved >= FLICK {
            chosen = picked;
        }
        if let Some(open) = &mut app.ws.radial.open {
            open.2 = false;
        }
        if chosen.is_none() && moved >= FLICK {
            app.ws.radial.open = None;
        }
    } else if !held && primary_clicked {
        chosen = picked;
        if chosen.is_none() {
            app.ws.radial.open = None;
        }
    }
    if let Some(i) = chosen {
        app.ws.radial.open = None;
        act(app, ui, ACTIONS[i].0, center, c);
    }
    ui.ctx().request_repaint();
}

fn paint(p: &egui::Painter, t: &Tokens, center: Pos2, picked: Option<usize>) {
    let bg = Color32::from_rgba_unmultiplied(6, 12, 26, 228);
    let accent = t.accent;
    // The ring, slice by slice.
    for (i, (_, icon, label)) in ACTIONS.iter().enumerate() {
        let a0 = (i as f32 * 45.0 - 22.5 - 90.0).to_radians();
        let a1 = a0 + 45f32.to_radians();
        let mut pts = Vec::new();
        let n = 10;
        for k in 0..=n {
            let a = a0 + (a1 - a0) * k as f32 / n as f32;
            pts.push(center + vec2(a.cos(), a.sin()) * OUTER);
        }
        for k in (0..=n).rev() {
            let a = a0 + (a1 - a0) * k as f32 / n as f32;
            pts.push(center + vec2(a.cos(), a.sin()) * INNER);
        }
        let on = picked == Some(i);
        let fill = if on { accent.gamma_multiply(0.42) } else { bg };
        p.add(egui::Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
        // Slice edges.
        p.line_segment([center + vec2(a0.cos(), a0.sin()) * INNER, center + vec2(a0.cos(), a0.sin()) * OUTER], Stroke::new(1.0, accent.gamma_multiply(0.35)));
        // The icon and a short label at the slice's middle.
        let mid = (i as f32 * 45.0 - 90.0).to_radians();
        let at = center + vec2(mid.cos(), mid.sin()) * ((INNER + OUTER) / 2.0);
        let ink = if on { Color32::WHITE } else { t.accent_text };
        let r = Rect::from_center_size(at - vec2(0.0, 7.0), vec2(20.0, 20.0));
        if *icon == "more" {
            for dx in [-5.0, 0.0, 5.0] {
                p.circle_filled(r.center() + vec2(dx, 0.0), 1.8, ink);
            }
        } else {
            crate::icons::paint(p, r, icon, ink, accent);
        }
        let g = p.layout(crate::i18n::t(label).to_string(), crate::theme::regular(9.5), ink, 76.0);
        p.galley(pos2(at.x - g.size().x / 2.0, at.y + 5.0), g, ink);
    }
    // Rims and the hub.
    p.circle_stroke(center, OUTER, Stroke::new(1.6, accent));
    p.circle_stroke(center, OUTER + 4.0, Stroke::new(4.0, accent.gamma_multiply(0.15)));
    p.circle_stroke(center, INNER, Stroke::new(1.2, accent.gamma_multiply(0.7)));
    p.circle_filled(center, INNER - 6.0, bg);
    let g = p.layout_no_wrap(crate::APP_NAME.to_string(), semibold(9.0), accent);
    p.galley(center - g.size() / 2.0, g, accent);
}

fn run(app: &mut RustGisApp, id: &str, params: serde_json::Value) {
    if let Err(e) = app.run(id, params) {
        app.session.notify(e, true);
    }
}

fn act(app: &mut RustGisApp, ui: &Ui, a: Action, at: Pos2, c: Coord) {
    let lon = ((c.x + 180.0).rem_euclid(360.0)) - 180.0;
    let lat = c.y;
    match a {
        Action::ZoomIn | Action::ZoomOut => {
            let f = if a == Action::ZoomIn { 0.5 } else { 2.0 };
            let scale = app.session.map().scale() * f;
            run(app, "map.goToXY", json!({"lon": lon, "lat": lat, "scale": scale}));
        }
        Action::Center => run(app, "map.goToXY", json!({"lon": lon, "lat": lat})),
        Action::Buffer => {
            let name = crate::i18n::fmt("Buffer {distance}", &[("distance", "1 km")]);
            run(app, "gp.bufferPoint", json!({"lon": lon, "lat": lat, "distance": 1.0, "units": "kilometers", "name": name}));
        }
        Action::Copy => {
            let text = format!("{lat:.6}, {lon:.6}");
            ui.ctx().copy_text(text.clone());
            app.session.notify(crate::i18n::fmt("Copied {text}", &[("text", &text)]), false);
        }
        Action::Directions => crate::navui::seed(app, None, Coord::new(lon, lat)),
        Action::Measure => run(app, "tool.set", json!({"tool": "measure"})),
        Action::More => app.ws.radial.classic = Some((at, c)),
    }
}

/// The classic right-click menu, opened from "More…".
fn classic(app: &mut RustGisApp, ui: &mut Ui, at: Pos2, c: Coord) {
    let resp = egui::Area::new(egui::Id::unique("khgis_classic_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| egui::Frame::menu(ui.style()).show(ui, |ui| crate::mapmenu::show(app, ui, c)));
    let _ = resp;
    // A click anywhere (on an entry, which has then run, or off the menu) closes it.
    if ui.input(|i| i.pointer.any_click() || i.key_pressed(egui::Key::Escape)) {
        app.ws.radial.classic = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_go_clockwise_from_the_top() {
        let c = pos2(100.0, 100.0);
        assert_eq!(slice_at(c, pos2(100.0, 30.0), false), Some(0), "up");
        assert_eq!(slice_at(c, pos2(170.0, 100.0), false), Some(2), "right");
        assert_eq!(slice_at(c, pos2(100.0, 170.0), false), Some(4), "down");
        assert_eq!(slice_at(c, pos2(30.0, 100.0), false), Some(6), "left");
        assert_eq!(slice_at(c, pos2(105.0, 100.0), false), None, "the hub");
        // A flick counts from a short distance out, and as far as the pointer goes.
        assert_eq!(slice_at(c, pos2(100.0, 70.0), true), Some(0));
        assert_eq!(slice_at(c, pos2(400.0, 100.0), true), Some(2));
    }
}
