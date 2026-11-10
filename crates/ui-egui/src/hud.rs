//! KHGIS's heads-up display over the map: glowing corner brackets, a centre reticle, a live
//! readout of the pointer's position and the map's scale and heading, and an animated fly-to
//! whenever the camera jumps (Zoom To Layer, a bookmark, a search result, Full Extent): the view
//! glides there, pulling back for long hops, instead of cutting.
//!
//! User motion (dragging, the wheel, the keyboard over the map) is never animated: only jumps
//! the map didn't make itself.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Ui, pos2, vec2};
use rustgis_engine::map::Camera;

use crate::RustGisApp;
use crate::theme::{Tokens, regular, semibold};

/// How long a fly-to takes, in seconds.
const FLY_SECONDS: f64 = 1.1;

#[derive(Clone, Debug)]
struct Fly {
    from: Camera,
    to: Camera,
    /// 0 to 1.
    t: f64,
    /// How far the view pulls back halfway, as a multiple of the larger resolution.
    bump: f64,
}

#[derive(Clone, Debug)]
pub struct HudState {
    /// The HUD is drawn (View › HUD, `ui.hud`).
    pub enabled: bool,
    /// Camera jumps glide (`ui.hud {"fly": false}` turns it off).
    pub fly_enabled: bool,
    last: Option<(u64, usize, Camera)>,
    fly: Option<Fly>,
    /// The map's rectangle last frame: where the pointer counts as moving the map.
    map_rect: Option<Rect>,
    /// The map moved under the user's hand recently (pointer or keys): not a jump.
    user_until: f64,
}

impl Default for HudState {
    fn default() -> Self {
        HudState { enabled: true, fly_enabled: true, last: None, fly: None, map_rect: None, user_until: 0.0 }
    }
}

impl HudState {
    pub fn flying(&self) -> bool {
        self.fly.is_some()
    }
}

/// Whether the user is moving the map with the pointer or the keyboard this frame.
fn user_moving(ui: &Ui, map_rect: Option<Rect>) -> bool {
    // Keys typed into a text box move its cursor, not the map.
    let typing = ui.ctx().egui_wants_keyboard_input();
    ui.input(|i| {
        let over = map_rect.zip(i.pointer.hover_pos()).is_some_and(|(r, p)| r.contains(p));
        let pointer = over && (i.pointer.any_down() || i.smooth_scroll_delta() != egui::Vec2::ZERO || i.zoom_delta() != 1.0);
        let keys = [
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::Plus,
            egui::Key::Minus,
            egui::Key::Equals,
            egui::Key::PageUp,
            egui::Key::PageDown,
        ]
        .iter()
        .any(|k| i.key_down(*k));
        pointer || (keys && !typing)
    })
}

fn smooth(t: f64) -> f64 {
    // Ease in and out (smootherstep).
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, s: f64) -> f64 {
    a + (b - a) * s
}

/// The shortest turn from `a` to `b`, in degrees.
fn lerp_angle(a: f64, b: f64, s: f64) -> f64 {
    let d = ((b - a + 540.0) % 360.0) - 180.0;
    a + d * s
}

fn camera_at(f: &Fly, s: f64) -> Camera {
    let e = smooth(s);
    let (r0, r1) = (f.from.res.max(1e-9).ln(), f.to.res.max(1e-9).ln());
    // A pull-back that peaks halfway: long hops rise, look, and come down.
    let rise = 1.0 + f.bump * 4.0 * e * (1.0 - e);
    Camera {
        center: rustgis_engine::geom::Coord::new(lerp(f.from.center.x, f.to.center.x, e), lerp(f.from.center.y, f.to.center.y, e)),
        res: lerp(r0, r1, e).exp() * rise,
        bearing: lerp_angle(f.from.bearing, f.to.bearing, e),
        pitch: lerp(f.from.pitch, f.to.pitch, e),
    }
}

/// Whether two cameras differ by a jump: the centre moved more than half the view, or the scale
/// changed more than 2.5 times.
fn is_jump(a: &Camera, b: &Camera, view: Rect) -> bool {
    let span = view.width().max(view.height()) as f64 * a.res.max(b.res);
    let moved = ((a.center.x - b.center.x).powi(2) + (a.center.y - b.center.y).powi(2)).sqrt();
    let ratio = (a.res / b.res.max(1e-12)).max(b.res / a.res.max(1e-12));
    (span > 0.0 && moved > span * 0.5) || ratio > 2.5
}

/// Runs once a frame before the map is drawn: turns camera jumps into a glide and advances one.
pub fn fly(app: &mut RustGisApp, ui: &mut Ui) {
    let now = ui.input(|i| i.time);
    let map_index = app.session.project.active_map;
    let opened = app.session.opened;
    let cam = app.session.map().camera;
    let moving = user_moving(ui, app.ws.hud.map_rect);
    if moving {
        app.ws.hud.user_until = now + 0.35;
        // A hand on the map stops a glide where it is.
        app.ws.hud.fly = None;
    }
    // Another project or map: nothing to glide from.
    let same = app.ws.hud.last.as_ref().is_some_and(|(o, m, _)| *o == opened && *m == map_index);
    if let Some(f) = app.ws.hud.fly.clone() {
        if cam != camera_at(&f, f.t) && cam != f.to {
            // Something else moved the camera mid-glide: aim at where it went.
            let from = camera_at(&f, f.t);
            app.ws.hud.fly = Some(Fly { from, to: cam, t: 0.0, bump: 0.0 });
        }
    } else if app.ws.hud.fly_enabled
        && same
        && now > app.ws.hud.user_until
        && app.ws.playing.is_none()
        && let (Some((_, _, before)), Some(view)) = (app.ws.hud.last.clone(), app.ws.hud.map_rect)
        && is_jump(&before, &cam, view)
    {
        let span = view.width().max(view.height()) as f64 * before.res.max(cam.res);
        let moved = ((before.center.x - cam.center.x).powi(2) + (before.center.y - cam.center.y).powi(2)).sqrt();
        let bump = (moved / span.max(1e-9) - 0.5).clamp(0.0, 2.5);
        app.ws.hud.fly = Some(Fly { from: before, to: cam, t: 0.0, bump });
    }
    if let Some(mut f) = app.ws.hud.fly.take() {
        let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
        f.t = (f.t + dt / FLY_SECONDS).min(1.0);
        let c = if f.t >= 1.0 { f.to } else { camera_at(&f, f.t) };
        app.session.map_mut().camera = c;
        if f.t < 1.0 {
            app.ws.hud.fly = Some(f);
        }
        ui.ctx().request_repaint();
    }
    app.ws.hud.last = Some((opened, map_index, app.session.map().camera));
}

/// Remembers the map's rectangle, for telling the user's motion from a jump.
pub fn note_rect(app: &mut RustGisApp, rect: Rect) {
    app.ws.hud.map_rect = Some(rect);
}

fn glow_line(p: &Painter, a: Pos2, b: Pos2, c: Color32, width: f32) {
    p.line_segment([a, b], Stroke::new(width + 4.0, c.gamma_multiply(0.18)));
    p.line_segment([a, b], Stroke::new(width, c));
}

/// Paints the HUD over the map.
pub fn paint(app: &RustGisApp, p: &Painter, rect: Rect, t: &Tokens) {
    if !app.ws.hud.enabled || rect.width() < 240.0 || rect.height() < 180.0 {
        return;
    }
    let c = t.accent;
    let inset = 10.0;
    let arm = 26.0;
    // Corner brackets.
    for (corner, dx, dy) in [
        (rect.left_top(), 1.0, 1.0),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        let o = corner + vec2(inset * dx, inset * dy);
        glow_line(p, o, o + vec2(arm * dx, 0.0), c, 1.6);
        glow_line(p, o, o + vec2(0.0, arm * dy), c, 1.6);
    }
    // Centre reticle: a broken ring and four ticks.
    let m = rect.center();
    let ring = c.gamma_multiply(0.75);
    for k in 0..4 {
        let a0 = (k as f32 * 90.0 + 12.0).to_radians();
        let pts: Vec<Pos2> = (0..=10)
            .map(|i| {
                let a = a0 + (66.0f32).to_radians() * i as f32 / 10.0;
                m + vec2(a.cos(), a.sin()) * 14.0
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.2, ring)));
        let a = (k as f32 * 90.0).to_radians();
        let d = vec2(a.cos(), a.sin());
        p.line_segment([m + d * 18.0, m + d * 26.0], Stroke::new(1.2, ring));
    }
    p.circle_filled(m, 1.6, c);

    // The readout: pointer position, scale and heading.
    let map = app.session.map();
    let scale = map.scale();
    let mut parts = Vec::new();
    if let Some(ll) = app.map.pointer_lonlat {
        let ns = if ll.y >= 0.0 { "N" } else { "S" };
        let ew = if ll.x >= 0.0 { "E" } else { "W" };
        parts.push(format!("{:.5}° {ns}  {:.5}° {ew}", ll.y.abs(), ll.x.abs()));
    }
    if scale.is_finite() && scale > 0.0 {
        parts.push(format!("1:{}", group_thousands(scale.round() as u64)));
    }
    parts.push(format!("HDG {:03.0}°", ((map.camera.bearing % 360.0) + 360.0) % 360.0));
    if app.ws.hud.flying() {
        parts.push(crate::i18n::t("FLYING").to_string());
    }
    let text = parts.join("   ·   ");
    let g = p.layout_no_wrap(text, egui::FontId::monospace(11.5), t.accent_text);
    let box_ = Rect::from_center_size(pos2(m.x, rect.min.y + 22.0), g.size() + vec2(28.0, 10.0));
    p.rect_filled(box_, egui::CornerRadius::same(4), Color32::from_rgba_unmultiplied(4, 10, 22, 190));
    p.rect_stroke(box_, egui::CornerRadius::same(4), Stroke::new(1.0, c.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    // Small notches on the box's ends, like an instrument's.
    for x in [box_.min.x, box_.max.x] {
        p.line_segment([pos2(x, box_.center().y - 4.0), pos2(x, box_.center().y + 4.0)], Stroke::new(2.0, c));
    }
    p.galley(box_.center() - g.size() / 2.0, g, t.accent_text);
    // The product mark, faint, bottom centre.
    let mark = p.layout_no_wrap(crate::APP_NAME.to_string(), semibold(10.0), c.gamma_multiply(0.55));
    p.galley(pos2(m.x - mark.size().x / 2.0, rect.max.y - inset - mark.size().y - 2.0), mark, c);
    let _ = regular;
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustgis_engine::geom::Coord;

    fn cam(x: f64, res: f64) -> Camera {
        Camera { center: Coord::new(x, 0.0), res, bearing: 0.0, pitch: 0.0 }
    }

    #[test]
    fn jumps_are_told_from_small_moves() {
        let view = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0));
        assert!(!is_jump(&cam(0.0, 100.0), &cam(1000.0, 100.0), view), "a nudge");
        assert!(!is_jump(&cam(0.0, 100.0), &cam(0.0, 80.0), view), "a wheel step");
        assert!(is_jump(&cam(0.0, 100.0), &cam(0.0, 10.0), view), "zoom to a layer");
        assert!(is_jump(&cam(0.0, 100.0), &cam(80_000.0, 100.0), view), "a bookmark across the map");
    }

    #[test]
    fn a_glide_starts_and_ends_on_its_cameras_and_pulls_back_between() {
        let f = Fly { from: cam(0.0, 100.0), to: cam(1e6, 100.0), t: 0.0, bump: 1.5 };
        assert_eq!(camera_at(&f, 0.0).center.x, 0.0);
        assert!((camera_at(&f, 1.0).center.x - 1e6).abs() < 1e-6);
        assert!(camera_at(&f, 0.5).res > 200.0, "pulled back halfway");
        assert!((camera_at(&f, 1.0).res - 100.0).abs() < 1e-6);
    }

    #[test]
    fn headings_turn_the_short_way() {
        assert!((lerp_angle(350.0, 10.0, 0.5) - 360.0).abs() < 1e-9);
        assert_eq!(group_thousands(1234567), "1,234,567");
    }
}
