//! The QGIS layout's print layout designer. While a layout is open the map's menus and toolbars
//! give way to the layout's own, as QGIS's Layout window has them: a menu bar (Layout, Edit,
//! View, Add Item, Atlas) headed by the layout's name and a way back to the map, and toolbars to
//! add items, arrange them and export. Item properties are in the Format pane on the right.

use egui::{CornerRadius, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::RustGisApp;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{enabled, menu_item, menu_row, submenu_row};

/// Adds an element of kind `p` to the open layout, for the selected map frame (or the first).
fn add(app: &mut RustGisApp, mut p: Value) {
    let Some(li) = app.session.active_layout else { return };
    p["layout"] = json!(li);
    if let Some(f) = crate::ribbon::layout_frame(app) {
        p["frame"] = json!(f);
    }
    match app.run_by_user("layout.addElement", p) {
        Ok(v) => app.layout.selected = v["element"].as_u64().map(|v| v as u32),
        Err(e) => app.session.notify(e, true),
    }
}

fn add_frame(app: &mut RustGisApp, shape: &str) {
    let Some(li) = app.session.active_layout else { return };
    if let Ok(v) = app.run_by_user("layout.addFrame", json!({"layout": li, "shape": shape})) {
        app.layout.selected = v["element"].as_u64().map(|v| v as u32);
    }
}

/// The selected element's arrangement or deletion.
fn on_selected(app: &mut RustGisApp, id: &str, extra: Value) {
    let (Some(li), Some(el)) = (app.session.active_layout, app.layout.selected) else { return };
    let mut p = json!({"layout": li, "element": el});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    if id == "layout.removeElement" {
        if app.run_by_user(id, p).is_ok() {
            app.layout.selected = None;
        }
    } else {
        let _ = app.run_by_user(id, p);
    }
}

/// The items one can add: (icon, label, what to add).
const ITEMS: &[(&str, &str, &str)] = &[
    ("label_item", "Add Label", "text"),
    ("dynamic_text", "Add Dynamic Text (page, date, scale…)", "dynamicText"),
    ("legend", "Add Legend", "legend"),
    ("scale_bar", "Add Scale Bar", "scaleBar"),
    ("north_arrow", "Add North Arrow", "northArrow"),
    ("g_rect", "Add Rectangle", "rectangle"),
    ("g_ellipse", "Add Ellipse", "ellipse"),
    ("g_arrow", "Add Arrow", "arrow"),
    ("attribute_table", "Add Attribute Table", "table"),
    ("chart", "Add Chart", "chart"),
    ("colorbar", "Add Color Bar", "colorbar"),
];

/// The menu bar while a layout is open.
pub fn menu_bar(app: &mut RustGisApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let name = app.session.active_layout.and_then(|i| app.session.project.layouts.get(i)).map(|l| l.name.clone()).unwrap_or_default();
    egui::Panel::top("qgis_layout_menu_bar")
        .exact_size(30.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.header).inner_margin(egui::Margin { left: 6, right: 8, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            ui.horizontal_centered(|ui| {
                // Back to the map, then the layout's name: which window this is, at a glance.
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(format!("← {}", tl!("Back to Map"))).color(t.on_accent))
                            .fill(t.accent)
                            .corner_radius(11),
                    )
                    .on_hover_text(tl!("Close the layout and show the map"))
                    .clicked()
                {
                    let _ = app.run_by_user("layout.close", Value::Null);
                }
                ui.add_space(6.0);
                let badge = format!("{} · {name}", tl!("Print Layout"));
                ui.label(egui::RichText::new(badge).font(semibold(13.0)).color(t.accent_text));
                ui.add_space(10.0);
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button(tl!("Layout"), |ui| layout_menu(app, ui));
                    ui.menu_button(tl!("Edit"), |ui| edit_menu(app, ui));
                    ui.menu_button(tl!("View"), |ui| view_menu(app, ui));
                    ui.menu_button(tl!("Add Item"), |ui| add_menu(app, ui));
                    ui.menu_button(tl!("Atlas"), |ui| atlas_menu(app, ui));
                });
            });
            let _ = full;
        });
}

fn layout_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(250.0);
    menu_item(ui, app, "new_layout", "New Layout", "layout.new", json!({"template": "blank"}));
    submenu_row(ui, "page_setup", tl!("Page Size"), |ui| {
        let Some(li) = app.session.active_layout else { return };
        let landscape = app.session.project.layouts.get(li).is_some_and(|l| l.width > l.height);
        for (id, label, _, _) in rustgis_engine::map::layout::PAGES {
            if menu_row(ui, "", tl!(*label), "", true).clicked() {
                let o = if landscape { "landscape" } else { "portrait" };
                let _ = app.run_by_user("layout.page", json!({"layout": li, "page": id, "orientation": o}));
                ui.close();
            }
        }
        ui.separator();
        for (o, label) in [("portrait", "Portrait"), ("landscape", "Landscape")] {
            if menu_row(ui, "", tl!(label), "", true).clicked() {
                let _ = app.run_by_user("layout.page", json!({"layout": li, "orientation": o}));
                ui.close();
            }
        }
    });
    ui.separator();
    menu_item(ui, app, "export", "Export as Image or PDF…", "ui.exportLayout", Value::Null);
    menu_item(ui, app, "print", "Print…", "ui.printLayout", Value::Null);
    menu_item(ui, app, "save_layout", "Save Layout File…", "ui.saveLayout", Value::Null);
    ui.separator();
    menu_item(ui, app, "close", "Close Layout (Back to Map)", "layout.close", Value::Null);
}

fn edit_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(230.0);
    menu_item(ui, app, "undo", "Undo", "edit.undo", Value::Null);
    menu_item(ui, app, "redo", "Redo", "edit.redo", Value::Null);
    ui.separator();
    let sel = app.layout.selected.is_some();
    for (label, id, extra) in [
        ("Duplicate Item", "layout.duplicateElement", json!({})),
        ("Delete Item", "layout.removeElement", json!({})),
        ("Raise to Top", "layout.arrange", json!({"to": "front"})),
        ("Raise", "layout.arrange", json!({"to": "forward"})),
        ("Lower", "layout.arrange", json!({"to": "backward"})),
        ("Lower to Bottom", "layout.arrange", json!({"to": "back"})),
    ] {
        if menu_row(ui, "", tl!(label), "", sel).clicked() {
            on_selected(app, id, extra);
            ui.close();
        }
    }
}

fn view_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    let mut nav = app.layout.navigate;
    if ui.checkbox(&mut nav, tl!("Pan and Zoom the Map in Frames")).clicked() {
        app.layout.navigate = nav;
        ui.close();
    }
    let mut snap = !app.layout.no_snap;
    if ui.checkbox(&mut snap, tl!("Snap to Guides and Items")).clicked() {
        app.layout.no_snap = !snap;
        ui.close();
    }
    let mut rulers = !app.layout.no_rulers;
    if ui.checkbox(&mut rulers, tl!("Rulers")).clicked() {
        app.layout.no_rulers = !rulers;
        ui.close();
    }
    if let Some(li) = app.session.active_layout {
        menu_item(ui, app, "delete", "Clear Guides", "layout.guides", json!({"layout": li, "clear": true}));
    }
    ui.separator();
    menu_item(ui, app, "pane_right", "Item Properties", "ui.pane.format", Value::Null);
}

fn add_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(230.0);
    submenu_row(ui, "map", tl!("Add Map"), |ui| {
        for (shape, label) in [("rectangle", "Rectangle"), ("circle", "Circle"), ("ellipse", "Ellipse")] {
            if menu_row(ui, "", tl!(label), "", true).clicked() {
                add_frame(app, shape);
                ui.close();
            }
        }
        for (shape, label) in [("polygon", "Polygon (draw)"), ("lasso", "Lasso (draw)")] {
            if menu_row(ui, "", tl!(label), "", true).clicked() {
                let _ = app.run_by_user("ui.layoutTool", json!({"shape": shape}));
                ui.close();
            }
        }
    });
    for (icon, label, kind) in ITEMS {
        if menu_row(ui, icon, tl!(*label), "", true).clicked() {
            add(app, json!({"kind": kind}));
            ui.close();
        }
    }
    menu_item(ui, app, "picture", "Add Picture…", "ui.layoutPicture", Value::Null);
    ui.separator();
    if menu_row(ui, "", tl!("Add Map Grid"), "", true).clicked() {
        add(app, json!({"kind": "grid"}));
        ui.close();
    }
    if menu_row(ui, "", tl!("Add Neatline"), "", true).clicked() {
        add(app, json!({"kind": "rectangle"}));
        ui.close();
    }
}

fn atlas_menu(app: &mut RustGisApp, ui: &mut Ui) {
    ui.set_min_width(250.0);
    let Some(li) = app.session.active_layout else { return };
    submenu_row(ui, "atlas", tl!("Generate an Atlas From Layer"), |ui| {
        let layers: Vec<String> = app.session.map().layers.iter().filter(|l| !l.is_raster()).map(|l| l.name.clone()).collect();
        if layers.is_empty() {
            ui.label(egui::RichText::new(tl!("No vector layers")).weak());
        }
        for n in layers {
            if menu_row(ui, "", &n, "", true).clicked() {
                let mut p = json!({"layout": li, "layer": n});
                if let Some(f) = crate::ribbon::layout_frame(app) {
                    p["frame"] = json!(f);
                }
                let _ = app.run_by_user("layout.series", p);
                ui.close();
            }
        }
    });
    menu_item(ui, app, "prev_extent", "Previous Feature", "layout.seriesPage", json!({"layout": li, "page": "previous"}));
    menu_item(ui, app, "next_extent", "Next Feature", "layout.seriesPage", json!({"layout": li, "page": "next"}));
    menu_item(ui, app, "close", "Turn Atlas Off", "layout.series", json!({"layout": li, "off": true}));
}

/// A toolbar button that does `act` (enabled when `on`).
fn tbtn(ui: &mut Ui, icon: &str, tip: &str, on: bool, checked: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let tip = tl!(tip).to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, &tip));
    if checked {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.checked);
        ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if on && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
    }
    crate::icons::paint(ui.painter(), r.shrink(4.0), icon, if on { t.icon } else { t.text_disabled }, t.accent);
    resp.on_hover_text(tip).clicked() && on
}

fn sep(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 22.0), Sense::hover());
    ui.painter().line_segment([pos2(r.center().x, r.min.y + 3.0), pos2(r.center().x, r.max.y - 3.0)], Stroke::new(1.0, t.border_strong));
}

/// The selected item's lock: (layout, element, locked).
fn selected_lock(app: &RustGisApp) -> Option<(usize, u32, bool)> {
    let (li, el) = (app.session.active_layout?, app.layout.selected?);
    let l = app.session.project.layouts.get(li)?;
    l.elements.iter().find(|e| e.id == el).map(|e| (li, el, e.locked))
}

/// The toolbars while a layout is open, as QGIS's Layout window has them: the Layout toolbar
/// (save, export, print, undo), the Navigation toolbar (the page's zoom) and the Actions
/// toolbar (the selected item's order, lock, copy and deletion) along the top, and the Toolbox
/// (select, move content and every item to add) down the left.
pub fn toolbars(app: &mut RustGisApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("qgis_layout_toolbars")
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 6, right: 6, top: 2, bottom: 2 }))
        .show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([pos2(r.min.x - 6.0, r.max.y + 2.0), pos2(r.max.x + 6.0, r.max.y + 2.0)], Stroke::new(1.0, t.border));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 2.0);
                // The Layout toolbar.
                for (icon, tip, id) in [
                    ("save", "Save Project", "ui.save"),
                    ("save_layout", "Save as Template (layout file)", "ui.saveLayout"),
                    ("print", "Print Layout", "ui.printLayout"),
                    ("export_image", "Export as Image or PDF…", "ui.exportLayout"),
                    ("undo", "Undo", "edit.undo"),
                    ("redo", "Redo", "edit.redo"),
                ] {
                    if tbtn(ui, icon, tip, enabled(app, id), false) {
                        let _ = app.run_by_user(id, Value::Null);
                    }
                }
                sep(ui, &t);
                // The Navigation toolbar: the page's zoom.
                if tbtn(ui, "zoom_full", "Zoom Full", true, false) {
                    app.layout.zoom_full();
                }
                if tbtn(ui, "zoom_in", "Zoom In", true, false) {
                    app.layout.zoom_by(1.25);
                }
                if tbtn(ui, "zoom_out", "Zoom Out", true, false) {
                    app.layout.zoom_by(0.8);
                }
                if tbtn(ui, "zoom_actual", "Zoom to 100%", true, false) {
                    app.layout.zoom_actual();
                }
                sep(ui, &t);
                // The Actions toolbar: the selected item.
                let sel = app.layout.selected.is_some();
                for (icon, tip, id, extra) in [
                    ("raise", "Raise Selected Items", "layout.arrange", json!({"to": "forward"})),
                    ("lower", "Lower Selected Items", "layout.arrange", json!({"to": "backward"})),
                    ("bring_front", "Bring Selected Items to Front", "layout.arrange", json!({"to": "front"})),
                    ("send_back", "Send Selected Items to Back", "layout.arrange", json!({"to": "back"})),
                    ("duplicate", "Duplicate Item", "layout.duplicateElement", json!({})),
                ] {
                    if tbtn(ui, icon, tip, sel, false) {
                        on_selected(app, id, extra);
                    }
                }
                let lock = selected_lock(app);
                let locked = lock.is_some_and(|l| l.2);
                if tbtn(ui, if locked { "lock" } else { "unlock" }, if locked { "Unlock Item" } else { "Lock Item" }, lock.is_some(), locked)
                    && let Some((li, el, locked)) = lock
                {
                    let _ = app.run_by_user("layout.update", json!({"layout": li, "element": el, "locked": !locked}));
                }
                if tbtn(ui, "delete", "Delete Item", sel && !locked, false) {
                    on_selected(app, "layout.removeElement", json!({}));
                }
                sep(ui, &t);
                let props = app.ui.right == Some(crate::RightPane::Format);
                if tbtn(ui, "pane_right", "Items and Item Properties", true, props) {
                    let _ = app.run_by_user("ui.pane.format", Value::Null);
                }
                // A hint of what the mouse does, at the end of the row.
                let nav = app.layout.navigate;
                let hint = if nav {
                    tl!("Drag in a map frame to pan its map, wheel to zoom it")
                } else {
                    tl!("Click an item to select it, drag to move it; its properties are on the right")
                };
                ui.add_space(10.0);
                ui.label(egui::RichText::new(hint).font(regular(11.5)).color(t.text_dim));
            });
        });
    // The Toolbox down the left, as QGIS's.
    egui::Panel::left("qgis_layout_toolbox")
        .exact_size(36.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin { left: 4, right: 4, top: 6, bottom: 6 }))
        .show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([pos2(r.max.x + 4.0, r.min.y - 6.0), pos2(r.max.x + 4.0, r.max.y + 6.0)], Stroke::new(1.0, t.border));
            egui::ScrollArea::vertical().id_salt("qgis_layout_toolbox_scroll").show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                let nav = app.layout.navigate;
                if tbtn(ui, "select_item", "Select/Move Item", true, !nav) {
                    app.layout.navigate = false;
                }
                if tbtn(ui, "move_content", "Move Item Content (pan and zoom the map in a frame)", true, nav) {
                    app.layout.navigate = true;
                }
                vsep(ui, &t);
                if tbtn(ui, "map", "Add Map", true, false) {
                    add_frame(app, "rectangle");
                }
                if tbtn(ui, "picture", "Add Picture…", true, false) {
                    let _ = app.run_by_user("ui.layoutPicture", Value::Null);
                }
                for (icon, tip, kind) in ITEMS {
                    if tbtn(ui, icon, tip, true, false) {
                        add(app, json!({"kind": kind}));
                    }
                }
            });
        });
}

/// A separator across the Toolbox.
fn vsep(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(26.0, 9.0), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x + 3.0, r.center().y), pos2(r.max.x - 3.0, r.center().y)], Stroke::new(1.0, t.border_strong));
}
