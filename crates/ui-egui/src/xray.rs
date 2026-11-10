//! KHGIS's X-ray lens: hold X over the map (or turn it on with `ui.xray`) and a circle under the
//! pointer hides the layer on top, showing what lies beneath (satellite imagery under land
//! cover, last year's layer under this year's). `[` and `]` resize it. The renderer masks the
//! layer (`Map::xray`), so it works for vector and raster layers alike.

use egui::{Color32, Painter, Rect, Stroke, Ui, vec2};

use crate::RustGisApp;
use crate::theme::{Tokens, semibold};

#[derive(Clone, Debug)]
pub struct XrayState {
    /// On without holding X.
    pub sticky: bool,
    /// The lens's radius, in points.
    pub radius: f32,
}

impl Default for XrayState {
    fn default() -> Self {
        XrayState { sticky: false, radius: 110.0 }
    }
}

/// The layer the lens sees through: the active layer when it shows, else the top layer shown.
fn target(app: &RustGisApp) -> Option<(u32, String)> {
    let map = app.session.map();
    let scale = map.scale();
    let shown = |l: &&rustgis_engine::map::Layer| l.visible && l.in_scale_range(scale);
    app.session
        .active_layer
        .and_then(|id| map.layers.iter().find(|l| l.id == id))
        .filter(shown)
        .or_else(|| map.layers.iter().find(shown))
        .map(|l| (l.id, l.name.clone()))
}

/// Updates the lens for this frame: on while X is held (or sticky) and the pointer is on the map.
pub fn update(app: &mut RustGisApp, ui: &Ui, rect: Rect) {
    let typing = ui.ctx().egui_wants_keyboard_input();
    let (held, hover, grow, shrink) = ui.input(|i| {
        (i.key_down(egui::Key::X) && !i.modifiers.any(), i.pointer.hover_pos(), i.key_pressed(egui::Key::CloseBracket), i.key_pressed(egui::Key::OpenBracket))
    });
    let on = (app.ws.xray.sticky || (held && !typing)) && rect.width() > 10.0;
    if on && !typing {
        if grow {
            app.ws.xray.radius = (app.ws.xray.radius * 1.2).min(600.0);
        }
        if shrink {
            app.ws.xray.radius = (app.ws.xray.radius / 1.2).max(30.0);
        }
    }
    let want = match (on, hover.filter(|p| rect.contains(*p)), target(app)) {
        (true, Some(p), Some((id, _))) => {
            let cx = ((p.x - rect.min.x) / rect.width()) as f64;
            let cy = ((p.y - rect.min.y) / rect.height()) as f64;
            let r = (app.ws.xray.radius / rect.width()) as f64;
            Some((id, [cx, cy, r]))
        }
        _ => None,
    };
    if app.session.map().xray != want {
        app.session.map_mut().xray = want;
        app.map.invalidate();
    }
}

/// The lens's rim and the name of the layer it sees through.
pub fn paint(app: &RustGisApp, p: &Painter, rect: Rect, t: &Tokens) {
    let Some((id, [cx, cy, r])) = app.session.map().xray else { return };
    let c = rect.min + vec2(cx as f32 * rect.width(), cy as f32 * rect.height());
    let r = r as f32 * rect.width();
    p.circle_stroke(c, r + 3.0, Stroke::new(6.0, t.accent.gamma_multiply(0.18)));
    p.circle_stroke(c, r, Stroke::new(1.8, t.accent));
    // Tick marks round the rim, like a scope.
    for k in 0..24 {
        let a = (k as f32 * 15.0).to_radians();
        let d = vec2(a.cos(), a.sin());
        let len = if k % 6 == 0 { 9.0 } else { 4.0 };
        p.line_segment([c + d * r, c + d * (r - len)], Stroke::new(1.0, t.accent.gamma_multiply(0.8)));
    }
    let name = app.session.map().layers.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default();
    let text = crate::i18n::fmt("X-ray: {layer}", &[("layer", &name)]);
    let g = p.layout_no_wrap(text, semibold(10.5), Color32::WHITE);
    let at = c + vec2(-g.size().x / 2.0, r + 8.0);
    let bg = Rect::from_min_size(at - vec2(6.0, 2.0), g.size() + vec2(12.0, 4.0));
    p.rect_filled(bg, egui::CornerRadius::same(3), Color32::from_rgba_unmultiplied(4, 10, 22, 200));
    p.galley(at, g, Color32::WHITE);
}
