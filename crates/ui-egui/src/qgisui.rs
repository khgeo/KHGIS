//! The QGIS-style layout, KHGIS's default: a menu bar (Project, Edit, View, Layer, Settings,
//! Plugins, Vector, Raster, Processing, Cambodia, Help), icon toolbars, the Browser and Layers
//! docked one above the other on the left, the Processing Toolbox on the right and the status
//! bar along the bottom. Settings › Interface switches back to the ribbon.
//!
//! Every menu entry and toolbar button runs one of KHGIS's commands, so the two layouts offer
//! the same work; the ribbon still opens below the toolbars while a print layout is open, for its
//! layout tools, or whenever View › Toolbars › Ribbon is on.
//!
//! Also here: Plugins › QGIS Plugins & Tools, which connects the user's own QGIS so its
//! Processing algorithms — QGIS's and those of installed QGIS plugins — join the toolbox
//! (`rustgis_qgis`).

use std::collections::BTreeMap;

use egui::{CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use rustgis_engine::Tool;
use rustgis_engine::analysis::{ParamKind, ToolSpec};
use serde_json::{Value, json};

use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{enabled, menu_item, menu_row, submenu_row};
use crate::{RightPane, RustGisApp, icons};

/// The preference naming the layout: `qgis` or `ribbon`.
pub const PREF: &str = "interface_style";
/// The preference naming the Python of the QGIS connected last.
pub const PYTHON_PREF: &str = "qgis_python";
/// The preference naming the web app's QGIS bridge service.
pub const SERVICE_PREF: &str = "qgis_service";

/// The toolbars, in the order they're drawn: (id, name).
pub const TOOLBARS: &[(&str, &str)] = &[
    ("project", "Project Toolbar"),
    ("navigation", "Map Navigation Toolbar"),
    ("attributes", "Attributes Toolbar"),
    ("digitizing", "Digitizing Toolbar"),
    ("data", "Data Source Manager Toolbar"),
    ("plugins", "Plugins Toolbar"),
];

/// The QGIS-style layout's own state.
pub struct State {
    /// The Browser dock shows (above the Layers dock).
    pub browser: bool,
    /// The Layers dock shows.
    pub layers: bool,
    /// Where each panel is docked, and how panels are tabbed and stacked.
    pub docks: crate::qgisdock::Docks,
    /// Toolbars hidden in View › Toolbars, by id.
    pub hidden: Vec<String>,
    /// The ribbon shows below the toolbars (View › Toolbars › Ribbon).
    pub ribbon: bool,
    /// Settings › Options… is open.
    pub options: bool,
    /// The Layer Styling panel's page: 0 Symbology, 1 Labels, 2 Information.
    pub styling_page: u8,
    /// Layer Styling's Live update: changes show on the map as they're made; off, they show
    /// when Apply is pressed.
    pub live: bool,
    /// With Live update off, the styled layer as the map last showed it: its id and a copy.
    pub staged: Option<(u32, Box<rustgis_engine::map::Layer>)>,
    /// Plugins › QGIS Plugins & Tools.
    pub bridge: Bridge,
    /// The project (`Session::opened`) whose maps were opened with editing off, as QGIS opens
    /// layers read-only until Toggle Editing.
    pub editing_off_for: Option<u64>,
    /// The toolbars' rows, as arranged by dragging (ids); their widths as last drawn; the one
    /// being dragged.
    pub rows: Vec<Vec<String>>,
    pub toolbar_widths: std::collections::HashMap<String, f32>,
    pub dragging: Option<String>,
    /// The Feature Attributes form (see `featureform`).
    pub form: crate::featureform::State,
}

impl Default for State {
    fn default() -> Self {
        State {
            browser: true,
            layers: true,
            docks: Default::default(),
            hidden: Vec::new(),
            ribbon: false,
            options: false,
            styling_page: 0,
            live: true,
            staged: None,
            bridge: Bridge::default(),
            editing_off_for: None,
            rows: Vec::new(),
            toolbar_widths: Default::default(),
            dragging: None,
            form: Default::default(),
        }
    }
}

/// The QGIS Plugins & Tools window.
#[derive(Default)]
pub struct Bridge {
    pub open: bool,
    /// The Python typed or picked (empty: look in the usual places).
    pub python: String,
    /// The web app's bridge service address.
    pub service: String,
    /// A QGIS plugin .zip to install.
    pub zip: String,
    /// Work running on a thread of its own: what it does, and its answer once it has one.
    busy: Option<(String, std::sync::Arc<std::sync::Mutex<Option<Result<Value, String>>>>)>,
    /// The last answer: connected, installed, or what went wrong.
    last: Option<Result<String, String>>,
}

/// Reads the layout's preferences: the QGIS layout unless the user picked the ribbon (or the
/// shell keeps the ribbon by default, as tests do).
pub fn load(app: &mut RustGisApp) {
    let load = |k: &str| app.services.load_pref.as_ref().and_then(|l| l(k));
    let style = load(PREF);
    app.ui.qgis_layout = match style.as_deref() {
        Some("ribbon") => false,
        Some("qgis") => true,
        _ => app.services.qgis_layout_default,
    };
    if let Some(h) = load("qgis_toolbars_hidden") {
        app.ui.qgis.hidden = h.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    }
    app.ui.qgis.rows = rows_from_pref(&load("qgis_toolbar_rows").unwrap_or_default());
    if let Some(p) = load(PYTHON_PREF).filter(|p| !p.is_empty()) {
        app.ui.qgis.bridge.python = p.clone();
        #[cfg(not(target_arch = "wasm32"))]
        rustgis_qgis::runner::choose(Some(std::path::PathBuf::from(p)));
    }
    if let Some(s) = load(SERVICE_PREF) {
        // The browser keeps no tool list between visits: the service connected last is
        // connected again.
        if !s.is_empty()
            && let Some(connect) = &app.services.qgis_connect
        {
            let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
            connect(s.clone(), slot.clone());
            app.ui.qgis.bridge.busy = Some(("connect".into(), slot));
        }
        app.ui.qgis.bridge.service = s;
    }
    // As set up for teaching: Layer Styling docked on the right (the Browser and the Layers are
    // tabbed together on the left, see `qgisdock`).
    if app.ui.qgis_layout && app.ui.right.is_none() {
        // LEE RS Lab is a tab beside it, a default tool for the remote sensing class.
        app.ui.open_right(RightPane::RsLab);
        app.ui.open_right(RightPane::Symbology);
    }
    app.ui.qgis.live = load("qgis_style_live").as_deref() != Some("off");
    crate::featureform::load(app);
}

/// `ui.interface {"style": "qgis" | "ribbon"}`: switches the layout (no style: the other one).
pub fn switch(app: &mut RustGisApp, p: &Value) -> Result<Value, String> {
    let qgis = match p.get("style").and_then(Value::as_str) {
        Some("qgis") => true,
        Some("ribbon") => false,
        None => !app.ui.qgis_layout,
        Some(other) => return Err(format!("unknown interface style '{other}': use qgis or ribbon")),
    };
    app.ui.qgis_layout = qgis;
    if let Some(save) = &app.services.save_pref {
        save(PREF, if qgis { "qgis" } else { "ribbon" });
    }
    Ok(json!({"style": if qgis { "qgis" } else { "ribbon" }}))
}

/// `ui.qgisToolbar {"id"}`: shows or hides a toolbar; `{"id": "ribbon"}` the ribbon.
pub fn toggle_toolbar(app: &mut RustGisApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_str).ok_or("id is required")?;
    if id == "ribbon" {
        app.ui.qgis.ribbon = !app.ui.qgis.ribbon;
        return Ok(Value::Null);
    }
    if !TOOLBARS.iter().any(|(t, _)| *t == id) {
        return Err(format!("no toolbar '{id}'"));
    }
    let hidden = &mut app.ui.qgis.hidden;
    if hidden.iter().any(|h| h == id) {
        hidden.retain(|h| h != id);
    } else {
        hidden.push(id.to_string());
    }
    if let Some(save) = &app.services.save_pref {
        save("qgis_toolbars_hidden", &app.ui.qgis.hidden.join(","));
    }
    Ok(Value::Null)
}

/// The top of the window: the header (when it is the native title bar), the menu bar, the
/// toolbars, and the ribbon when it shows.
pub fn top(app: &mut RustGisApp, ui: &mut Ui) {
    // On macOS the header is the window's title bar: it stays, with the menus below it.
    if app.ui.titlebar.is_some() {
        crate::chrome::header(app, ui);
    }
    // A print layout open: its own menus and toolbars, as QGIS's Layout window has them.
    if app.session.active_layout.is_some() {
        crate::qgislayout::menu_bar(app, ui);
        crate::qgislayout::toolbars(app, ui);
    } else {
        menu_bar(app, ui);
        toolbars(app, ui);
    }
    if app.ui.qgis.ribbon {
        crate::ribbon::show(app, ui);
    }
}

fn menu_bar(app: &mut RustGisApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("qgis_menu_bar")
        .exact_size(28.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.header).inner_margin(egui::Margin { left: 6, right: 8, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            // The locator on the right: Command Search, as QGIS's search bar.
            let w = 230.0_f32.min(full.width() * 0.3).max(26.0);
            let r = Rect::from_min_max(pos2(full.max.x - w, full.center().y - 10.0), pos2(full.max.x, full.center().y + 10.0));
            let resp = ui.interact(r, egui::Id::unique("qgis_locator"), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::same(4), if resp.hovered() { t.hover } else { t.input });
            ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(r.min.x + 11.0, r.center().y), vec2(12.0, 12.0)),
                "search",
                t.text_dim,
                t.accent,
            );
            if w > 80.0 {
                let hint = if ui.ctx().os().is_mac() { "⌘K" } else { "Ctrl+K" };
                let label = format!("{}  ({hint})", tl!("Command Search"));
                ui.painter().with_clip_rect(r.shrink(2.0)).text(
                    pos2(r.min.x + 22.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    regular(11.5),
                    t.text_dim,
                );
            }
            if resp.on_hover_text(tl!("Command Search")).clicked() {
                app.ui.command_search = Some(String::new());
            }
            // The language switch, beside it: ខ្មែរ | EN.
            let sw = 96.0;
            let lr = Rect::from_min_max(pos2(r.min.x - 10.0 - sw, full.center().y - 11.0), pos2(r.min.x - 10.0, full.center().y + 11.0));
            if full.width() > 520.0 {
                lang_switch(app, ui, lr, &t);
            }
            let left = Rect::from_min_max(full.min, pos2(if full.width() > 520.0 { lr.min.x } else { r.min.x } - 8.0, full.max.y));
            let mut menus = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::left_to_right(egui::Align::Center)));
            menus.set_clip_rect(left);
            if app.ui.titlebar.is_none() {
                let (lr, _) = menus.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                app.paint_logo(&menus, lr);
                menus.add_space(2.0);
            }
            egui::MenuBar::new().ui(&mut menus, |ui| {
                ui.menu_button(tl!("Project"), |ui| project_menu(app, ui));
                ui.menu_button(tl!("Edit"), |ui| edit_menu(app, ui));
                ui.menu_button(tl!("View"), |ui| view_menu(app, ui));
                ui.menu_button(tl!("Layer"), |ui| layer_menu(app, ui));
                ui.menu_button(tl!("Settings"), |ui| settings_menu(app, ui));
                ui.menu_button(tl!("Plugins"), |ui| plugins_menu(app, ui));
                ui.menu_button(tl!("Vector"), |ui| tools_menu(app, ui, false));
                ui.menu_button(tl!("Raster"), |ui| tools_menu(app, ui, true));
                ui.menu_button(tl!("Web"), |ui| web_menu(app, ui));
                ui.menu_button(tl!("Processing"), |ui| processing_menu(app, ui));
                ui.menu_button(tl!("Cambodia"), |ui| cambodia_menu(app, ui));
                ui.menu_button(tl!("Help"), |ui| help_menu(app, ui));
            });
            ui.advance_cursor_after_rect(full);
        });
}

/// The ខ្មែរ | EN switch: the interface language in one click.
fn lang_switch(app: &mut RustGisApp, ui: &mut Ui, r: Rect, t: &Tokens) {
    let current = crate::i18n::current().code();
    ui.painter().rect_filled(r, CornerRadius::same(11), t.input);
    ui.painter().rect_stroke(r, CornerRadius::same(11), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let half = r.width() / 2.0;
    for (i, (code, label, tip)) in [("km", "ខ្មែរ", "ប្ដូរទៅភាសាខ្មែរ"), ("en", "EN", "Switch to English")].into_iter().enumerate()
    {
        let part = Rect::from_min_size(pos2(r.min.x + half * i as f32, r.min.y), vec2(half, r.height())).shrink(2.0);
        let resp = ui.interact(part, egui::Id::unique(("qgis_lang", code)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, current == code, label));
        let on = current == code;
        if on {
            ui.painter().rect_filled(part, CornerRadius::same(9), t.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(part, CornerRadius::same(9), t.hover);
        }
        let font = if code == "km" { regular(12.5) } else { semibold(11.5) };
        // Khmer letters need the Khmer font, which loads after the first frames.
        let label = if code == "km" && !ui.fonts_mut(|f| f.has_glyphs(&font, label)) { "KH" } else { label };
        ui.painter().text(part.center(), egui::Align2::CENTER_CENTER, label, font, if on { t.on_accent } else { t.text_dim });
        if resp.on_hover_text(tip).clicked() && !on {
            let _ = app.run_by_user("ui.language", json!({"code": code}));
        }
    }
}

/// A menu row that sets the map tool, checked while it is the tool.
fn tool_item(ui: &mut Ui, app: &mut RustGisApp, icon: &str, label: &str, tool: Tool) {
    let on = app.session.tool == tool;
    let shortcut = crate::keymap::shortcut_text(ui.ctx(), app, "tool.set", &json!({"tool": tool.code()}));
    let allowed = !tool.edits() || app.session.map().editing.settings.enabled;
    let resp = menu_row(ui, icon, tl!(label), &shortcut, allowed);
    if on {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(
            resp.rect.expand2(vec2(4.0, 0.0)),
            CornerRadius::same(5),
            Stroke::new(1.0, t.accent),
            egui::StrokeKind::Inside,
        );
    }
    if resp.clicked() {
        app.map.cancel_sketch();
        let _ = app.run_by_user("tool.set", json!({"tool": tool.code()}));
        ui.close();
    }
}

/// A menu row with a check mark that runs `id` (a toggle).
fn check_item(ui: &mut Ui, app: &mut RustGisApp, on: bool, label: &str, id: &str, params: Value) {
    let mut checked = on;
    let resp = ui.add_enabled(enabled(app, id), egui::Checkbox::new(&mut checked, tl!(label)));
    if resp.clicked() {
        let _ = app.run_by_user(id, params);
        ui.close();
    }
}

fn project_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    menu_item(ui, app, "new", "New", "project.new", Value::Null);
    menu_item(ui, app, "open", "Open…", "ui.open", Value::Null);
    if app.session.project.maps.len() > 1 || !app.session.project.layouts.is_empty() {
        menu_item(ui, app, "catalog", "Open From Catalog", "ui.pane.catalog", Value::Null);
    }
    ui.separator();
    menu_item(ui, app, "save", "Save", "ui.save", Value::Null);
    menu_item(ui, app, "save_as", "Save As…", "ui.saveAs", Value::Null);
    menu_item(ui, app, "package", "Save As GeoPackage for QGIS…", "ui.saveGpkgQgis", Value::Null);
    ui.separator();
    submenu_row(ui, "export", tl!("Import/Export"), |ui| {
        menu_item(ui, app, "export_map", "Export Map to Image…", "ui.exportMap", Value::Null);
        menu_item(ui, app, "print", "Export Map to PDF…", "ui.printMap", Value::Null);
        menu_item(ui, app, "export_layer", "Export Layer…", "ui.exportLayer", Value::Null);
        menu_item(ui, app, "bookmark_import", "Import Bookmarks…", "ui.openItem", json!({"kind": "bookmarks"}));
        menu_item(ui, app, "bookmark_export", "Export Bookmarks…", "ui.saveItem", json!({"kind": "bookmarks"}));
    });
    ui.separator();
    menu_item(ui, app, "new_map", "New Map View", "map.new", Value::Null);
    menu_item(ui, app, "new_layout", "New Print Layout", "layout.new", json!({"template": "blank"}));
    menu_item(ui, app, "new_layout", "Cambodia A4 Map Layout", "khgis.autoLayout", json!({"lang": lang()}));
    ui.separator();
    menu_item(ui, app, "settings", "Properties… (Coordinate System)", "ui.dialog", json!({"name": "coordinateSystem"}));
    menu_item(ui, app, "goto_xy", "Snapping Options", "edit.snapping", Value::Null);
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.separator();
        if menu_row(ui, "close", tl!("Exit KHGIS"), "", true).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            ui.close();
        }
    }
}

fn edit_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    menu_item(ui, app, "undo", "Undo", "edit.undo", Value::Null);
    menu_item(ui, app, "redo", "Redo", "edit.redo", Value::Null);
    ui.separator();
    menu_item(ui, app, "cut", "Cut Features", "ui.cut", Value::Null);
    menu_item(ui, app, "copy", "Copy Features", "ui.copy", Value::Null);
    menu_item(ui, app, "paste", "Paste Features", "edit.paste", Value::Null);
    menu_item(ui, app, "paste", "Paste Features as New Layer", "edit.pasteSpecial", Value::Null);
    menu_item(ui, app, "delete", "Delete Selected", "edit.delete", Value::Null);
    ui.separator();
    submenu_row(ui, "select", tl!("Select"), |ui| {
        tool_item(ui, app, "select", "Select Features by Area or Single Click", Tool::Select);
        tool_item(ui, app, "select", "Select Features by Polygon", Tool::SelectPolygon);
        tool_item(ui, app, "select", "Select Features by Freehand", Tool::SelectLasso);
        tool_item(ui, app, "select", "Select Features by Radius", Tool::SelectRadius);
        ui.separator();
        menu_item(ui, app, "select", "Select All Features", "select.all", Value::Null);
        menu_item(ui, app, "select", "Invert Feature Selection", "select.switch", Value::Null);
        menu_item(ui, app, "clear_sel", "Deselect Features from All Layers", "select.clear", Value::Null);
        menu_item(ui, app, "new", "New Layer From Selection", "select.layerFromSelection", Value::Null);
    });
    ui.separator();
    let editing = app.session.map().editing.settings.enabled;
    check_item(ui, app, editing, "Toggle Editing", "edit.enable", Value::Null);
    tool_item(ui, app, "point_tool", "Add Point Feature", Tool::CreatePoint);
    tool_item(ui, app, "line_tool", "Add Line Feature", Tool::CreateLine);
    tool_item(ui, app, "polygon_tool", "Add Polygon Feature", Tool::CreatePolygon);
    tool_item(ui, app, "vertices", "Vertex Tool", Tool::EditVertices);
    tool_item(ui, app, "split", "Split Features", Tool::CutPolygon);
    tool_item(ui, app, "edit", "Reshape Features", Tool::Reshape);
    menu_item(ui, app, "union", "Merge Selected Features", "edit.merge", Value::Null);
    menu_item(ui, app, "split", "Multipart to Singleparts", "edit.explode", Value::Null);
    menu_item(ui, app, "history", "Edit Status", "ui.pane.editing", Value::Null);
}

fn view_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(250.0);
    tool_item(ui, app, "explore", "Pan Map", Tool::Explore);
    tool_item(ui, app, "zoom_box", "Zoom In (Box)", Tool::ZoomIn);
    menu_item(ui, app, "zoom_in", "Zoom In", "map.zoomIn", Value::Null);
    menu_item(ui, app, "zoom_out", "Zoom Out", "map.zoomOut", Value::Null);
    ui.separator();
    menu_item(ui, app, "zoom_full", "Zoom Full", "map.zoomFull", Value::Null);
    menu_item(ui, app, "zoom_sel", "Zoom to Selection", "select.zoomTo", Value::Null);
    menu_item(ui, app, "zoom_full", "Zoom to Layer", "layer.zoomTo", Value::Null);
    menu_item(ui, app, "prev_extent", "Zoom Last", "map.prevExtent", Value::Null);
    menu_item(ui, app, "next_extent", "Zoom Next", "map.nextExtent", Value::Null);
    menu_item(ui, app, "goto_xy", "Go to Coordinates…", "ui.dialog", json!({"name": "goToXY"}));
    ui.separator();
    tool_item(ui, app, "measure", "Measure Line", Tool::Measure);
    tool_item(ui, app, "measure_area", "Measure Area", Tool::MeasureArea);
    ui.separator();
    submenu_row(ui, "bookmark", tl!("Spatial Bookmarks"), |ui| {
        menu_item(ui, app, "bookmark_new", "New Spatial Bookmark…", "ui.dialog", json!({"name": "bookmark"}));
        menu_item(ui, app, "bookmark", "Show Spatial Bookmarks", "ui.pane.bookmarks", Value::Null);
    });
    submenu_row(ui, "graticule", tl!("Decorations"), |ui| {
        let g = app.session.map().graticule;
        check_item(ui, app, g, "Grid (Graticule)", "map.graticule", Value::Null);
        menu_item(ui, app, "transparency", "Map Effects", "map.effects", Value::Null);
    });
    menu_item(ui, app, "globe_view", "3D Map View (Globe)", "map.projection", json!({"projection": "globe"}));
    menu_item(ui, app, "map", "2D Map View", "map.projection", json!({"projection": "mercator"}));
    menu_item(ui, app, "relief", "Open in Cambodia 3D (Landscape, Trees, Houses)", "ui.cambodia3d", json!({}));
    menu_item(ui, app, "time_slider", "Temporal Controller", "ui.timeSlider", Value::Null);
    ui.separator();
    submenu_row(ui, "pane", tl!("Panels"), |ui| panels(app, ui));
    submenu_row(ui, "pane", tl!("Toolbars"), |ui| {
        for (id, name) in TOOLBARS {
            let on = !app.ui.qgis.hidden.iter().any(|h| h == id);
            check_item(ui, app, on, name, "ui.qgisToolbar", json!({"id": id}));
        }
        ui.separator();
        let on = app.ui.qgis.ribbon;
        check_item(ui, app, on, "Ribbon (every KHGIS tool)", "ui.qgisToolbar", json!({"id": "ribbon"}));
    });
    ui.separator();
    menu_item(ui, app, "zoom_full", "Refresh", "map.reloadWebLayers", json!({}));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let full = ui.ctx().input(|i| i.viewport().fullscreen).unwrap_or(false);
        if menu_row(ui, if full { "single" } else { "" }, tl!("Toggle Full Screen Mode"), "F11", true).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            ui.close();
        }
    }
}

/// View › Panels: each dock, checked while it shows.
fn panels(app: &mut RustGisApp, ui: &mut Ui) {
    let browser = app.ui.qgis.browser;
    let layers = app.ui.qgis.layers;
    check_item(ui, app, layers, "Layers", "ui.pane.contents", Value::Null);
    check_item(ui, app, browser, "Browser", "ui.pane.catalog", Value::Null);
    let sources = app.ui.left == Some(crate::LeftPane::Sources);
    check_item(ui, app, sources, "Data Source Manager", "ui.pane.sources", Value::Null);
    ui.separator();
    for (pane, label, id) in [
        (RightPane::Geoprocessing, "Processing Toolbox", "ui.pane.geoprocessing"),
        (RightPane::Symbology, "Layer Styling", "ui.pane.symbology"),
        (RightPane::Labeling, "Labels", "ui.pane.labeling"),
        (RightPane::History, "Processing History", "ui.pane.history"),
        (RightPane::Bookmarks, "Spatial Bookmarks", "ui.pane.bookmarks"),
        (RightPane::RsLab, "LEE RS Lab", "ui.pane.rslab"),
        (RightPane::Script, "Python Console (Scripts)", "ui.pane.script"),
        (RightPane::Notebook, "Notebook", "ui.pane.notebook"),
        (RightPane::Sql, "SQL", "ui.pane.sql"),
        (RightPane::Details, "Layer Details", "ui.pane.details"),
        (RightPane::Editing, "Edits", "ui.pane.editing"),
        (RightPane::Directions, "Directions", "ui.pane.directions"),
        (RightPane::Assistant, "AI Assistant", "ui.pane.assistant"),
        (RightPane::DiagnosticLog, "Log Messages", "ui.pane.diagnosticLog"),
    ] {
        let on = app.ui.right_open.contains(&pane);
        check_item(ui, app, on, label, id, Value::Null);
    }
}

fn layer_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    menu_item(ui, app, "add_data", "Data Source Manager", "ui.pane.sources", Value::Null);
    submenu_row(ui, "add_data", tl!("Add Layer"), |ui| {
        menu_item(ui, app, "add_data", "Add Vector or Raster Layer…", "ui.addData", Value::Null);
        menu_item(ui, app, "table", "Add Delimited Text Layer…", "ui.addDelimited", Value::Null);
        menu_item(ui, app, "web_data", "Add Layer From URL…", "ui.addUrl", Value::Null);
        menu_item(ui, app, "web_map", "Add WMS/WMTS, WFS or ArcGIS Layer…", "ui.addService", Value::Null);
        menu_item(ui, app, "basemap", "Add XYZ Tile Layer…", "ui.dialog", json!({"name": "addTiles"}));
        menu_item(ui, app, "package", "Add GeoPackage Layer…", "ui.dialog", json!({"name": "gpkgAddLayer"}));
        menu_item(ui, app, "table", "Add Database Layer…", "ui.dialog", json!({"name": "addDatabase"}));
        menu_item(ui, app, "raster_layer", "Add Zarr Variable…", "ui.addZarr", Value::Null);
        menu_item(ui, app, "add_folder", "Add Folder…", "ui.addFolder", Value::Null);
    });
    menu_item(ui, app, "cambodia", "Add Cambodia Data…", "ui.dialog", json!({"name": "cambodia"}));
    ui.separator();
    menu_item(ui, app, "table", "Open Attribute Table", "table.open", Value::Null);
    let editing = app.session.map().editing.settings.enabled;
    check_item(ui, app, editing, "Toggle Editing", "edit.enable", Value::Null);
    menu_item(ui, app, "export_layer", "Save As…", "ui.exportLayer", Value::Null);
    menu_item(ui, app, "delete", "Remove Layer", "layer.remove", Value::Null);
    ui.separator();
    menu_item(ui, app, "calculate", "Field Calculator…", "ui.dialog", json!({"name": "calculateField"}));
    menu_item(ui, app, "field", "Add Field…", "ui.dialog", json!({"name": "addField"}));
    menu_item(ui, app, "symbology", "Layer Styling (Symbology)", "ui.pane.symbology", Value::Null);
    menu_item(ui, app, "label", "Labeling", "ui.pane.labeling", Value::Null);
    menu_item(ui, app, "about", "Layer Properties (Details)", "ui.pane.details", Value::Null);
    menu_item(ui, app, "zoom_full", "Zoom to Layer", "layer.zoomTo", Value::Null);
}

fn settings_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(250.0);
    submenu_row(ui, "pane", tl!("Interface"), |ui| {
        let q = app.ui.qgis_layout;
        check_item(ui, app, q, "QGIS Layout (menus and toolbars)", "ui.interface", json!({"style": "qgis"}));
        check_item(ui, app, !q, "Ribbon Layout", "ui.interface", json!({"style": "ribbon"}));
    });
    submenu_row(ui, "globe_lang", tl!("Language"), |ui| {
        let current = app.chosen_lang().code();
        for (code, name) in [("en", "English"), ("km", "ខ្មែរ (Khmer)")] {
            let resp = menu_row(ui, if current == code { "single" } else { "" }, name, "", true);
            if resp.clicked() {
                let _ = app.run_by_user("ui.language", json!({"code": code}));
                ui.close();
            }
        }
        ui.separator();
        menu_item(ui, app, "globe_lang", "Other Languages…", "ui.qgisOptions", Value::Null);
    });
    submenu_row(ui, "dark", tl!("Theme"), |ui| {
        let dark = Tokens::get(ui.ctx()).dark;
        check_item(ui, app, !dark, "Light", "ui.theme", json!({"dark": false}));
        check_item(ui, app, dark, "Dark", "ui.theme", json!({"dark": true}));
    });
    let suppress = app.ui.qgis.form.suppress;
    if menu_row(ui, if suppress { "single" } else { "" }, tl!("Suppress Attribute Form After Feature Creation"), "", true)
        .on_hover_text(tl!("Off: drawing a feature opens a form to type its attributes (the class of a training area…)"))
        .clicked()
    {
        crate::featureform::set_suppress(app, !suppress);
        ui.close();
    }
    menu_item(ui, app, "command", "Keyboard Shortcuts…", "ui.customizeShortcuts", Value::Null);
    ui.separator();
    menu_item(ui, app, "settings", "Options…", "ui.qgisOptions", Value::Null);
}

fn plugins_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    menu_item(ui, app, "plugin", "Manage and Install Plugins…", "ui.plugins", json!({"open": true}));
    menu_item(ui, app, "plugin_add", "Install Plugin From File…", "ui.installPlugin", Value::Null);
    ui.separator();
    menu_item(ui, app, "toolbox", "QGIS Plugins & Tools…", "ui.qgisBridge", Value::Null);
    menu_item(ui, app, "python", "Python Console (Scripts)", "ui.pane.script", Value::Null);
    // Each enabled plugin: its panel, commands and tools.
    let lang = app.lang().code();
    let plugins: Vec<_> = rustgis_engine::plugins::list()
        .into_iter()
        .filter(|p| p.enabled && (p.manifest.panel.is_some() || !p.manifest.commands.is_empty() || !p.manifest.tools.is_empty()))
        .collect();
    if !plugins.is_empty() {
        ui.separator();
    }
    for p in plugins {
        let m = p.manifest.clone();
        let name = m.translate(lang, &m.name).to_string();
        submenu_row(ui, "plugin", &name, |ui| {
            if let Some(d) = &m.panel {
                menu_item(ui, app, "pane_right", m.translate(lang, &d.title), "ui.pane.plugin", json!({"id": m.id}));
            }
            for c in &m.commands {
                let icon = if icons::ALL.contains(&c.icon.as_str()) { c.icon.as_str() } else { "plugin" };
                let id = rustgis_engine::plugins::command_id(&m.id, &c.id);
                menu_item(ui, app, icon, &m.translate(lang, &c.label).replace('\n', " "), &id, Value::Null);
            }
            for tool in &m.tools {
                let id = rustgis_engine::plugins::command_id(&m.id, &tool.id);
                menu_item(ui, app, "toolbox", m.translate(lang, &tool.name), "ui.gp.open", json!({"tool": id}));
            }
        });
    }
}

/// Whether a tool works on rasters (its menu is Raster) rather than vectors (Vector).
fn is_raster_tool(t: &ToolSpec) -> bool {
    t.params.iter().any(|p| matches!(p.kind, ParamKind::RasterLayer | ParamKind::RasterLayers))
        || t.toolbox.contains("Raster")
        || t.toolbox.contains("GDAL")
}

/// The Vector or Raster menu: KHGIS's tools, then QGIS's (when QGIS is connected), as submenus
/// by toolbox, as QGIS groups its Vector and Raster menus.
fn tools_menu(app: &mut RustGisApp, ui: &mut Ui, raster: bool) {
    ui.set_min_width(240.0);
    let (mut own, mut qgis): (BTreeMap<String, Vec<&'static ToolSpec>>, BTreeMap<String, Vec<&'static ToolSpec>>) = Default::default();
    for t in rustgis_engine::analysis::tools() {
        if is_raster_tool(t) != raster || t.toolbox.starts_with("Test") {
            continue;
        }
        if let Some(rest) = t.toolbox.strip_prefix("QGIS › ") {
            qgis.entry(rest.to_string()).or_default().push(t);
        } else if t.toolbox.starts_with("GeoLibre") {
            continue;
        } else {
            own.entry(t.toolbox.to_string()).or_default().push(t);
        }
    }
    if own.is_empty() && qgis.is_empty() {
        ui.label(egui::RichText::new(tl!("No tools")).weak());
    }
    if raster {
        menu_item(ui, app, "rgb", "LEE RS Lab…", "ui.pane.rslab", Value::Null);
        ui.separator();
    }
    tree(app, ui, &own);
    if !qgis.is_empty() {
        ui.separator();
        submenu_row(ui, "toolbox", tl!("QGIS Processing"), |ui| tree(app, ui, &qgis));
    } else if !raster {
        ui.separator();
        menu_item(ui, app, "toolbox", "Connect QGIS for its Algorithms…", "ui.qgisBridge", Value::Null);
    }
    ui.separator();
    menu_item(ui, app, "toolbox", "Processing Toolbox", "ui.pane.geoprocessing", Value::Null);
}

/// Tools grouped by `A › B` toolbox paths, as nested submenus.
fn tree(app: &mut RustGisApp, ui: &mut Ui, groups: &BTreeMap<String, Vec<&'static ToolSpec>>) {
    let mut top: BTreeMap<&str, Vec<(&str, &Vec<&'static ToolSpec>)>> = BTreeMap::new();
    for (path, tools) in groups {
        let (a, b) = path.split_once(" › ").unwrap_or((path.as_str(), ""));
        top.entry(a).or_default().push((b, tools));
    }
    for (a, subs) in top {
        submenu_row(ui, "toolbox", tl!(a), |ui| {
            for (b, tools) in subs {
                let mut items = |ui: &mut Ui| {
                    egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                        for t in tools.iter() {
                            menu_item(ui, app, "tools", t.name, "ui.gp.open", json!({"tool": t.id}));
                        }
                    });
                };
                if b.is_empty() {
                    items(ui);
                } else {
                    submenu_row(ui, "toolbox", tl!(b), items);
                }
            }
        });
    }
}

fn web_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    menu_item(ui, app, "basemap", "Basemaps and XYZ Tiles…", "ui.dialog", json!({"name": "addTiles"}));
    menu_item(ui, app, "web_map", "WMS/WMTS, WFS and ArcGIS Services…", "ui.addService", Value::Null);
    menu_item(ui, app, "search", "Search Open Data…", "ui.searchOpenData", Value::Null);
    submenu_row(ui, "planet", tl!("Satellite Imagery (STAC)"), |ui| {
        menu_item(ui, app, "planet", "Sentinel-2 Explorer…", "ui.stac.sentinel2", Value::Null);
        menu_item(ui, app, "planet", "Planet Open Data…", "ui.stac.planet", Value::Null);
        menu_item(ui, app, "planet", "Portolan…", "ui.stac.portolan", Value::Null);
        menu_item(ui, app, "planet", "Vantor…", "ui.stac.vantor", Value::Null);
    });
    menu_item(ui, app, "web_data", "OpenStreetMap…", "ui.dialog", json!({"name": "osm"}));
    menu_item(ui, app, "web_data", "Administrative Boundaries…", "ui.dialog", json!({"name": "boundaries"}));
    menu_item(ui, app, "directions", "Directions", "ui.pane.directions", Value::Null);
    menu_item(ui, app, "share", "Share…", "ui.share", Value::Null);
}

fn processing_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    menu_item(ui, app, "toolbox", "Toolbox", "ui.pane.geoprocessing", Value::Null);
    menu_item(ui, app, "modelbuilder", "Graphical Modeler…", "ui.modelBuilder", Value::Null);
    menu_item(ui, app, "history", "History…", "ui.pane.history", Value::Null);
    menu_item(ui, app, "python", "Scripts", "ui.pane.script", Value::Null);
    menu_item(ui, app, "run", "Tasks", "ui.pane.tasks", Value::Null);
    ui.separator();
    menu_item(ui, app, "toolbox", "QGIS Plugins & Tools…", "ui.qgisBridge", Value::Null);
}

fn cambodia_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    menu_item(ui, app, "cambodia", "Load a Province or District…", "ui.dialog", json!({"name": "cambodia"}));
    menu_item(ui, app, "new_layout", "A4 Map Layout (Khmer or English)", "khgis.autoLayout", json!({"lang": lang()}));
    menu_item(ui, app, "rgb", "LEE RS Lab (Satellite Imagery)", "ui.pane.rslab", Value::Null);
    menu_item(ui, app, "relief", "Open in Cambodia 3D", "ui.cambodia3d", json!({}));
    ui.separator();
    for (id, label) in [
        ("floodExposure", "Villages Exposed to Floods"),
        ("schoolAccess", "Villages Far From a School"),
        ("healthAccess", "Villages Far From Health Care"),
        ("districtSchools", "Schools per District"),
    ] {
        menu_item(ui, app, "run", label, "khgis.recipe", json!({"id": id, "distance": 3, "lang": lang()}));
    }
}

fn help_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(220.0);
    menu_item(ui, app, "help", "User Guide", "ui.learn", Value::Null);
    menu_item(ui, app, "command", "Keyboard Shortcuts", "ui.shortcuts", Value::Null);
    menu_item(ui, app, "message_log", "Log Messages", "ui.pane.diagnosticLog", Value::Null);
    #[cfg(not(target_arch = "wasm32"))]
    menu_item(ui, app, "export", "Check for Updates…", "ui.checkUpdate", Value::Null);
    ui.separator();
    menu_item(ui, app, "about", "About KHGIS", "ui.about", Value::Null);
}

fn lang() -> &'static str {
    if crate::i18n::current().code() == "km" { "km" } else { "en" }
}

/// A toolbar button: the icon, checked or disabled as the command is, its name on hover.
fn button(ui: &mut Ui, app: &mut RustGisApp, icon: &str, tip: &str, id: &str, params: Value, checked: bool) {
    button_when(ui, app, icon, tip, id, params, checked, true);
}

/// [`button`], greyed out unless `allowed` too.
#[allow(clippy::too_many_arguments)]
fn button_when(ui: &mut Ui, app: &mut RustGisApp, icon: &str, tip: &str, id: &str, params: Value, checked: bool, allowed: bool) {
    let t = Tokens::get(ui.ctx());
    let on = allowed && enabled(app, id);
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let tip = tl!(tip).to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, &tip));
    if checked {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.checked);
        ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if on && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), if resp.is_pointer_button_down_on() { t.pressed } else { t.hover });
    }
    icons::paint(ui.painter(), r.shrink(4.0), icon, if on { t.icon } else { t.text_disabled }, t.accent);
    let shortcut = crate::keymap::shortcut_text(ui.ctx(), app, id, &params);
    let hover = if shortcut.is_empty() { tip } else { format!("{tip} ({shortcut})") };
    if resp.on_hover_text(hover).clicked() && on {
        if id == "tool.set" {
            app.map.cancel_sketch();
        }
        let _ = app.run_by_user(id, params);
    }
}

fn tool_button(ui: &mut Ui, app: &mut RustGisApp, icon: &str, tip: &str, tool: Tool) {
    let on = app.session.tool == tool;
    // Digitizing tools wait for Toggle Editing, as in QGIS.
    let allowed = !tool.edits() || app.session.map().editing.settings.enabled;
    button_when(ui, app, icon, tip, "tool.set", json!({"tool": tool.code()}), on, allowed);
}

/// A project just opened (or the first at start) opens with editing off, as QGIS's layers do:
/// Toggle Editing turns it on. Not a change to the project, so it isn't marked as one.
fn editing_off_at_open(app: &mut RustGisApp) {
    if app.ui.qgis.editing_off_for == Some(app.session.opened) {
        return;
    }
    app.ui.qgis.editing_off_for = Some(app.session.opened);
    for m in &mut app.session.project.maps {
        m.editing.settings.enabled = false;
    }
    if app.session.tool.edits() {
        app.session.tool = Tool::default();
    }
}

/// A toolbar's buttons.
fn contents(ui: &mut Ui, app: &mut RustGisApp, id: &str) {
    match id {
        "project" => {
        button(ui, app, "new", "New Project", "project.new", Value::Null, false);
        button(ui, app, "open", "Open Project", "ui.open", Value::Null, false);
        button(ui, app, "save", "Save Project", "ui.save", Value::Null, false);
        button(ui, app, "save_as", "Save Project As…", "ui.saveAs", Value::Null, false);
        button(ui, app, "new_layout", "New Print Layout", "layout.new", json!({"template": "blank"}), false);
        button(ui, app, "print", "Export Map to PDF", "ui.printMap", Value::Null, false);
        }
        "navigation" => {
        tool_button(ui, app, "explore", "Pan Map (click a feature to identify it)", Tool::Explore);
        tool_button(ui, app, "zoom_box", "Zoom In (drag a box)", Tool::ZoomIn);
        button(ui, app, "zoom_in", "Zoom In", "map.zoomIn", Value::Null, false);
        button(ui, app, "zoom_out", "Zoom Out", "map.zoomOut", Value::Null, false);
        button(ui, app, "zoom_full", "Zoom Full", "map.zoomFull", Value::Null, false);
        button(ui, app, "zoom_sel", "Zoom to Selection", "select.zoomTo", Value::Null, false);
        button(ui, app, "zoom_world", "Zoom to Layer", "layer.zoomTo", Value::Null, false);
        button(ui, app, "prev_extent", "Zoom Last", "map.prevExtent", Value::Null, false);
        button(ui, app, "next_extent", "Zoom Next", "map.nextExtent", Value::Null, false);
        button(ui, app, "bookmark_new", "New Spatial Bookmark…", "ui.dialog", json!({"name": "bookmark"}), false);
        let b = app.ui.right == Some(RightPane::Bookmarks);
        button(ui, app, "bookmark", "Show Spatial Bookmarks", "ui.pane.bookmarks", Value::Null, b);
        }
        "attributes" => {
        tool_button(ui, app, "select", "Select Features by Area or Single Click", Tool::Select);
        tool_button(ui, app, "select_loc", "Select Features by Polygon", Tool::SelectPolygon);
        button(ui, app, "clear_sel", "Deselect Features from All Layers", "select.clear", Value::Null, false);
        button(ui, app, "table", "Open Attribute Table", "table.open", Value::Null, false);
        button(ui, app, "calculate", "Open Field Calculator", "ui.dialog", json!({"name": "calculateField"}), false);
        tool_button(ui, app, "measure", "Measure Line", Tool::Measure);
        tool_button(ui, app, "measure_area", "Measure Area", Tool::MeasureArea);
        let s = app.ui.right == Some(RightPane::Symbology);
        button(ui, app, "symbology", "Layer Styling", "ui.pane.symbology", Value::Null, s);
        let l = app.ui.right == Some(RightPane::Labeling);
        button(ui, app, "label", "Labels", "ui.pane.labeling", Value::Null, l);
        }
        "digitizing" => {
        let editing = app.session.map().editing.settings.enabled;
        button(ui, app, "edit", "Toggle Editing", "edit.enable", Value::Null, editing);
        tool_button(ui, app, "point_tool", "Add Point Feature", Tool::CreatePoint);
        tool_button(ui, app, "line_tool", "Add Line Feature", Tool::CreateLine);
        tool_button(ui, app, "polygon_tool", "Add Polygon Feature", Tool::CreatePolygon);
        tool_button(ui, app, "vertices", "Vertex Tool", Tool::EditVertices);
        button(ui, app, "delete", "Delete Selected", "edit.delete", Value::Null, false);
        button(ui, app, "cut", "Cut Features", "ui.cut", Value::Null, false);
        button(ui, app, "copy", "Copy Features", "ui.copy", Value::Null, false);
        button(ui, app, "paste", "Paste Features", "edit.paste", Value::Null, false);
        button(ui, app, "undo", "Undo", "edit.undo", Value::Null, false);
        button(ui, app, "redo", "Redo", "edit.redo", Value::Null, false);
        }
        "data" => {
        let s = app.ui.left == Some(crate::LeftPane::Sources);
        button(ui, app, "add_data", "Open Data Source Manager", "ui.pane.sources", Value::Null, s);
        button(ui, app, "point_layer", "Add Vector or Raster Layer", "ui.addData", Value::Null, false);
        button(ui, app, "table", "Add Delimited Text Layer", "ui.addDelimited", Value::Null, false);
        button(ui, app, "web_map", "Add WMS/WMTS, WFS or ArcGIS Layer", "ui.addService", Value::Null, false);
        button(ui, app, "basemap", "Add XYZ Tile Layer", "ui.dialog", json!({"name": "addTiles"}), false);
        button(ui, app, "cambodia", "Add Cambodia Data", "ui.dialog", json!({"name": "cambodia"}), false);
        let lab = app.ui.right == Some(RightPane::RsLab);
        button(ui, app, "rgb", "LEE RS Lab", "ui.pane.rslab", Value::Null, lab);
        button(ui, app, "relief", "Open in Cambodia 3D", "ui.cambodia3d", json!({}), false);
        }
        "plugins" => {
        let tb = app.ui.right == Some(RightPane::Geoprocessing);
        button(ui, app, "toolbox", "Processing Toolbox", "ui.pane.geoprocessing", Value::Null, tb);
        button(ui, app, "modelbuilder", "Graphical Modeler", "ui.modelBuilder", Value::Null, app.mb.shown());
        let py = app.ui.right == Some(RightPane::Script);
        button(ui, app, "python", "Python Console (Scripts)", "ui.pane.script", Value::Null, py);
        button(ui, app, "plugin", "Manage and Install Plugins", "ui.plugins", json!({"open": true}), false);
        let q = app.ui.qgis.bridge.open;
        button(ui, app, "geoprocessing", "QGIS Plugins & Tools", "ui.qgisBridge", Value::Null, q);
        let ai = app.ui.right == Some(RightPane::Assistant);
        button(ui, app, "command", "AI Assistant", "ui.pane.assistant", Value::Null, ai);
        }
        _ => {}
    }
}

/// The toolbars' rows, as the user arranged them: every toolbar once, in rows (none empty).
fn arranged(rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Vec<String>> = Vec::new();
    for row in rows {
        let mut r: Vec<String> = Vec::new();
        for id in row {
            if TOOLBARS.iter().any(|(t, _)| t == id) && !seen.contains(id) {
                seen.push(id.clone());
                r.push(id.clone());
            }
        }
        if !r.is_empty() {
            out.push(r);
        }
    }
    let missing: Vec<String> = TOOLBARS.iter().filter(|(t, _)| !seen.iter().any(|x| x == t)).map(|(t, _)| t.to_string()).collect();
    if !missing.is_empty() {
        match out.last_mut() {
            Some(last) => last.extend(missing),
            None => out.push(missing),
        }
    }
    out
}

/// The rows as a preference: toolbars by `,`, rows by `;`.
fn rows_pref(rows: &[Vec<String>]) -> String {
    rows.iter().map(|r| r.join(",")).collect::<Vec<_>>().join(";")
}

fn rows_from_pref(text: &str) -> Vec<Vec<String>> {
    let rows: Vec<Vec<String>> = text.split(';').map(|r| r.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).collect();
    arranged(&rows)
}

/// Moves a toolbar: into row `row` before its `at`-th toolbar (both counted with the toolbar
/// still where it was), or into a row of its own put at `row` when `own_row`.
fn move_toolbar(rows: &mut Vec<Vec<String>>, id: &str, row: usize, at: usize, own_row: bool) {
    let (mut row, mut at) = (row, at);
    if let Some((r, i)) = rows.iter().enumerate().find_map(|(r, v)| v.iter().position(|x| x == id).map(|i| (r, i))) {
        if !own_row && r == row && i < at {
            at -= 1;
        }
        if let Some(v) = rows.get_mut(r) {
            v.remove(i);
        }
        if rows.get(r).is_some_and(Vec::is_empty) {
            rows.remove(r);
            if r < row {
                row -= 1;
            } else if r == row && !own_row {
                // Its own row, now gone: back where it was, a row of its own again.
                rows.insert(r.min(rows.len()), vec![id.to_string()]);
                return;
            }
        }
    }
    if own_row || row >= rows.len() {
        rows.insert(row.min(rows.len()), vec![id.to_string()]);
    } else if let Some(v) = rows.get_mut(row) {
        v.insert(at.min(v.len()), id.to_string());
    }
}

/// The toolbars' menu (right-click on them): each one shown or hidden, and their places reset.
fn toolbars_menu(app: &mut RustGisApp, ui: &mut Ui) {
    for (id, name) in TOOLBARS {
        let mut on = !app.ui.qgis.hidden.iter().any(|h| h == id);
        if ui.checkbox(&mut on, tl!(*name)).clicked() {
            let _ = toggle_toolbar(app, &json!({"id": id}));
        }
    }
    ui.separator();
    if ui.button(tl!("Reset Toolbar Positions")).clicked() {
        app.ui.qgis.rows = arranged(&[]);
        save_pref(app, "qgis_toolbar_rows", "");
        ui.close();
    }
}

fn save_pref(app: &RustGisApp, key: &str, value: &str) {
    if let Some(s) = &app.services.save_pref {
        s(key, value);
    }
}

/// A toolbar's start: a grip, as QGIS draws one, to drag the toolbar by.
fn grip(ui: &mut Ui, id: &str, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(10.0, 24.0), Sense::click_and_drag());
    let colour = if resp.hovered() || resp.dragged() { t.accent } else { t.border_strong };
    for i in 0..4 {
        let y = r.min.y + 5.0 + i as f32 * 4.5;
        ui.painter().circle_filled(pos2(r.center().x - 1.5, y), 1.0, colour);
        ui.painter().circle_filled(pos2(r.center().x + 1.5, y), 1.0, colour);
    }
    let name = TOOLBARS.iter().find(|(x, _)| *x == id).map_or("", |(_, n)| n);
    let tip = crate::i18n::fmt("{name}: drag to move it", &[("name", tl!(name))]);
    resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tip)
}

/// One toolbar: its grip and buttons, wrapping onto more lines only when it's wider than the room
/// left. The toolbar's rect and its grip's answer.
fn toolbar(ui: &mut Ui, app: &mut RustGisApp, id: &str, room: f32) -> (Rect, egui::Response) {
    let t = Tokens::get(ui.ctx());
    let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
    let inner = ui.allocate_ui_with_layout(vec2(room.max(60.0), 24.0), layout, |ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 2.0);
        let g = grip(ui, id, &t);
        contents(ui, app, id);
        ui.add_space(6.0);
        g
    });
    (inner.response.rect, inner.inner)
}

/// The toolbars, as QGIS has them: in rows, each moved by dragging its grip (onto another row,
/// between two toolbars, or above or below the others for a row of its own). A row too long for
/// the window goes on over more lines, so no button is hidden. Right-click shows or hides them.
fn toolbars(app: &mut RustGisApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if TOOLBARS.iter().all(|(id, _)| app.ui.qgis.hidden.iter().any(|h| h == id)) {
        return;
    }
    if app.ui.qgis.rows.is_empty() {
        app.ui.qgis.rows = arranged(&[]);
    }
    egui::Panel::top("qgis_toolbars")
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 4, right: 4, top: 2, bottom: 2 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            // Right-click between the toolbars: the toolbars' menu (the buttons answer their own).
            ui.interact(full, ui.scope_id().with("toolbars_bg"), Sense::click()).context_menu(|ui| toolbars_menu(app, ui));
            let room = ui.available_width();
            let rows = app.ui.qgis.rows.clone();
            // The lines drawn: (row, its toolbars and their rects).
            let mut lines: Vec<(usize, Vec<(String, Rect)>)> = Vec::new();
            let mut released = false;
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (ri, row) in rows.iter().enumerate() {
                let shown: Vec<&String> = row.iter().filter(|id| !app.ui.qgis.hidden.contains(id)).collect();
                let mut k = 0;
                while k < shown.len() {
                    // As many as fit on this line (their widths as last drawn), at least one.
                    let mut used = 0.0;
                    let mut end = k;
                    while let Some(id) = shown.get(end) {
                        let w = app.ui.qgis.toolbar_widths.get(id.as_str()).copied().unwrap_or(200.0);
                        if end > k && used + w > room {
                            break;
                        }
                        used += w + 2.0;
                        end += 1;
                    }
                    let mut placed = Vec::new();
                    ui.horizontal(|ui| {
                        for id in shown.get(k..end).unwrap_or_default() {
                            let left = room - (ui.cursor().min.x - full.min.x);
                            let (r, g) = toolbar(ui, app, id, left);
                            app.ui.qgis.toolbar_widths.insert(id.to_string(), r.width());
                            if g.drag_started() {
                                app.ui.qgis.dragging = Some(id.to_string());
                            }
                            released |= g.drag_stopped();
                            g.context_menu(|ui| toolbars_menu(app, ui));
                            placed.push((id.to_string(), r));
                        }
                    });
                    lines.push((ri, placed));
                    k = end;
                }
            }
            let bottom = ui.min_rect().max.y + 2.0;
            ui.painter().line_segment([pos2(full.min.x - 4.0, bottom), pos2(full.max.x + 4.0, bottom)], Stroke::new(1.0, t.border));
            let Some(dragged) = app.ui.qgis.dragging.clone() else { return };
            let Some(at) = ui.ctx().pointer_latest_pos() else { return };
            let target = drop_target(&lines, at);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("toolbar_drag")));
            // Where it goes: a bar between toolbars, or a line above or below the rows for a row of
            // its own; the toolbar's name follows the pointer.
            match &target {
                Drop::Between { x, line, .. } => {
                    let r = lines.get(*line).map_or(full, |l| line_rect(&l.1));
                    painter.line_segment([pos2(*x, r.min.y), pos2(*x, r.max.y)], Stroke::new(3.0, t.accent));
                }
                Drop::OwnRow { y, .. } => {
                    painter.line_segment([pos2(full.min.x, *y), pos2(full.max.x, *y)], Stroke::new(3.0, t.accent));
                }
            }
            let name = TOOLBARS.iter().find(|(x, _)| *x == dragged).map_or("", |(_, n)| n);
            let galley = painter.layout_no_wrap(tl!(name).to_string(), egui::FontId::proportional(12.0), t.text);
            let tag = Rect::from_min_size(at + vec2(12.0, 10.0), galley.size() + vec2(12.0, 6.0));
            painter.rect(tag, 4.0, t.pane, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            painter.galley(tag.min + vec2(6.0, 3.0), galley, t.text);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if released || !ui.ctx().input(|i| i.pointer.any_down()) {
                app.ui.qgis.dragging = None;
                // The lines as drawn become the rows (as QGIS keeps toolbars where they were put),
                // so a drop lands where it shows; hidden toolbars keep a place in the last row.
                let mut rows: Vec<Vec<String>> = lines.iter().map(|l| l.1.iter().map(|(id, _)| id.clone()).collect()).collect();
                let hidden: Vec<String> = app.ui.qgis.rows.iter().flatten().filter(|id| app.ui.qgis.hidden.contains(id)).cloned().collect();
                if let Some(last) = rows.last_mut() {
                    last.extend(hidden);
                }
                let mut rows = arranged(&rows);
                match target {
                    Drop::Between { row, at, .. } => move_toolbar(&mut rows, &dragged, row, at, false),
                    Drop::OwnRow { row, .. } => move_toolbar(&mut rows, &dragged, row, 0, true),
                }
                save_pref(app, "qgis_toolbar_rows", &rows_pref(&rows));
                app.ui.qgis.rows = rows;
            }
        });
}

/// Where a dragged toolbar would go, counted in the lines drawn.
enum Drop {
    /// Into `row`, before its `at`-th toolbar (the bar drawn at `x` on drawn line `line`).
    Between { row: usize, at: usize, x: f32, line: usize },
    /// A row of its own, put at `row` (the line drawn at `y`).
    OwnRow { row: usize, y: f32 },
}

fn line_rect(placed: &[(String, Rect)]) -> Rect {
    placed.iter().fold(Rect::NOTHING, |a, (_, r)| a.union(*r))
}

fn drop_target(lines: &[(usize, Vec<(String, Rect)>)], at: egui::Pos2) -> Drop {
    if let Some(f) = lines.first().map(|l| line_rect(&l.1))
        && at.y < f.min.y - 2.0
    {
        return Drop::OwnRow { row: 0, y: f.min.y };
    }
    if let Some(l) = lines.last().map(|l| line_rect(&l.1))
        && at.y > l.max.y + 3.0
    {
        return Drop::OwnRow { row: lines.len(), y: l.max.y + 2.0 };
    }
    // The drawn line under the pointer (or nearest it).
    let gap = |r: Rect| {
        if at.y < r.min.y {
            r.min.y - at.y
        } else if at.y > r.max.y {
            at.y - r.max.y
        } else {
            0.0
        }
    };
    let li = lines.iter().enumerate().min_by(|a, b| gap(line_rect(&a.1.1)).total_cmp(&gap(line_rect(&b.1.1)))).map_or(0, |(i, _)| i);
    let Some((_, placed)) = lines.get(li) else { return Drop::OwnRow { row: 0, y: at.y } };
    // Its place on the line (the line drawn is the row it goes in): after those left of the pointer.
    let k = placed.iter().filter(|(_, r)| r.center().x < at.x).count();
    let x = match placed.get(k) {
        Some((_, r)) => r.min.x - 1.0,
        None => placed.last().map_or(at.x, |(_, r)| r.max.x + 1.0),
    };
    Drop::Between { row: li, at: k, x, line: li }
}

/// The docks on both sides of the map (see `qgisdock`): the rects of the left and right areas.
pub fn docks(app: &mut RustGisApp, ui: &mut Ui, frame: egui::Frame, t: &Tokens) -> [Option<Rect>; 2] {
    editing_off_at_open(app);
    crate::qgisdock::show(app, ui, frame, t)
}

/// The Layer Styling panel, as QGIS has it: the layer at the top, a strip of pages down the
/// left (Symbology, Labels, Information) and the page beside it.
pub fn layer_styling(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens) {
    // The layer, picked from the map's layers.
    use rustgis_engine::data::GeometryType;
    let layers: Vec<(u32, String, &'static str)> = app
        .session
        .map()
        .layers
        .iter()
        .map(|l| {
            let icon = if l.is_raster() {
                "raster_layer"
            } else {
                match l.geometry_type() {
                    GeometryType::Point => "point_layer",
                    GeometryType::Polyline => "line_layer",
                    _ => "polygon_layer",
                }
            };
            (l.id, l.name.clone(), icon)
        })
        .collect();
    let active = app.session.active_layer.and_then(|id| layers.iter().find(|l| l.0 == id)).cloned();
    egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            let icon = active.as_ref().map_or("contents", |l| l.2);
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            icons::paint(ui.painter(), r, icon, t.icon, t.accent);
            let shown = active.as_ref().map_or_else(|| tl!("Pick a layer").to_string(), |l| l.1.clone());
            let combo = egui::ComboBox::from_id_salt("qgis_styling_layer").width(ui.available_width() - 4.0).selected_text(shown).show_ui(
                ui,
                |ui| {
                    for (id, name, _) in &layers {
                        if ui.selectable_label(Some(*id) == active.as_ref().map(|l| l.0), name).clicked() {
                            let _ = app.run_by_user("layer.select", json!({"layer": name}));
                        }
                    }
                },
            );
            let name = tl!("Layer").to_string();
            combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        });
    });
    let sep_y = ui.cursor().min.y;
    ui.painter().line_segment([pos2(ui.max_rect().min.x, sep_y), pos2(ui.max_rect().max.x, sep_y)], Stroke::new(1.0, t.border));
    let all = ui.available_rect_before_wrap();
    // The foot: Live update, and Apply when it's off.
    let foot = Rect::from_min_max(pos2(all.min.x, all.max.y - 36.0), all.max);
    let rest = Rect::from_min_max(all.min, pos2(all.max.x, foot.min.y));
    styling_foot(app, ui, foot, active.as_ref().map(|l| (l.0, l.2 == "raster_layer")), t);
    // The page strip.
    let strip = Rect::from_min_size(rest.min, vec2(36.0, rest.height()));
    ui.painter().rect_filled(strip, 0.0, t.pane_header);
    ui.painter().line_segment([strip.right_top(), strip.right_bottom()], Stroke::new(1.0, t.border));
    for (i, (icon, tip)) in [("symbology", "Symbology"), ("label", "Labels"), ("about", "Information")].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(strip.min.x + 3.0, strip.min.y + 4.0 + 34.0 * i as f32), vec2(30.0, 30.0));
        let resp = ui.interact(r, egui::Id::unique(("qgis_styling_page", i)), Sense::click());
        let on = app.ui.qgis.styling_page as usize == i;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, tl!(tip)));
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(4), t.checked);
            ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
        }
        icons::paint(ui.painter(), r.shrink(5.0), icon, t.icon, t.accent);
        if resp.on_hover_text(tl!(tip)).clicked() {
            app.ui.qgis.styling_page = i as u8;
        }
    }
    // The page.
    let body = Rect::from_min_max(pos2(strip.max.x + 1.0, rest.min.y), rest.max);
    let mut page = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    page.set_clip_rect(body.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical().id_salt(("qgis_styling_scroll", app.ui.qgis.styling_page)).auto_shrink([false, false]).show(
        &mut page,
        |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| match app.ui.qgis.styling_page {
                1 => crate::labelpane::show(app, ui, t),
                2 => crate::panes::details_in(app, ui, t),
                _ => crate::panes::symbology_in(app, ui, t),
            });
        },
    );
    ui.advance_cursor_after_rect(all);
}

/// Layer Styling's foot: the Live update switch, and Apply while it's off.
fn styling_foot(app: &mut RustGisApp, ui: &mut Ui, foot: Rect, layer: Option<(u32, bool)>, t: &Tokens) {
    ui.painter().rect_filled(foot, 0.0, t.pane_header);
    ui.painter().line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.border));
    let mut bar = ui
        .new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    // Rasters always update live: holding their style back would copy their pixels.
    let raster = layer.is_some_and(|l| l.1);
    let mut live = app.ui.qgis.live || raster;
    let resp = bar
        .add_enabled(!raster, egui::Checkbox::new(&mut live, tl!("Live update")))
        .on_hover_text(tl!("On: the map shows each change at once. Off: change the style, then press Apply."))
        .on_disabled_hover_text(tl!("A raster's style always shows at once"));
    if resp.changed() {
        app.ui.qgis.live = live;
        if live {
            // Everything changed so far shows now.
            app.ui.qgis.staged = None;
            app.map.invalidate();
        }
        if let Some(save) = &app.services.save_pref {
            save("qgis_style_live", if live { "on" } else { "off" });
        }
    }
    // Live update off: the layer as the map shows it is kept, to draw the map with until Apply.
    if !app.ui.qgis.live && !raster {
        match (layer, &app.ui.qgis.staged) {
            (Some((id, _)), Some((staged, _))) if *staged == id => {}
            (Some((id, _)), _) => {
                app.ui.qgis.staged = app.session.map().layer(id).map(|l| (id, Box::new(l.clone())));
            }
            (None, _) => app.ui.qgis.staged = None,
        }
    } else {
        app.ui.qgis.staged = None;
    }
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |bar| {
        if !app.ui.qgis.live && !raster && crate::widgets::primary_button(bar, tl!("Apply"), true, vec2(80.0, 26.0)).clicked() {
            app.ui.qgis.staged = None;
            app.map.invalidate();
        }
    });
}

/// Swaps the styled layer's look with the copy kept while Live update is off, so the map draws
/// the style it showed last until Apply; called before the map is drawn and again after.
pub fn swap_staged(app: &mut RustGisApp) {
    if !app.ui.qgis_layout || app.ui.qgis.live {
        return;
    }
    let Some((id, kept)) = app.ui.qgis.staged.as_mut() else { return };
    let Some(l) = app.session.map_mut().layer_mut(*id) else {
        app.ui.qgis.staged = None;
        return;
    };
    let k = kept.as_mut();
    std::mem::swap(&mut l.renderer, &mut k.renderer);
    std::mem::swap(&mut l.labels, &mut k.labels);
    std::mem::swap(&mut l.transparency, &mut k.transparency);
    std::mem::swap(&mut l.min_scale, &mut k.min_scale);
    std::mem::swap(&mut l.max_scale, &mut k.max_scale);
    std::mem::swap(&mut l.blend, &mut k.blend);
    std::mem::swap(&mut l.mask, &mut k.mask);
    std::mem::swap(&mut l.display_filters, &mut k.display_filters);
    std::mem::swap(&mut l.aggregation, &mut k.aggregation);
    std::mem::swap(&mut l.extrusion, &mut k.extrusion);
    std::mem::swap(&mut l.line_strokes, &mut k.line_strokes);
    std::mem::swap(&mut l.fill_layers, &mut k.fill_layers);
    std::mem::swap(&mut l.no_fill, &mut k.no_fill);
    std::mem::swap(&mut l.stroke_levels, &mut k.stroke_levels);
    std::mem::swap(&mut l.marker_rotation, &mut k.marker_rotation);
    std::mem::swap(&mut l.diagram, &mut k.diagram);
    std::mem::swap(&mut l.symbol_by, &mut k.symbol_by);
    std::mem::swap(&mut l.reference_scale, &mut k.reference_scale);
}

/// `ui.saveGpkgQgis`: Project › Save As GeoPackage for QGIS — the map's vector layers, their
/// styles and a QGIS project in one GeoPackage (`rustgis_engine::qgispkg`). The desktop asks
/// where; the browser downloads it.
pub fn save_gpkg(app: &mut RustGisApp) -> Result<Value, String> {
    let stem = rustgis_engine::qgispkg::title_file(&app.session.project.name);
    let suggested = format!("{stem}.gpkg");
    #[cfg(not(target_arch = "wasm32"))]
    let (path, file) = {
        let Some(path) = crate::dialogs::pick_save(&suggested, &["gpkg"], tl!("GeoPackage for QGIS")) else { return Ok(Value::Null) };
        let file = std::path::Path::new(&path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(suggested.clone());
        (path, file)
    };
    #[cfg(target_arch = "wasm32")]
    let file = suggested.clone();
    let pkg = rustgis_engine::qgispkg::save(&app.session, &file)?;
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::write(&path, &pkg.bytes).map_err(|e| format!("{path}: {e}"))?;
    #[cfg(target_arch = "wasm32")]
    {
        let download = app.services.download.as_ref().ok_or("this page can't download files")?;
        download(&file, pkg.bytes)?;
    }
    for n in pkg.notices {
        app.session.notify(n, false);
    }
    app.session.notify(
        crate::i18n::fmt(
            "Saved {file} for QGIS. In QGIS open it with Project › Open From › GeoPackage (or drag it into QGIS): the layers keep their styles. Keep the file's name, as the project refers to it.",
            &[("file", &file)],
        ),
        false,
    );
    Ok(json!({"file": file}))
}

/// The windows the layout opens: Settings › Options… and Plugins › QGIS Plugins & Tools.
pub fn windows(app: &mut RustGisApp, ctx: &egui::Context) {
    if app.ui.qgis.options {
        let mut open = true;
        egui::Window::new(tl!("Options"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(860.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| crate::ribbon::settings_tab(app, ui));
                });
                ui.separator();
                ui.label(egui::RichText::new(tl!("Settings › Interface switches between the QGIS layout and the ribbon.")).weak());
            });
        app.ui.qgis.options &= open;
    }
    bridge_window(app, ctx);
    crate::featureform::window(app, ctx);
}

/// Starts `work` on a thread of its own, its answer kept for the window (desktop only).
#[cfg(not(target_arch = "wasm32"))]
fn start(app: &mut RustGisApp, ctx: &egui::Context, what: &str, work: impl FnOnce() -> Result<Value, String> + Send + 'static) {
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (out, ctx) = (slot.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err("the QGIS bridge failed unexpectedly".into()));
        if let Ok(mut o) = out.lock() {
            *o = Some(r);
        }
        ctx.request_repaint();
    });
    app.ui.qgis.bridge.busy = Some((what.to_string(), slot));
    app.ui.qgis.bridge.last = None;
}

fn bridge_window(app: &mut RustGisApp, ctx: &egui::Context) {
    // Work that finished: its answer.
    if let Some((what, slot)) = &app.ui.qgis.bridge.busy {
        let done = slot.lock().ok().and_then(|mut s| s.take());
        if let Some(r) = done {
            let what = what.clone();
            app.ui.qgis.bridge.busy = None;
            app.ui.qgis.bridge.last = Some(match r {
                Ok(v) if what == "connect" => {
                    let n = v.get("algorithms").and_then(Value::as_u64).unwrap_or(0);
                    let qgis = v.get("qgis").and_then(Value::as_str).unwrap_or("").to_string();
                    let text = crate::i18n::fmt(
                        "Connected QGIS {version}: {n} algorithms are in the Processing Toolbox under QGIS.",
                        &[("version", &qgis), ("n", &n.to_string())],
                    );
                    app.session.notify(text.clone(), false);
                    Ok(text)
                }
                Ok(v) if what == "install" => {
                    let name = v.get("title").and_then(Value::as_str).unwrap_or("").to_string();
                    let processing = v.get("processing").and_then(Value::as_bool).unwrap_or(false);
                    Ok(if processing {
                        crate::i18n::fmt("Installed {name}. Connect again to list its algorithms.", &[("name", &name)])
                    } else {
                        crate::i18n::fmt(
                            "Installed {name}. It has no Processing algorithms, so it works in QGIS itself but adds no tools to KHGIS.",
                            &[("name", &name)],
                        )
                    })
                }
                Ok(_) => Ok(tl!("Done").to_string()),
                Err(e) => Err(e),
            });
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
    if !app.ui.qgis.bridge.open {
        return;
    }
    let mut open = true;
    egui::Window::new(tl!("QGIS Plugins & Tools"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            bridge_contents(app, ui);
        });
    app.ui.qgis.bridge.open &= open;
}

fn bridge_contents(app: &mut RustGisApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
    ui.label(tl!(
        "KHGIS runs the algorithms of your own QGIS: QGIS's Processing toolbox and every installed QGIS plugin that adds Processing algorithms. They appear in the Processing Toolbox under QGIS, and in the Vector and Raster menus. A plugin's own windows and dock panels stay in QGIS."
    ));
    let status = rustgis_qgis::status();
    let connected = status.get("connected").and_then(Value::as_bool).unwrap_or(false);
    ui.separator();
    if connected {
        let qgis = status.get("qgis").and_then(Value::as_str).unwrap_or("?");
        let n = status.get("algorithms").and_then(Value::as_u64).unwrap_or(0);
        ui.label(
            egui::RichText::new(crate::i18n::fmt("Connected: QGIS {version}, {n} algorithms", &[("version", qgis), ("n", &n.to_string())]))
                .color(t.green),
        );
    } else {
        ui.label(egui::RichText::new(tl!("QGIS isn't connected.")).color(t.text_dim));
    }
    let busy = app.ui.qgis.bridge.busy.is_some();
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.horizontal(|ui| {
            ui.label(tl!("QGIS Python:"));
            ui.add(
                egui::TextEdit::singleline(&mut app.ui.qgis.bridge.python)
                    .hint_text(tl!("Found automatically (python-qgis.bat, QGIS.app/…/python3)"))
                    .desired_width(ui.available_width() - 90.0),
            );
            if ui.button(tl!("Browse…")).clicked()
                && let Some(p) = rfd::FileDialog::new().set_title(tl!("QGIS's Python")).pick_file()
            {
                app.ui.qgis.bridge.python = p.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            let label = if connected { tl!("Reconnect (reload algorithms)") } else { tl!("Connect QGIS") };
            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                let python = app.ui.qgis.bridge.python.trim().to_string();
                rustgis_qgis::runner::choose((!python.is_empty()).then(|| std::path::PathBuf::from(&python)));
                if let Some(save) = &app.services.save_pref {
                    save(PYTHON_PREF, &python);
                }
                start(app, ui.ctx(), "connect", rustgis_qgis::runner::connect);
            }
            if connected && ui.add_enabled(!busy, egui::Button::new(tl!("Disconnect"))).clicked() {
                rustgis_qgis::runner::disconnect();
                app.ui.qgis.bridge.last = None;
            }
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        ui.label(tl!(
            "In the browser, QGIS runs as a KHGIS QGIS bridge service (packaging/qgis-bridge: a Docker image for Cloud Run or any server). Enter its address:"
        ));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.ui.qgis.bridge.service)
                    .hint_text("https://qgis-bridge-….run.app")
                    .desired_width(ui.available_width() - 110.0),
            );
            if ui.add_enabled(!busy && !app.ui.qgis.bridge.service.trim().is_empty(), egui::Button::new(tl!("Connect"))).clicked() {
                let url = app.ui.qgis.bridge.service.trim().trim_end_matches('/').to_string();
                if let Some(save) = &app.services.save_pref {
                    save(SERVICE_PREF, &url);
                }
                match &app.services.qgis_connect {
                    Some(connect) => {
                        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
                        connect(url, slot.clone());
                        app.ui.qgis.bridge.busy = Some(("connect".into(), slot));
                    }
                    None => app.ui.qgis.bridge.last = Some(Err(tl!("This page can't reach a QGIS bridge service.").into())),
                }
            }
        });
    }
    if let Some((what, _)) = &app.ui.qgis.bridge.busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if what == "connect" {
                tl!("Starting QGIS and loading its plugins… (this can take a minute)")
            } else {
                tl!("Working…")
            });
        });
    }
    match &app.ui.qgis.bridge.last {
        Some(Ok(text)) => {
            ui.label(egui::RichText::new(text).color(t.green));
        }
        Some(Err(e)) => {
            ui.label(egui::RichText::new(e).color(t.red));
        }
        None => {}
    }
    // The plugins QGIS loaded, with what each adds.
    let plugins = status.get("plugins").and_then(Value::as_array).cloned().unwrap_or_default();
    if connected {
        ui.separator();
        ui.label(egui::RichText::new(tl!("Installed QGIS plugins")).font(semibold(13.0)));
        if plugins.is_empty() {
            ui.label(egui::RichText::new(tl!("No plugins are installed in your QGIS profile.")).weak());
        }
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            egui::Grid::new("qgis_plugins").striped(true).num_columns(3).show(ui, |ui| {
                for p in &plugins {
                    let title = p.get("title").and_then(Value::as_str).unwrap_or("?");
                    let version = p.get("version").and_then(Value::as_str).unwrap_or("");
                    let processing = p.get("processing").and_then(Value::as_bool).unwrap_or(false);
                    let loaded = p.get("loaded").and_then(Value::as_bool).unwrap_or(false);
                    let error = p.get("error").and_then(Value::as_str);
                    ui.label(title).on_hover_text(p.get("description").and_then(Value::as_str).unwrap_or(""));
                    ui.label(egui::RichText::new(version).weak());
                    match (processing, loaded, error) {
                        (true, true, _) => ui.label(egui::RichText::new(tl!("Algorithms in the toolbox")).color(t.green)),
                        (true, false, Some(e)) => ui.label(egui::RichText::new(tl!("Failed to load")).color(t.red)).on_hover_text(e),
                        _ => ui.label(egui::RichText::new(tl!("Works in QGIS only (no Processing algorithms)")).weak()),
                    };
                    ui.end_row();
                }
            });
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.separator();
        ui.label(egui::RichText::new(tl!("Install a QGIS plugin")).font(semibold(13.0)));
        ui.label(egui::RichText::new(tl!(
            "A plugin .zip from plugins.qgis.org goes into your QGIS profile, where QGIS and KHGIS both find it. Plugins run Python code on this computer: install only those you trust."
        )).weak());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.ui.qgis.bridge.zip).hint_text("plugin.zip").desired_width(ui.available_width() - 170.0),
            );
            if ui.button(tl!("Browse…")).clicked()
                && let Some(p) = rfd::FileDialog::new().add_filter("QGIS plugin", &["zip"]).pick_file()
            {
                app.ui.qgis.bridge.zip = p.display().to_string();
            }
            let zip = app.ui.qgis.bridge.zip.trim().to_string();
            if ui.add_enabled(!busy && !zip.is_empty(), egui::Button::new(tl!("Install"))).clicked() {
                start(app, ui.ctx(), "install", move || rustgis_qgis::runner::install_zip(std::path::Path::new(&zip)));
            }
        });
        if ui.link(tl!("Browse plugins.qgis.org")).clicked() {
            app.ui.open_url = Some("https://plugins.qgis.org/plugins/?sort=-downloads".into());
        }
    }
}

#[cfg(test)]
mod toolbar_tests {
    use super::*;

    #[test]
    fn toolbars_move_between_rows_and_onto_rows_of_their_own() {
        let mut rows = arranged(&[]);
        assert_eq!(rows_pref(&rows), "project,navigation,attributes,digitizing,data,plugins");
        // Plugins onto a row of its own, below.
        move_toolbar(&mut rows, "plugins", 1, 0, true);
        assert_eq!(rows_pref(&rows), "project,navigation,attributes,digitizing,data;plugins");
        // Digitizing beside it, before it.
        move_toolbar(&mut rows, "digitizing", 1, 0, false);
        assert_eq!(rows_pref(&rows), "project,navigation,attributes,data;digitizing,plugins");
        // Project after navigation, in its own row (counted with it still there).
        move_toolbar(&mut rows, "project", 0, 2, false);
        assert_eq!(rows_pref(&rows), "navigation,project,attributes,data;digitizing,plugins");
        // Onto a row of its own above the others.
        move_toolbar(&mut rows, "data", 0, 0, true);
        assert_eq!(rows_pref(&rows), "data;navigation,project,attributes;digitizing,plugins");
        // A row emptied goes; a toolbar dropped back in its own row stays.
        move_toolbar(&mut rows, "data", 0, 1, false);
        assert_eq!(rows_pref(&rows), "data;navigation,project,attributes;digitizing,plugins");
        move_toolbar(&mut rows, "data", 1, 0, false);
        assert_eq!(rows_pref(&rows), "data,navigation,project,attributes;digitizing,plugins");
        // Saved and read back; unknown and repeated ids dropped, missing ones added.
        assert_eq!(rows_pref(&rows_from_pref(&rows_pref(&rows))), rows_pref(&rows));
        assert_eq!(rows_pref(&rows_from_pref("data,bogus;data,project")), "data;project,navigation,attributes,digitizing,plugins");
    }
}
