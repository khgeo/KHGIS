//! The QGIS layout's docks: every panel (Browser, Layers, Data Source Manager and each right pane)
//! sits in a dock area left or right of the map. An area holds groups stacked top to bottom, and
//! a group holds panels as tabs. As in QGIS, a panel is moved by dragging its tab: dropped on
//! another group's tabs or middle it joins that group as a tab; dropped on a group's top or
//! bottom edge it gets a group of its own there; dropped on the window's left or right edge it
//! goes to that side. A tab's right-click menu does the same without dragging.
//!
//! Which panels are open is still decided where it always was (`UiState::right_open`, the
//! Layers and Browser switches, `UiState::left` for the Data Source Manager): this module only
//! places them. A closed panel keeps its place, so it opens again where it was. The arrangement
//! is saved in the `qgis_docks` preference.

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::theme::{Tokens, regular, semibold};
use crate::{LeftPane, RightPane, RustGisApp, icons};

/// The preference holding the arrangement.
pub const PREF: &str = "qgis_docks";
/// A group's tab strip.
const TAB_H: f32 = 26.0;
/// The bar between two stacked groups.
const SPLIT_H: f32 = 5.0;
/// The smallest height a group is squeezed to.
const MIN_GROUP_H: f32 = 70.0;
/// The window-edge strips a dragged panel can be dropped on.
const EDGE_W: f32 = 44.0;

/// A dockable panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Browser,
    Layers,
    Sources,
    Right(RightPane),
}

impl Panel {
    fn key(self) -> &'static str {
        match self {
            Panel::Browser => "browser",
            Panel::Layers => "layers",
            Panel::Sources => "sources",
            Panel::Right(p) => crate::panes::right_id(p),
        }
    }

    fn from_key(k: &str) -> Option<Panel> {
        match k {
            "browser" => Some(Panel::Browser),
            "layers" => Some(Panel::Layers),
            "sources" => Some(Panel::Sources),
            _ => crate::panes::RIGHT_PANES.iter().copied().find(|p| crate::panes::right_id(*p) == k).map(Panel::Right),
        }
    }

    /// The side a panel docks on until the user moves it.
    fn home(self) -> usize {
        match self {
            Panel::Browser | Panel::Layers | Panel::Sources => LEFT,
            Panel::Right(_) => RIGHT,
        }
    }
}

const LEFT: usize = 0;
const RIGHT: usize = 1;

/// Panels shown as tabs, one in view.
#[derive(Clone, Debug)]
pub struct Group {
    tabs: Vec<Panel>,
    active: Panel,
    /// Its share of the area's height among the groups shown.
    weight: f32,
}

/// Where a dragged panel would land.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drop {
    /// As a tab of group `g` of `side`.
    Tab(usize, usize),
    /// As a group of its own before group `g` of `side` (`g` may be the count: at the end).
    Group(usize, usize),
}

/// A panel being dragged by its tab.
#[derive(Clone, Copy, Debug)]
struct Dragging(Panel);

/// The arrangement: the groups of the left and right areas.
#[derive(Clone, Debug, Default)]
pub struct Docks {
    areas: [Vec<Group>; 2],
    /// The right pane in view last frame, so a pane brought into view (by a command) shows.
    last_right: Option<RightPane>,
    /// The layout open last frame, so opening one brings up its Item Properties.
    last_layout: Option<usize>,
    /// Where the panel being dragged would land, found while drawing this frame.
    drop: Option<Drop>,
    loaded: bool,
}

impl Docks {
    fn find(&self, p: Panel) -> Option<(usize, usize)> {
        (0..2).find_map(|s| self.areas[s].iter().position(|g| g.tabs.contains(&p)).map(|g| (s, g)))
    }

    /// Puts `p` where `drop` says, from wherever it is.
    fn place(&mut self, p: Panel, drop: Drop) {
        let mut drop = drop;
        if let Some((s, g)) = self.find(p) {
            let group = &mut self.areas[s][g];
            group.tabs.retain(|t| *t != p);
            if group.active == p
                && let Some(first) = group.tabs.first()
            {
                group.active = *first;
            }
            if group.tabs.is_empty() {
                self.areas[s].remove(g);
                // The groups after the one removed move up by one.
                drop = match drop {
                    Drop::Tab(ds, dg) if ds == s && dg == g => Drop::Group(s, g),
                    Drop::Tab(ds, dg) if ds == s && dg > g => Drop::Tab(ds, dg - 1),
                    Drop::Group(ds, dg) if ds == s && dg > g => Drop::Group(ds, dg - 1),
                    other => other,
                };
            }
        }
        match drop {
            Drop::Tab(side, g) => {
                if let Some(group) = self.areas[side].get_mut(g) {
                    group.tabs.push(p);
                    group.active = p;
                } else {
                    self.areas[side].push(Group { tabs: vec![p], active: p, weight: 1.0 });
                }
            }
            Drop::Group(side, g) => {
                let g = g.min(self.areas[side].len());
                self.areas[side].insert(g, Group { tabs: vec![p], active: p, weight: 1.0 });
            }
        }
    }

    fn json(&self) -> Value {
        let side = |s: usize| -> Value {
            Value::Array(
                self.areas[s]
                    .iter()
                    .map(|g| json!({"tabs": g.tabs.iter().map(|t| t.key()).collect::<Vec<_>>(), "active": g.active.key(), "weight": g.weight}))
                    .collect(),
            )
        };
        json!({"left": side(LEFT), "right": side(RIGHT)})
    }

    fn from_json(v: &Value) -> Docks {
        let mut d = Docks::default();
        for (s, name) in [(LEFT, "left"), (RIGHT, "right")] {
            for g in v.get(name).and_then(Value::as_array).into_iter().flatten().take(32) {
                let mut tabs: Vec<Panel> = Vec::new();
                for p in g.get("tabs").and_then(Value::as_array).into_iter().flatten().filter_map(|t| t.as_str().and_then(Panel::from_key))
                {
                    if d.find(p).is_none() && !tabs.contains(&p) && tabs.len() < 64 {
                        tabs.push(p);
                    }
                }
                let Some(first) = tabs.first().copied() else { continue };
                let active =
                    g.get("active").and_then(Value::as_str).and_then(Panel::from_key).filter(|a| tabs.contains(a)).unwrap_or(first);
                let weight =
                    g.get("weight").and_then(Value::as_f64).filter(|w| w.is_finite() && *w > 0.05 && *w < 20.0).unwrap_or(1.0) as f32;
                d.areas[s].push(Group { tabs, active, weight });
            }
        }
        d
    }
}

/// The panels open now.
fn open_panels(app: &RustGisApp) -> Vec<Panel> {
    let mut out = Vec::new();
    if app.ui.qgis.browser {
        out.push(Panel::Browser);
    }
    if app.ui.qgis.layers {
        out.push(Panel::Layers);
    }
    if app.ui.left == Some(LeftPane::Sources) {
        out.push(Panel::Sources);
    }
    out.extend(app.ui.right_open.iter().map(|p| Panel::Right(*p)));
    out
}

/// Whether a panel is drawn: open, and, while a print layout is open, one of the layout's
/// (Item Properties, as in QGIS's Layout window).
fn is_open(app: &RustGisApp, p: Panel) -> bool {
    if app.session.active_layout.is_some() && p != Panel::Right(RightPane::Format) {
        return false;
    }
    is_open_any(app, p)
}

fn is_open_any(app: &RustGisApp, p: Panel) -> bool {
    match p {
        Panel::Browser => app.ui.qgis.browser,
        Panel::Layers => app.ui.qgis.layers,
        Panel::Sources => app.ui.left == Some(LeftPane::Sources),
        Panel::Right(r) => app.ui.right_open.contains(&r),
    }
}

fn close(app: &mut RustGisApp, p: Panel) {
    match p {
        Panel::Browser => app.ui.qgis.browser = false,
        Panel::Layers => app.ui.qgis.layers = false,
        Panel::Sources => app.ui.left = None,
        Panel::Right(r) => app.ui.close_right(r),
    }
}

fn save(app: &RustGisApp) {
    if let Some(save) = &app.services.save_pref {
        save(PREF, &app.ui.qgis.docks.json().to_string());
    }
}

/// Brings the arrangement up to date with the panels open: a panel opened for the first time
/// docks on its home side, as a tab of the first group shown there (the Browser and the Layers
/// stack, as QGIS starts).
fn sync(app: &mut RustGisApp) {
    if !app.ui.qgis.docks.loaded {
        app.ui.qgis.docks.loaded = true;
        if let Some(text) = app.services.load_pref.as_ref().and_then(|l| l(PREF))
            && let Ok(v) = serde_json::from_str::<Value>(&text)
        {
            app.ui.qgis.docks = Docks { loaded: true, ..Docks::from_json(&v) };
        }
    }
    // Commands open the Layers and the Browser by setting the left pane.
    match app.ui.left {
        Some(LeftPane::Contents) => {
            app.ui.qgis.layers = true;
            app.ui.left = None;
        }
        Some(LeftPane::Catalog) => {
            app.ui.qgis.browser = true;
            app.ui.left = None;
        }
        _ => {}
    }
    // A print layout opened: its Item Properties come up, as in QGIS's Layout window.
    if app.session.active_layout != app.ui.qgis.docks.last_layout {
        app.ui.qgis.docks.last_layout = app.session.active_layout;
        if app.session.active_layout.is_some() {
            app.ui.open_right(RightPane::Format);
        } else {
            // Back on the map, the layout's panel goes with it.
            app.ui.close_right(RightPane::Format);
        }
    }
    let open = open_panels(app);
    for p in open {
        if app.ui.qgis.docks.find(p).is_some() {
            continue;
        }
        let side = p.home();
        let shown = app.ui.qgis.docks.areas[side].iter().position(|g| g.tabs.iter().any(|t| is_open_any(app, *t)));
        let drop = match (p, shown) {
            (Panel::Browser, _) => Drop::Group(side, 0),
            // The Layers join the Browser as a tab, in view, as the panels start.
            (Panel::Layers, _) => match app.ui.qgis.docks.find(Panel::Browser) {
                Some((s, g)) if s == side => Drop::Tab(side, g),
                _ => Drop::Group(side, app.ui.qgis.docks.areas[side].len()),
            },
            (_, Some(g)) => Drop::Tab(side, g),
            (_, None) => Drop::Group(side, app.ui.qgis.docks.areas[side].len()),
        };
        app.ui.qgis.docks.place(p, drop);
    }
    // A right pane brought into view (a command, a toolbar button) is the tab in view.
    if app.ui.right != app.ui.qgis.docks.last_right {
        if let Some(r) = app.ui.right
            && let Some((s, g)) = app.ui.qgis.docks.find(Panel::Right(r))
        {
            app.ui.qgis.docks.areas[s][g].active = Panel::Right(r);
        }
        app.ui.qgis.docks.last_right = app.ui.right;
    }
}

/// Draws both dock areas; returns their rects.
pub fn show(app: &mut RustGisApp, ui: &mut Ui, frame: egui::Frame, t: &Tokens) -> [Option<Rect>; 2] {
    sync(app);
    app.ui.qgis.docks.drop = None;
    let dragging = egui::DragAndDrop::payload::<Dragging>(ui.ctx()).map(|d| d.0);
    let left = area(app, ui, LEFT, frame, t, dragging);
    let right = area(app, ui, RIGHT, frame, t, dragging);
    // The window's edges take a panel to a side, also one with nothing docked yet.
    if dragging.is_some() {
        let room = ui.available_rect_before_wrap();
        for (side, edge) in [
            (LEFT, Rect::from_min_size(room.min, vec2(EDGE_W, room.height()))),
            (RIGHT, Rect::from_min_size(pos2(room.max.x - EDGE_W, room.min.y), vec2(EDGE_W, room.height()))),
        ] {
            let hover = ui.ctx().pointer_hover_pos().is_some_and(|p| edge.contains(p));
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("qgis_dock_edges")));
            painter.rect_filled(edge, CornerRadius::same(4), t.accent.gamma_multiply(if hover { 0.45 } else { 0.15 }));
            painter.text(edge.center(), Align2::CENTER_CENTER, if side == LEFT { "⇤" } else { "⇥" }, semibold(16.0), t.on_accent);
            if hover {
                let n = app.ui.qgis.docks.areas[side].len();
                app.ui.qgis.docks.drop = Some(Drop::Group(side, n));
            }
        }
    }
    // A drag let go: the panel moves where it was dropped.
    if let Some(p) = dragging
        && ui.ctx().input(|i| i.pointer.any_released())
    {
        if let Some(drop) = app.ui.qgis.docks.drop.take() {
            app.ui.qgis.docks.place(p, drop);
            if let Panel::Right(r) = p {
                app.ui.right = Some(r);
            }
            save(app);
        }
        egui::DragAndDrop::clear_payload(ui.ctx());
    }
    [left, right]
}

fn area(app: &mut RustGisApp, ui: &mut Ui, side: usize, frame: egui::Frame, t: &Tokens, dragging: Option<Panel>) -> Option<Rect> {
    // The groups with a panel open; closed panels keep their place but aren't drawn.
    let shown: Vec<usize> = (0..app.ui.qgis.docks.areas[side].len())
        .filter(|g| app.ui.qgis.docks.areas[side][*g].tabs.iter().any(|p| is_open(app, *p)))
        .collect();
    if shown.is_empty() {
        return None;
    }
    let room = ui.available_width();
    let panel = if side == LEFT {
        egui::Panel::left("qgis_dock_left").default_size(280.0).size_range(200.0..=(room * 0.45).max(220.0))
    } else {
        egui::Panel::right("qgis_dock_right").default_size(330.0).size_range(240.0..=(room * 0.6).max(260.0))
    };
    let resp = panel.resizable(true).frame(frame).show(ui, |ui| {
        let mut all = ui.available_rect_before_wrap();
        // The search box sits at the foot of the left area, the app's lower-left corner.
        if side == LEFT {
            let foot = Rect::from_min_max(pos2(all.min.x, all.max.y - 40.0), all.max);
            let mut f = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::bottom_up(egui::Align::Min)));
            crate::searchbox::show(app, &mut f, t);
            all.max.y = f.min_rect().min.y.min(all.max.y - 30.0);
        }
        let n = shown.len() as f32;
        let total_w: f32 = shown.iter().map(|g| app.ui.qgis.docks.areas[side][*g].weight).sum::<f32>().max(0.01);
        let free = (all.height() - SPLIT_H * (n - 1.0)).max(MIN_GROUP_H * n);
        let mut y = all.min.y;
        for (i, &g) in shown.iter().enumerate() {
            let h = if i + 1 == shown.len() {
                (all.max.y - y).max(MIN_GROUP_H)
            } else {
                (free * app.ui.qgis.docks.areas[side][g].weight / total_w).max(MIN_GROUP_H)
            };
            let rect = Rect::from_min_size(pos2(all.min.x, y), vec2(all.width(), h));
            group(app, ui, side, g, rect, t, dragging);
            y = rect.max.y;
            if let Some(&next) = shown.get(i + 1) {
                let sr = Rect::from_min_size(pos2(all.min.x, y), vec2(all.width(), SPLIT_H));
                let r = ui
                    .interact(sr, egui::Id::unique(("qgis_dock_split", side, i)), Sense::drag())
                    .on_hover_cursor(egui::CursorIcon::ResizeVertical);
                ui.painter().rect_filled(sr, 0.0, if r.hovered() || r.dragged() { t.border_strong } else { t.border });
                if r.dragged() {
                    // Weight moves from one group to its neighbour, as the bar moves.
                    let d = r.drag_delta().y / free * total_w;
                    let docks = &mut app.ui.qgis.docks.areas[side];
                    let (a, b) = (docks[g].weight, docks[next].weight);
                    let floor = 0.12 * total_w;
                    let moved = d.clamp(floor - a, b - floor);
                    docks[g].weight = a + moved;
                    docks[next].weight = b - moved;
                }
                if r.drag_stopped() {
                    save(app);
                }
                y = sr.max.y;
            }
        }
        ui.advance_cursor_after_rect(all);
    });
    Some(resp.response.rect)
}

fn label(app: &RustGisApp, p: Panel) -> String {
    match p {
        Panel::Browser => tl!("Browser").to_string(),
        Panel::Layers => tl!("Layers").to_string(),
        Panel::Sources => tl!("Data Source Manager").to_string(),
        Panel::Right(r) => crate::panes::right_label(app, r),
    }
}

fn group(app: &mut RustGisApp, ui: &mut Ui, side: usize, g: usize, rect: Rect, t: &Tokens, dragging: Option<Panel>) {
    let tabs: Vec<Panel> = app.ui.qgis.docks.areas[side][g].tabs.iter().copied().filter(|p| is_open(app, *p)).collect();
    let mut active = app.ui.qgis.docks.areas[side][g].active;
    if !tabs.contains(&active) {
        active = tabs[0];
        app.ui.qgis.docks.areas[side][g].active = active;
    }
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), TAB_H));
    ui.painter().rect_filled(strip, 0.0, t.pane_header);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    // The tabs, cut to the strip; each drags, clicks into view, closes with its ✕, and has a menu.
    let mut x = strip.min.x + 2.0;
    let mut clicked = None;
    let mut closed = None;
    let mut moved: Option<(Panel, Drop)> = None;
    let painter = ui.painter().with_clip_rect(strip);
    for p in &tabs {
        let name = label(app, *p);
        let w =
            (ui.fonts_mut(|f| f.layout_no_wrap(name.clone(), semibold(12.0), t.text).size().x) + 34.0).min(strip.width() - 4.0).max(60.0);
        let r = Rect::from_min_size(pos2(x, strip.min.y + 2.0), vec2(w, TAB_H - 2.0));
        x += w + 2.0;
        let on = *p == active;
        let resp = ui.interact(r.intersect(strip), egui::Id::unique(("qgis_tab", p.key())), Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tab, true, &name));
        if on {
            painter.rect_filled(r, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, t.pane);
            painter.line_segment([pos2(r.min.x + 4.0, r.max.y - 1.0), pos2(r.max.x - 4.0, r.max.y - 1.0)], Stroke::new(2.0, t.accent));
        } else if resp.hovered() {
            painter.rect_filled(r, CornerRadius::same(4), t.hover);
        }
        painter.text(
            pos2(r.min.x + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            &name,
            if on { semibold(12.0) } else { regular(12.0) },
            if on { t.text } else { t.text_dim },
        );
        let xr = Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(16.0, 16.0));
        let xresp = ui.interact(xr.intersect(strip), egui::Id::unique(("qgis_tab_close", p.key())), Sense::click());
        if xresp.hovered() {
            painter.rect_filled(xr, CornerRadius::same(3), t.hover);
        }
        icons::paint(&painter, xr.shrink(4.0), "close", t.text_dim, t.accent);
        if xresp.on_hover_text(tl!("Close")).clicked() {
            closed = Some(*p);
        } else if resp.clicked() {
            clicked = Some(*p);
        }
        if resp.drag_started() {
            resp.dnd_set_drag_payload(Dragging(*p));
        }
        let resp = resp
            .on_hover_text(tl!("Drag to move this panel: onto other tabs, above or below a panel, or to the window's left or right edge"));
        resp.context_menu(|ui| {
            let other = if side == LEFT { RIGHT } else { LEFT };
            let to = if side == LEFT { tl!("Move to the Right Dock") } else { tl!("Move to the Left Dock") };
            if ui.button(to).clicked() {
                moved = Some((*p, Drop::Group(other, app.ui.qgis.docks.areas[other].len())));
                ui.close();
            }
            if tabs.len() > 1 && ui.button(tl!("Separate From the Tabs")).clicked() {
                moved = Some((*p, Drop::Group(side, g + 1)));
                ui.close();
            }
            if g > 0 && ui.button(tl!("Tab With the Panel Above")).clicked() {
                moved = Some((*p, Drop::Tab(side, g - 1)));
                ui.close();
            }
            if ui.button(tl!("Close")).clicked() {
                closed = Some(*p);
                ui.close();
            }
        });
    }
    if let Some(p) = clicked {
        app.ui.qgis.docks.areas[side][g].active = p;
        if let Panel::Right(r) = p {
            app.ui.right = Some(r);
            app.ui.qgis.docks.last_right = Some(r);
        }
        active = p;
    }
    // The panel in view.
    let body = Rect::from_min_max(pos2(rect.min.x, strip.max.y), rect.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)).id_salt(("qgis_dock_body", active.key())),
    );
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    match active {
        Panel::Browser => crate::qgisbrowser::show(app, &mut child, t),
        Panel::Layers => crate::panes::contents_in(app, &mut child, t),
        Panel::Sources => crate::sourcespane::show(app, &mut child, t),
        Panel::Right(r) => crate::panes::right_body(app, &mut child, r, t),
    }
    // While a panel is dragged, where it would land here: the tabs or middle join this group, the
    // top or bottom quarter makes a group above or below it.
    if let Some(d) = dragging
        && let Some(at) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p))
        && !(tabs.len() == 1 && tabs[0] == d)
    {
        let q = (rect.height() * 0.25).min(60.0);
        let (drop, hl) = if at.y > strip.max.y && at.y < rect.min.y + TAB_H + q {
            (Drop::Group(side, g), Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.5)))
        } else if at.y > rect.max.y - q {
            (Drop::Group(side, g + 1), Rect::from_min_max(pos2(rect.min.x, rect.center().y), rect.max))
        } else {
            (Drop::Tab(side, g), rect)
        };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("qgis_dock_drop")));
        painter.rect_filled(hl, CornerRadius::same(4), t.accent.gamma_multiply(0.25));
        painter.rect_stroke(hl, CornerRadius::same(4), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        app.ui.qgis.docks.drop = Some(drop);
    }
    if let Some(p) = closed {
        close(app, p);
    }
    if let Some((p, drop)) = moved {
        app.ui.qgis.docks.place(p, drop);
        save(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(d: &Docks, side: usize) -> Vec<Vec<&'static str>> {
        d.areas[side].iter().map(|g| g.tabs.iter().map(|p| p.key()).collect()).collect()
    }

    #[test]
    fn panels_move_between_groups_and_sides() {
        let mut d = Docks::default();
        d.place(Panel::Browser, Drop::Group(LEFT, 0));
        d.place(Panel::Layers, Drop::Group(LEFT, 1));
        d.place(Panel::Right(RightPane::Geoprocessing), Drop::Group(RIGHT, 0));
        d.place(Panel::Right(RightPane::Symbology), Drop::Tab(RIGHT, 0));
        assert_eq!(tabs(&d, LEFT), [["browser"], ["layers"]]);
        assert_eq!(tabs(&d, RIGHT), [["ui.pane.geoprocessing", "ui.pane.symbology"]]);
        // Layers as a tab with the Browser.
        d.place(Panel::Layers, Drop::Tab(LEFT, 0));
        assert_eq!(tabs(&d, LEFT), [["browser", "layers"]]);
        // The Browser to the right, below the toolbox; the Layers stay.
        d.place(Panel::Browser, Drop::Group(RIGHT, 1));
        assert_eq!(tabs(&d, LEFT), [["layers"]]);
        assert_eq!(tabs(&d, RIGHT), [vec!["ui.pane.geoprocessing", "ui.pane.symbology"], vec!["browser"]]);
        // A group emptied before the target doesn't shift it wrongly.
        d.place(Panel::Layers, Drop::Group(RIGHT, 0));
        assert!(d.areas[LEFT].is_empty());
        assert_eq!(tabs(&d, RIGHT)[0], ["layers"]);
        let again = Docks::from_json(&d.json());
        assert_eq!(tabs(&again, RIGHT), tabs(&d, RIGHT));
    }

    #[test]
    fn a_group_moved_within_its_side_keeps_the_others_in_order() {
        let mut d = Docks::default();
        d.place(Panel::Browser, Drop::Group(LEFT, 0));
        d.place(Panel::Layers, Drop::Group(LEFT, 1));
        d.place(Panel::Sources, Drop::Group(LEFT, 2));
        // The Browser (group 0, emptied) dropped below the Layers (target 2 before removal).
        d.place(Panel::Browser, Drop::Group(LEFT, 2));
        assert_eq!(tabs(&d, LEFT), [["layers"], ["browser"], ["sources"]]);
        d.place(Panel::Sources, Drop::Tab(LEFT, 0));
        assert_eq!(tabs(&d, LEFT), [vec!["layers", "sources"], vec!["browser"]]);
    }

    #[test]
    fn junk_arrangements_are_read_safely() {
        let d = Docks::from_json(&json!({"left": [{"tabs": ["nope", 3, "layers", "layers"], "weight": -1}], "right": "x"}));
        assert_eq!(tabs(&d, LEFT), [["layers"]]);
        assert_eq!(d.areas[LEFT][0].weight, 1.0);
    }
}
