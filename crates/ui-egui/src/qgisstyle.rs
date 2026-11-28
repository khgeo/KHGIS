//! Layer Styling as QGIS lays it out (the QGIS layout's panel, `qgisui::layer_styling`):
//!
//! - a single symbol as a preview beside its tree of symbol layers (Fill › Simple Fill and the
//!   fills stacked on it), with buttons to add, remove and reorder them, the properties of the one
//!   picked below, and a gallery of symbols to start from (with the user's own, saved);
//! - a raster's render type by QGIS's names, and Paletted/Unique values as a table of values,
//!   colours and labels with Classify, add, remove and Delete All;
//! - Layer Rendering: blending mode and opacity, and a raster's brightness, saturation, contrast,
//!   gamma and inverted colours.
//!
//! Every change runs a command, as the ribbon layout's Symbology pane does.

use egui::{CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use rustgis_engine::data::GeometryType;
use rustgis_engine::map::fills::{FillLayer, FillPattern, GradientPoint, GradientShape, Spread, Stop, Stops};
use rustgis_engine::map::{Layer, Marker, RasterRenderer, Rgba, Symbol};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::theme::{Tokens, c32, rgba, semibold};
use crate::{RustGisApp, icons};

/// The node of the symbol tree picked: the symbol itself, its simple layer, or a stacked fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Root,
    Simple,
    Fill(usize),
}

/// The pref the user's saved symbols are kept in.
const SAVED_PREF: &str = "qgis_symbols";
/// Most symbols kept.
const MAX_SAVED: usize = 64;

/// A symbol in the gallery: one of KHGIS's, or one the user saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Preset {
    pub name: String,
    /// "polygon", "line" or "point".
    pub geom: String,
    pub symbol: Symbol,
    #[serde(default)]
    pub fills: Vec<FillLayer>,
    /// A line's dash pattern, in line widths.
    #[serde(default)]
    pub dash: Option<Vec<f32>>,
}

fn geom_code(g: GeometryType) -> &'static str {
    match g {
        GeometryType::Point => "point",
        GeometryType::Polyline => "line",
        _ => "polygon",
    }
}

fn sym(fill: Rgba, stroke: Rgba, width: f32, size: f32, marker: Marker) -> Symbol {
    Symbol { fill, stroke, stroke_width: width, size, marker }
}

fn hex(h: &str) -> Rgba {
    Rgba::from_hex(h).unwrap_or(Rgba::rgb(0, 0, 0))
}

fn hatch(angle: f32, color: &str) -> FillLayer {
    FillLayer { color: Some(hex(color)), pattern: FillPattern::Hatch { angle, spacing: 6.0, width: 0.8, offset: 0.0 } }
}

/// The gallery's own symbols, as QGIS's default style library has them.
pub(crate) fn defaults(g: GeometryType) -> Vec<Preset> {
    let none = Rgba::rgb(0, 0, 0).with_alpha(0);
    let p = |name: &str, symbol: Symbol, fills: Vec<FillLayer>, dash: Option<Vec<f32>>| Preset {
        name: name.to_string(),
        geom: geom_code(g).to_string(),
        symbol,
        fills,
        dash,
    };
    match g {
        GeometryType::Point => [
            ("red circle", "#e31a1c", Marker::Circle),
            ("blue square", "#1f78b4", Marker::Square),
            ("green triangle", "#33a02c", Marker::Triangle),
            ("orange diamond", "#ff7f00", Marker::Diamond),
            ("yellow star", "#ffd92f", Marker::Star),
            ("black dot", "#232323", Marker::Circle),
            ("purple circle", "#6a3d9a", Marker::Circle),
            ("white square", "#ffffff", Marker::Square),
        ]
        .into_iter()
        .map(|(n, c, m)| p(n, sym(hex(c), hex("#232323"), 0.6, if n == "black dot" { 4.0 } else { 8.0 }, m), Vec::new(), None))
        .collect(),
        GeometryType::Polyline => vec![
            p("simple black", sym(none, hex("#232323"), 0.8, 6.0, Marker::Circle), Vec::new(), None),
            p("river", sym(none, hex("#3a86c8"), 1.6, 6.0, Marker::Circle), Vec::new(), None),
            p("main road", sym(none, hex("#e31a1c"), 2.4, 6.0, Marker::Circle), Vec::new(), None),
            p("minor road", sym(none, hex("#ff9f1c"), 1.4, 6.0, Marker::Circle), Vec::new(), None),
            p("dashed black", sym(none, hex("#232323"), 1.0, 6.0, Marker::Circle), Vec::new(), Some(vec![4.0, 2.0])),
            p("dotted gray", sym(none, hex("#7f7f7f"), 1.2, 6.0, Marker::Circle), Vec::new(), Some(vec![1.0, 2.0])),
            p("boundary", sym(none, hex("#6a3d9a"), 1.2, 6.0, Marker::Circle), Vec::new(), Some(vec![4.0, 2.0, 1.0, 2.0])),
            p("thin gray", sym(none, hex("#9a9a9a"), 0.4, 6.0, Marker::Circle), Vec::new(), None),
        ],
        _ => {
            let plasma = FillLayer {
                color: Some(hex("#0d0887")),
                pattern: FillPattern::Gradient {
                    shape: GradientShape::Linear,
                    color2: hex("#f0f921"),
                    stops: Stops::new([Stop { at: 0.33, color: hex("#9c179e") }, Stop { at: 0.66, color: hex("#ed7953") }]),
                    discrete: false,
                    from: GradientPoint::At([0.0, 0.0]),
                    to: GradientPoint::At([1.0, 1.0]),
                    spread: Spread::Pad,
                    angle: 0.0,
                    in_view: false,
                },
            };
            let dots = FillLayer {
                color: Some(hex("#232323")),
                pattern: FillPattern::Dots {
                    marker: Marker::Circle,
                    size: 2.0,
                    outline: None,
                    outline_width: 0.0,
                    spacing: [6.0, 6.0],
                    displacement: [3.0, 0.0],
                    offset: [0.0, 0.0],
                },
            };
            let black = hex("#232323");
            vec![
                p("gradient plasma", sym(hex("#0d0887"), black, 0.6, 6.0, Marker::Circle), vec![plasma], None),
                p("gray 3 fill", sym(hex("#9a9a9a"), hex("#9a9a9a"), 0.0, 6.0, Marker::Circle), Vec::new(), None),
                p("hashed black /", sym(none, black, 0.8, 6.0, Marker::Circle), vec![hatch(45.0, "#232323")], None),
                p("hashed black \\", sym(none, black, 0.8, 6.0, Marker::Circle), vec![hatch(135.0, "#232323")], None),
                p(
                    "hashed black X",
                    sym(none, black, 0.8, 6.0, Marker::Circle),
                    vec![hatch(45.0, "#232323"), hatch(135.0, "#232323")],
                    None,
                ),
                p("outline blue", sym(none, hex("#1f78b4"), 1.6, 6.0, Marker::Circle), Vec::new(), None),
                p("outline green", sym(none, hex("#33a02c"), 1.6, 6.0, Marker::Circle), Vec::new(), None),
                p("outline red", sym(none, hex("#e31a1c"), 1.6, 6.0, Marker::Circle), Vec::new(), None),
                p("dotted black", sym(none, black, 0.6, 6.0, Marker::Circle), vec![dots], None),
                p("water", sym(hex("#a6cee3"), hex("#3a86c8"), 0.6, 6.0, Marker::Circle), Vec::new(), None),
                p("forest", sym(hex("#4d9a4d"), hex("#2d6b2d"), 0.6, 6.0, Marker::Circle), Vec::new(), None),
                p("sand", sym(hex("#f2dc9b"), hex("#b8a56a"), 0.6, 6.0, Marker::Circle), Vec::new(), None),
            ]
        }
    }
}

/// The user's saved symbols (all kinds).
fn saved(app: &RustGisApp) -> Vec<Preset> {
    app.services
        .load_pref
        .as_ref()
        .and_then(|l| l(SAVED_PREF))
        .and_then(|t| serde_json::from_str::<Vec<Preset>>(&t).ok())
        .unwrap_or_default()
}

fn store(app: &mut RustGisApp, list: &[Preset]) {
    if let Some(save) = &app.services.save_pref {
        save(SAVED_PREF, &serde_json::to_string(list).unwrap_or_else(|_| "[]".into()));
    }
}

/// Styles `l` with `p`, as one step to undo.
pub(crate) fn apply(app: &mut RustGisApp, l: &Layer, p: &Preset, key: &str) {
    let mut steps: Vec<(&str, Value)> = Vec::new();
    let s = &p.symbol;
    match l.geometry_type() {
        GeometryType::Polyline => {
            steps.push(("layer.color", json!({"color": s.stroke.hex8()})));
            steps.push(("layer.outline", json!({"width": s.stroke_width})));
            let strokes = match &p.dash {
                Some(d) => json!([{"color": s.stroke.hex8(), "width": s.stroke_width, "dash": d}]),
                None => json!([]),
            };
            steps.push(("layer.lineStrokes", json!({"strokes": strokes})));
        }
        GeometryType::Point => {
            steps.push(("layer.color", json!({"color": s.fill.hex8()})));
            steps.push(("layer.outline", json!({"color": s.stroke.hex8(), "width": s.stroke_width})));
            let mut m = json!({"size": s.size});
            match s.marker {
                Marker::Char(g) => {
                    m["char"] = json!(g.ch.to_string());
                    m["font"] = json!(g.font.as_str());
                }
                other => m["shape"] = json!(other.label()),
            }
            steps.push(("layer.marker", m));
        }
        _ => {
            steps.push(("layer.color", json!({"color": s.fill.hex8()})));
            steps.push(("layer.outline", json!({"color": s.stroke.hex8(), "width": s.stroke_width})));
            steps.push(("layer.fillLayers", json!({"fills": p.fills.iter().map(crate::fill_editor::fill_json).collect::<Vec<_>>()})));
        }
    }
    for (cmd, params) in steps {
        app.session.merge_undo(key.to_string());
        if let Err(e) = app.run(cmd, params) {
            app.session.notify(e, true);
            return;
        }
    }
}

/// The QGIS name of a fill in the tree.
fn fill_name(f: &FillLayer) -> &'static str {
    match f.pattern {
        FillPattern::Solid => "Solid Fill",
        FillPattern::Hatch { .. } => "Line Pattern Fill",
        FillPattern::Dots { .. } => "Point Pattern Fill",
        FillPattern::Gradient { .. } => "Gradient Fill",
        FillPattern::Shapeburst { .. } => "Shapeburst Fill",
    }
}

/// Draws `p` (a symbol with its fills and dash) on `r`, as the map would.
fn draw_preset(ui: &Ui, r: Rect, p: &Preset, g: GeometryType) {
    let painter = ui.painter();
    match g {
        GeometryType::Polygon | GeometryType::None => {
            let inner = r.shrink(p.symbol.stroke_width.max(1.0) / 2.0);
            if !p.fills.is_empty() {
                crate::fill_editor::preview(ui, inner, &p.fills, &p.symbol);
            } else if p.symbol.fill.a() > 0 {
                painter.rect_filled(inner, 0.0, c32(p.symbol.fill));
            }
            if p.symbol.stroke_width > 0.0 && p.symbol.stroke.a() > 0 {
                painter.rect_stroke(
                    inner,
                    0.0,
                    Stroke::new(p.symbol.stroke_width.max(1.0) * 1.4, c32(p.symbol.stroke)),
                    egui::StrokeKind::Middle,
                );
            }
        }
        GeometryType::Polyline => {
            let y = r.center().y;
            let w = (p.symbol.stroke_width * 1.6).max(1.0);
            let c = c32(p.symbol.stroke);
            match &p.dash {
                Some(d) if !d.is_empty() => {
                    let mut x = r.min.x;
                    let mut i = 0;
                    while x < r.max.x && i < 200 {
                        let len = d[i % d.len()] * w;
                        if i % 2 == 0 {
                            painter.line_segment([pos2(x, y), pos2((x + len).min(r.max.x), y)], Stroke::new(w, c));
                        }
                        x += len.max(1.0);
                        i += 1;
                    }
                }
                _ => {
                    painter.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(w, c));
                }
            }
        }
        _ => {
            let s = Symbol { size: p.symbol.size.min(r.height() * 0.8), ..p.symbol };
            crate::panes::patch(painter, r, &s, GeometryType::Point);
        }
    }
}

/// The layer's single symbol as QGIS shows it: preview, tree, the picked node's properties and
/// the gallery.
pub(crate) fn single_symbol(app: &mut RustGisApp, ui: &mut Ui, l: &Layer, s: Symbol, t: &Tokens) {
    let g = l.geometry_type();
    let polygon = l.has_geometry(GeometryType::Polygon) && g != GeometryType::Point && g != GeometryType::Polyline;
    let stack: Vec<FillLayer> = if polygon && l.no_fill.is_none() { l.fill_layers.clone() } else { Vec::new() };
    let node_id = egui::Id::unique(("qgis_symbol_node", l.id));
    let mut node = ui.data(|d| d.get_temp::<(u8, usize)>(node_id)).map_or(Node::Simple, |(k, i)| match k {
        0 => Node::Root,
        2 => Node::Fill(i),
        _ => Node::Simple,
    });
    if let Node::Fill(i) = node
        && i >= stack.len()
    {
        node = Node::Simple;
    }
    let (root_name, simple_name) = match g {
        GeometryType::Point => ("Marker", "Simple Marker"),
        GeometryType::Polyline => ("Line", "Simple Line"),
        _ => ("Fill", "Simple Fill"),
    };
    let current = Preset { name: String::new(), geom: geom_code(g).into(), symbol: s, fills: stack.clone(), dash: None };
    let mut picked = node;
    let mut set_stack: Option<Vec<FillLayer>> = None;
    // Preview | tree | buttons.
    ui.horizontal_top(|ui| {
        let side = 96.0_f32.min(ui.available_width() * 0.3);
        let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(4), egui::Color32::WHITE);
        ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        let inner = r.shrink(side * 0.2);
        draw_preset(ui, inner, &current, g);
        // Room for the buttons' column beside it (and the frame's margins).
        let tree_w = (ui.available_width() - 26.0 - 2.0 * ui.spacing().item_spacing.x - 10.0).max(110.0);
        egui::Frame::NONE
            .fill(t.input)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(tree_w);
                ui.set_min_height(side - 10.0);
                ui.vertical(|ui| {
                    let row = |ui: &mut Ui, depth: f32, label: &str, on: bool, sw: &dyn Fn(&Ui, Rect)| -> bool {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                        if on {
                            ui.painter().rect_filled(r, CornerRadius::same(3), t.checked);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(3), t.hover);
                        }
                        let sw_r = Rect::from_min_size(pos2(r.min.x + depth, r.min.y + 3.0), vec2(16.0, 16.0));
                        ui.painter().rect_filled(sw_r, 0.0, egui::Color32::WHITE);
                        sw(ui, sw_r.shrink(1.0));
                        ui.painter().text(
                            pos2(sw_r.max.x + 6.0, r.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            if depth == 4.0 { semibold(12.5) } else { crate::theme::regular(12.5) },
                            t.text,
                        );
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, true, on, label));
                        resp.clicked()
                    };
                    if row(ui, 4.0, &format!("▾ {}", tl!(root_name)), picked == Node::Root, &|ui, r| draw_preset(ui, r, &current, g)) {
                        picked = Node::Root;
                    }
                    for i in (0..stack.len()).rev() {
                        let one = Preset { fills: vec![stack[i]], symbol: Symbol { stroke_width: 0.0, ..s }, ..current.clone() };
                        if row(ui, 22.0, tl!(fill_name(&stack[i])), picked == Node::Fill(i), &|ui, r| draw_preset(ui, r, &one, g)) {
                            picked = Node::Fill(i);
                        }
                    }
                    let simple = Preset { fills: Vec::new(), ..current.clone() };
                    if row(ui, 22.0, tl!(simple_name), picked == Node::Simple, &|ui, r| draw_preset(ui, r, &simple, g)) {
                        picked = Node::Simple;
                    }
                });
            });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let enabled = |ui: &mut Ui, on: bool, icon: &str, tip: &str| -> bool {
                let r = ui.add_enabled_ui(on, |ui| crate::widgets::icon_button(ui, icon, tip)).inner;
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tip));
                r.clicked()
            };
            let n = stack.len();
            let can_add = polygon && n < rustgis_engine::map::fills::MAX_FILLS;
            if enabled(ui, can_add, "plus", tl!("Add symbol layer")) {
                let mut next = stack.clone();
                // The first stands for the symbol's own fill; a hatch goes on it.
                if next.is_empty() {
                    next.push(FillLayer::default());
                }
                next.push(FillLayer::hatch(45.0, 6.0, 1.0));
                picked = Node::Fill(next.len() - 1);
                set_stack = Some(next);
            }
            let sel = match picked {
                Node::Fill(i) => Some(i),
                _ => None,
            };
            if enabled(ui, sel.is_some(), "minus", tl!("Remove symbol layer"))
                && let Some(i) = sel
            {
                let mut next = stack.clone();
                next.remove(i);
                // Only the symbol's own fill left: no stack.
                if next.len() == 1 && next[0] == FillLayer::default() {
                    next.clear();
                }
                picked = if next.is_empty() { Node::Simple } else { Node::Fill(i.saturating_sub(1).min(next.len() - 1)) };
                set_stack = Some(next);
            }
            if enabled(ui, sel.is_some_and(|i| i + 1 < n), "up", tl!("Move up"))
                && let Some(i) = sel
            {
                let mut next = stack.clone();
                next.swap(i, i + 1);
                picked = Node::Fill(i + 1);
                set_stack = Some(next);
            }
            if enabled(ui, sel.is_some_and(|i| i > 0), "down", tl!("Move down"))
                && let Some(i) = sel
            {
                let mut next = stack.clone();
                next.swap(i, i - 1);
                picked = Node::Fill(i - 1);
                set_stack = Some(next);
            }
        });
    });
    if picked != node {
        let v = match picked {
            Node::Root => (0u8, 0usize),
            Node::Simple => (1, 0),
            Node::Fill(i) => (2, i),
        };
        ui.data_mut(|d| d.insert_temp(node_id, v));
    }
    if let Some(next) = set_stack {
        crate::fill_editor::set_fills(app, None, &next);
        return;
    }
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    // The picked node's properties.
    match picked {
        Node::Root => root_rows(app, ui, l, s, g),
        Node::Simple => crate::panes::symbol_rows(app, ui, s, g),
        Node::Fill(i) => crate::fill_editor::fill_rows(app, ui, i, &stack, None, &s, t),
    }
    ui.add_space(8.0);
    ui.separator();
    gallery(app, ui, l, &current, t);
}

/// The symbol's own rows, as QGIS's top node has them: its colour across the panel, and opacity.
fn root_rows(app: &mut RustGisApp, ui: &mut Ui, l: &Layer, s: Symbol, g: GeometryType) {
    egui::Grid::new(("qgis_root_rows", l.id)).num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
        ui.label(tl!("Color"));
        let line = g == GeometryType::Polyline;
        let mut c = c32(if line { s.stroke } else { s.fill });
        let w = ui.available_width().max(120.0);
        let resp = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.x = w;
                crate::widgets::color_button(ui, &mut c)
            })
            .inner;
        if resp.changed() {
            crate::widgets::one_step_per_press(app, &resp);
            let _ = app.run("layer.color", json!({"color": rgba(c).hex8()}));
        }
        ui.end_row();
        let input_label = ui.label(tl!("Opacity"));
        let mut op = 100.0 - l.transparency;
        let r = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = (w - 70.0).max(80.0);
                ui.add(egui::Slider::new(&mut op, 0.0..=100.0).update_while_editing(false).suffix(" %").max_decimals(1))
            })
            .inner
            .labelled_by(input_label.id);
        crate::panes::transparency_drag(app, ui, &r, l, 100.0 - op);
        ui.end_row();
        ui.label(tl!("Unit"));
        ui.label(tl!("Points")).on_hover_text(tl!("Sizes and widths are in points (1/72 inch)"));
        ui.end_row();
    });
}

/// The symbol gallery: KHGIS's symbols and the user's, by name, a click to use one; Save Symbol…
/// and the Advanced menu below.
fn gallery(app: &mut RustGisApp, ui: &mut Ui, l: &Layer, current: &Preset, t: &Tokens) {
    let g = l.geometry_type();
    let search_id = egui::Id::unique("qgis_symbol_search");
    let mut q: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, t.input_border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            if crate::widgets::search_field(ui, &mut q, tl!("Favorites — search symbols")).changed() {
                ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
            }
        });
    let ql = q.to_lowercase();
    let mine: Vec<Preset> = saved(app).into_iter().filter(|p| p.geom == geom_code(g)).collect();
    let key = format!("symbol preset {} {}", l.id, ui.input(|i| i.time));
    let mut chosen: Option<Preset> = None;
    let mut forget: Option<String> = None;
    let mut section = |ui: &mut Ui, title: &str, list: &[Preset], own: bool| {
        let list: Vec<&Preset> = list.iter().filter(|p| ql.is_empty() || p.name.to_lowercase().contains(&ql)).collect();
        if list.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!(title)).font(semibold(13.0)));
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, t.border);
        ui.add_space(4.0);
        let tile = vec2(84.0, 82.0);
        let per_row = ((ui.available_width() / (tile.x + 4.0)).floor() as usize).max(1);
        for chunk in list.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for p in chunk {
                    let (r, resp) = ui.allocate_exact_size(tile, Sense::click());
                    let on = p.symbol == current.symbol && p.fills == current.fills;
                    if on {
                        ui.painter().rect_filled(r, CornerRadius::same(4), t.checked);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
                    }
                    let sw = Rect::from_center_size(pos2(r.center().x, r.min.y + 28.0), vec2(46.0, 46.0));
                    draw_preset(ui, sw, p, g);
                    let name = if p.name.chars().count() > 12 {
                        format!("{}…", p.name.chars().take(11).collect::<String>())
                    } else {
                        p.name.clone()
                    };
                    ui.painter().text(
                        pos2(r.center().x, r.max.y - 9.0),
                        egui::Align2::CENTER_CENTER,
                        name,
                        crate::theme::regular(11.0),
                        t.text,
                    );
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &p.name));
                    let resp = resp.on_hover_text(&p.name);
                    if resp.clicked() {
                        chosen = Some((*p).clone());
                    }
                    if own {
                        resp.context_menu(|ui| {
                            if ui.button(tl!("Remove from my symbols")).clicked() {
                                forget = Some(p.name.clone());
                                ui.close();
                            }
                        });
                    }
                }
            });
        }
    };
    section(ui, "Default", &defaults(g), false);
    section(ui, "My Symbols", &mine, true);
    if let Some(p) = chosen {
        apply(app, l, &p, &key);
    }
    if let Some(name) = forget {
        let mut all = saved(app);
        all.retain(|p| !(p.name == name && p.geom == geom_code(g)));
        store(app, &all);
    }
    ui.add_space(8.0);
    // Save Symbol… and Advanced.
    let naming_id = egui::Id::unique(("qgis_symbol_naming", l.id));
    let mut naming: Option<String> = ui.data(|d| d.get_temp(naming_id));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(tl!("Advanced ▾"), |ui| advanced_menu(app, ui, l));
            if ui.button(tl!("Save Symbol…")).on_hover_text(tl!("Keep this symbol in My Symbols, for any layer")).clicked() {
                naming = Some(String::new());
            }
        });
    });
    if let Some(mut name) = naming.take() {
        let mut keep = true;
        ui.horizontal(|ui| {
            let input_label = ui.label(tl!("Name"));
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(160.0).char_limit(40)).labelled_by(input_label.id);
            if !r.has_focus() && name.is_empty() && !r.lost_focus() {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button(tl!("Save")).clicked() || enter) && !name.trim().is_empty() {
                let mut all = saved(app);
                let p = Preset { name: name.trim().to_string(), ..current.clone() };
                // A line's dash, as its first stroke has it.
                let p = Preset { dash: l.line_strokes.first().and_then(|st| st.dash.map(|d| d.steps().to_vec())), ..p };
                all.retain(|o| !(o.name == p.name && o.geom == p.geom));
                all.insert(0, p);
                all.truncate(MAX_SAVED);
                store(app, &all);
                keep = false;
            }
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
        });
        if keep {
            naming = Some(name);
        }
    }
    ui.data_mut(|d| match naming {
        Some(n) => {
            d.insert_temp(naming_id, n);
        }
        None => d.remove::<String>(naming_id),
    });
    // The Advanced menu's sections, shown when asked for.
    let more_id = egui::Id::unique(("qgis_symbol_more", l.id));
    if ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false) {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(egui::RichText::new(tl!("Data-defined")).font(semibold(12.5))).default_open(true).show(ui, |ui| {
            crate::panes::symbol_by_editor(app, ui, l, t);
        });
    }
}

/// The Advanced menu below the gallery.
fn advanced_menu(app: &mut RustGisApp, ui: &mut Ui, l: &Layer) {
    if l.has_geometry(GeometryType::Polygon)
        && ui.button(tl!("No Fill")).on_hover_text(tl!("Draw only polygon outlines, keeping their colors and widths.")).clicked()
    {
        let _ = app.run("layer.noFill", Value::Null);
        ui.close();
    }
    let more_id = egui::Id::unique(("qgis_symbol_more", l.id));
    let mut more = ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false);
    if ui.checkbox(&mut more, tl!("Data-defined size and colors")).changed() {
        ui.data_mut(|d| d.insert_temp(more_id, more));
    }
    if ui.button(tl!("Import QGIS Style…")).on_hover_text(tl!("Style this layer as a QGIS layer style file (.qml) does")).clicked() {
        let _ = app.run("ui.openItem", json!({"kind": "qgisStyle"}));
        ui.close();
    }
}

/// Layer Rendering: blending, opacity and (rasters) the colour adjustments.
pub(crate) fn layer_rendering(app: &mut RustGisApp, ui: &mut Ui, l: &Layer) {
    let raster = l.raster.as_ref().map(|r| r.style.clone());
    egui::CollapsingHeader::new(egui::RichText::new(tl!("Layer Rendering")).font(semibold(13.5)))
        .id_salt(("qgis_layer_rendering", raster.is_some()))
        .default_open(raster.is_some())
        .show(ui, |ui| {
            egui::Grid::new(("qgis_rendering", l.id)).num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                let input_label = ui.label(tl!("Blending mode"));
                egui::ComboBox::from_id_salt(("qgis_blend", l.id))
                    .selected_text(tl!(l.blend.label()))
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for b in rustgis_engine::map::effects::Blend::ALL {
                            if ui.selectable_label(b == l.blend, tl!(b.label())).clicked() && b != l.blend {
                                let _ = app.run("layer.blend", json!({"mode": b.code()}));
                            }
                        }
                    })
                    .response
                    .labelled_by(input_label.id);
                ui.end_row();
                let input_label = ui.label(tl!("Opacity"));
                let mut op = 100.0 - l.transparency;
                let r = ui
                    .add(egui::Slider::new(&mut op, 0.0..=100.0).update_while_editing(false).suffix(" %").integer())
                    .labelled_by(input_label.id);
                crate::panes::transparency_drag(app, ui, &r, l, 100.0 - op);
                ui.end_row();
                let Some(style) = raster else { return };
                // Brightness, saturation, contrast and gamma: one step per drag.
                let num = |app: &mut RustGisApp,
                           ui: &mut Ui,
                           label: &str,
                           key: &str,
                           v: f64,
                           range: std::ops::RangeInclusive<f64>,
                           dec: usize| {
                    let input_label = ui.label(tl!(label));
                    let mut x = v;
                    let r =
                        ui.add(egui::Slider::new(&mut x, range).update_while_editing(false).max_decimals(dec)).labelled_by(input_label.id);
                    if crate::widgets::changed(&r) && x != v {
                        crate::widgets::one_step_per_press(app, &r);
                        let _ = app.run("raster.enhance", json!({ key: x }));
                    }
                    ui.end_row();
                };
                num(app, ui, "Brightness", "brightness", style.brightness, -100.0..=100.0, 0);
                num(app, ui, "Saturation", "saturation", style.saturation, -100.0..=100.0, 0);
                num(app, ui, "Contrast", "contrast", style.contrast, -100.0..=100.0, 0);
                num(app, ui, "Gamma", "gamma", style.gamma, 0.1..=10.0, 2);
                ui.label("");
                let mut inv = style.invert;
                if ui.checkbox(&mut inv, tl!("Invert colors")).changed() {
                    let _ = app.run("raster.enhance", json!({"invert": inv}));
                }
                ui.end_row();
                if style.brightness != 0.0 || style.contrast != 0.0 || style.saturation != 0.0 || style.gamma != 1.0 || style.invert {
                    ui.label("");
                    if ui.small_button(tl!("Reset")).clicked() {
                        let _ = app
                            .run("raster.enhance", json!({"brightness": 0, "contrast": 0, "saturation": 0, "gamma": 1, "invert": false}));
                    }
                    ui.end_row();
                }
            });
            if l.raster.is_none() {
                crate::panes::reference_scale_row(app, ui, l);
            }
        });
}

/// The raster render types by QGIS's names.
fn render_name(code: &str) -> &'static str {
    match code {
        "rgb" => "Multiband color",
        "unique" => "Paletted/Unique values",
        "classify" => "Singleband pseudocolor (classes)",
        "colormap" => "Paletted (the file's colors)",
        "shadedRelief" => "Hillshade",
        _ => "Singleband pseudocolor",
    }
}

/// A raster's symbology as QGIS lays it out: the render type, its settings, Layer Rendering.
pub(crate) fn raster(app: &mut RustGisApp, ui: &mut Ui, l: &Layer, t: &Tokens) {
    let Some(rl) = l.raster.as_ref() else { return };
    if let Some(d) = rl.data.as_ref() {
        let has_categories = d.bands.first().is_some_and(|b| b.stats.categories.is_some());
        let palette = matches!(d.color, rustgis_engine::data::raster::ColorHint::Palette(_));
        let code = rl.style.renderer.code();
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            let icon = match code {
                "rgb" => "rgb",
                "unique" => "unique",
                "classify" => "classify",
                "shadedRelief" => "relief",
                _ => "stretch",
            };
            icons::paint(ui.painter(), r, icon, t.icon, t.accent);
            let combo = egui::ComboBox::from_id_salt("qgis_render_type")
                .width(ui.available_width() - 8.0)
                .selected_text(tl!(render_name(code)))
                .show_ui(ui, |ui| {
                    for (c, _) in RasterRenderer::KINDS {
                        let ok = match c {
                            "rgb" => d.band_count() >= 3,
                            "unique" => has_categories,
                            "colormap" => palette,
                            _ => true,
                        };
                        let resp = ui.add_enabled(ok, egui::Button::selectable(c == code, tl!(render_name(c))));
                        let resp = resp.on_disabled_hover_text(match c {
                            "rgb" => tl!("Needs three bands or more"),
                            "unique" => tl!("Needs a band of a few whole-number values (classes)"),
                            _ => tl!("Needs a color table in the file"),
                        });
                        if resp.clicked() && c != code {
                            let _ = app.run("raster.symbology", json!({"type": c}));
                        }
                    }
                });
            let name = tl!("Render type").to_string();
            combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        });
        ui.add_space(6.0);
    }
    crate::panes::raster_symbology(app, ui, l, t);
    ui.add_space(10.0);
    layer_rendering(app, ui, l);
}

/// Paletted/Unique values' table: each value's colour and label, with Classify, add, remove and
/// Delete All below.
pub(crate) fn unique_table(
    app: &mut RustGisApp,
    ui: &mut Ui,
    l: &Layer,
    band: usize,
    ramp: &rustgis_engine::map::Ramp,
    values: &[(f64, Rgba)],
    t: &Tokens,
) {
    let Some(d) = l.raster.as_ref().and_then(|r| r.data.clone()) else { return };
    let sel_id = egui::Id::unique(("qgis_unique_row", l.id));
    let mut sel: Option<usize> = ui.data(|m| m.get_temp(sel_id)).filter(|i: &usize| *i < values.len());
    let set = |app: &mut RustGisApp, list: &[(f64, Rgba)]| {
        let v: Vec<Value> = list.iter().map(|(v, c)| json!([v, c.hex()])).collect();
        if let Err(e) = app.run("raster.uniqueValues", json!({"values": v})) {
            app.session.notify(e, true);
        }
    };
    let label_of = |v: f64| d.bands.get(band).and_then(|b| b.class_name(v)).map(str::to_string);
    let fmt = |v: f64| if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") };
    egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let w = ui.available_width();
        let cols = [w * 0.22, w * 0.24, w * 0.54];
        // The header.
        let (hr, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        ui.painter().rect_filled(hr, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, t.pane_header);
        let mut x = hr.min.x;
        for (i, name) in ["Value", "Color", "Label"].into_iter().enumerate() {
            ui.painter().text(pos2(x + 8.0, hr.center().y), egui::Align2::LEFT_CENTER, tl!(name), crate::theme::regular(12.5), t.text);
            x += cols[i];
            if i < 2 {
                ui.painter().line_segment([pos2(x, hr.min.y + 3.0), pos2(x, hr.max.y - 3.0)], Stroke::new(1.0, t.border));
            }
        }
        egui::ScrollArea::vertical().id_salt(("qgis_unique_scroll", l.id)).max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
            for (i, (v, c)) in values.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                if sel == Some(i) {
                    ui.painter().rect_filled(r, 0.0, t.checked);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                if resp.clicked() {
                    sel = Some(i);
                }
                ui.painter().text(
                    pos2(r.min.x + 12.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    fmt(*v),
                    crate::theme::regular(13.0),
                    t.text,
                );
                // The colour, a button across its cell.
                let cr = Rect::from_min_size(pos2(r.min.x + cols[0] + 6.0, r.min.y + 3.0), vec2(cols[1] - 12.0, 24.0));
                let mut cc = c32(*c);
                let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(cr));
                cell.spacing_mut().interact_size = vec2(cr.width(), cr.height());
                let cresp = crate::widgets::color_button(&mut cell, &mut cc);
                if cresp.changed() {
                    crate::widgets::one_step_per_press(app, &cresp);
                    let mut next = values.to_vec();
                    next[i].1 = rgba(cc);
                    set(app, &next);
                }
                // The label: a class name, set when typed (whole-number classes only).
                let lr = Rect::from_min_size(pos2(r.min.x + cols[0] + cols[1] + 6.0, r.min.y + 4.0), vec2(cols[2] - 12.0, 22.0));
                let whole = *v >= 0.0 && v.fract() == 0.0 && *v < 100_000.0;
                let edit_id = egui::Id::unique(("qgis_unique_label", l.id, i));
                let shown = label_of(*v).unwrap_or_default();
                let mut text: String = ui.data(|m| m.get_temp(edit_id)).unwrap_or_else(|| shown.clone());
                let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(lr));
                let te = cell.add_enabled(
                    whole,
                    egui::TextEdit::singleline(&mut text).desired_width(lr.width()).hint_text(fmt(*v)).frame(egui::Frame::NONE),
                );
                if te.changed() {
                    ui.data_mut(|m| m.insert_temp(edit_id, text.clone()));
                }
                if te.lost_focus() {
                    ui.data_mut(|m| m.remove::<String>(edit_id));
                    if text != shown {
                        let r = app.run("raster.classNames", json!({"layer": l.name, "names": { fmt(*v): text }}));
                        if let Err(e) = r {
                            app.session.notify(e, true);
                        }
                    }
                }
                ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(0.5, t.border));
            }
            if values.is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("No values: press Classify, or + to add one.")).color(t.text_dim));
                ui.add_space(6.0);
            }
        });
    });
    ui.data_mut(|m| match sel {
        Some(i) => {
            m.insert_temp(sel_id, i);
        }
        None => m.remove::<usize>(sel_id),
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let w = ui.available_width();
        if ui
            .add(egui::Button::new(tl!("Classify")).min_size(vec2(((w - 120.0) / 2.0).max(60.0), 26.0)))
            .on_hover_text(tl!("A color for each value in the band"))
            .clicked()
        {
            let _ = app.run("raster.symbology", json!({"type": "unique", "band": band + 1}));
        }
        let add = crate::widgets::icon_button(ui, "plus", tl!("Add a value"));
        add.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Add a value")));
        if add.clicked() {
            let next_v = values.iter().map(|(v, _)| *v).fold(f64::NEG_INFINITY, f64::max);
            let v = if next_v.is_finite() { next_v.floor() + 1.0 } else { 0.0 };
            let mut next = values.to_vec();
            next.push((v, ramp.pick(values.len(), values.len() + 1)));
            set(app, &next);
            ui.data_mut(|m| m.insert_temp(sel_id, values.len()));
        }
        let rm =
            ui.add_enabled_ui(!values.is_empty(), |ui| crate::widgets::icon_button(ui, "minus", tl!("Remove the selected value"))).inner;
        rm.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !values.is_empty(), tl!("Remove the selected value")));
        if rm.clicked() {
            let i = sel.unwrap_or(values.len() - 1);
            let mut next = values.to_vec();
            next.remove(i);
            set(app, &next);
            ui.data_mut(|m| m.remove::<usize>(sel_id));
        }
        if ui
            .add_enabled(!values.is_empty(), egui::Button::new(tl!("Delete All")).min_size(vec2(((w - 120.0) / 2.0).max(60.0), 26.0)))
            .clicked()
        {
            set(app, &[]);
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use egui::vec2;
    use egui_kittest::kittest::Queryable;
    use serde_json::{Value, json};

    use crate::{RightPane, RustGisApp};

    fn harness(app: RustGisApp) -> egui_kittest::Harness<'static, RustGisApp> {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 1000.0)).with_max_steps(64).build_ui_state(
            |ui, app: &mut RustGisApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        h
    }

    fn qgis_app() -> RustGisApp {
        let mut app = RustGisApp::with_sample();
        let _ = app.run("ui.language", json!({"code": "en"}));
        app.ui.qgis_layout = true;
        app.ui.qgis.live = true;
        app.ui.open_right(RightPane::Symbology);
        app
    }

    /// The symbol tree adds and removes a fill layer; a gallery symbol styles the layer in one
    /// step to undo.
    #[test]
    fn symbol_tree_and_gallery() {
        let mut app = qgis_app();
        app.run("layer.select", json!({"layer": "Countries"})).unwrap();
        app.run("layer.symbology", json!({"type": "single"})).unwrap();
        let mut h = harness(app);
        let fills = |h: &egui_kittest::Harness<'_, RustGisApp>| {
            let s = &h.state().session;
            s.map().layer_by_name("Countries").unwrap().fill_layers.clone()
        };
        h.get_by_label("Add symbol layer").click();
        h.run_steps(3);
        assert_eq!(fills(&h).len(), 2, "the symbol's own fill and a hatch on it");
        assert!(h.query_by_label("Line Pattern Fill").is_some());
        h.get_by_label("Remove symbol layer").click();
        h.run_steps(3);
        assert!(fills(&h).is_empty(), "the symbol's own fill alone is no stack");
        let before = h.state().session.map().layer_by_name("Countries").unwrap().renderer.clone();
        h.get_by_label("hashed black X").click();
        h.run_steps(3);
        assert_eq!(fills(&h).len(), 2);
        h.state_mut().run("edit.undo", Value::Null).unwrap();
        h.run_steps(2);
        assert!(fills(&h).is_empty());
        assert_eq!(h.state().session.map().layer_by_name("Countries").unwrap().renderer, before, "one step undoes it all");
    }

    /// Paletted/Unique values' table: Delete All empties it, + adds a value, Classify fills it again.
    #[test]
    fn unique_values_table() {
        let mut app = qgis_app();
        let asc = "ncols 4\nnrows 2\nxllcorner 104\nyllcorner 11\ncellsize 0.1\nNODATA_value -9999\n10 10 20 20\n30 30 20 10\n";
        app.run("layer.addDataBytes", json!({"fileName": "lc.asc", "data": base64_of(asc.as_bytes())})).unwrap();
        app.run("raster.symbology", json!({"type": "unique"})).unwrap();
        let mut h = harness(app);
        let values = |h: &egui_kittest::Harness<'_, RustGisApp>| match &h
            .state()
            .session
            .map()
            .layer_by_name("lc")
            .unwrap()
            .raster
            .as_ref()
            .unwrap()
            .style
            .renderer
        {
            rustgis_engine::map::RasterRenderer::UniqueValues { values, .. } => values.iter().map(|v| v.0).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        assert_eq!(values(&h), [10.0, 20.0, 30.0]);
        assert!(h.query_by_label("Render type").is_some());
        h.get_by_label("Delete All").click();
        h.run_steps(3);
        assert!(values(&h).is_empty());
        h.get_by_label("Add a value").click();
        h.run_steps(3);
        assert_eq!(values(&h), [0.0]);
        h.get_by_label("Classify").click();
        h.run_steps(3);
        assert_eq!(values(&h), [10.0, 20.0, 30.0]);
    }

    fn base64_of(b: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(b)
    }
}
