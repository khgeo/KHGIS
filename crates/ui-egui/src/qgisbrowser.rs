//! The QGIS layout's Browser panel, as QGIS's: a toolbar (Add Folder, Refresh, Collapse All, a
//! filter) over a tree of Favorites (the project's folder connections), Spatial Bookmarks,
//! Project Home, Home and the file system's roots on the desktop; the folders the browser was
//! let read in the web app (Chrome, Edge); XYZ Tiles; and the project's maps and layouts.
//! Folders open in place; a double-click adds a file (a Shapefile by its `.shp`, its parts read
//! from beside it) or opens a project.

use std::collections::HashMap;

use egui::{Sense, Ui, vec2};
use serde_json::json;

use crate::theme::{Tokens, regular};
use crate::{RustGisApp, Slot, icons};

/// The Browser panel's own state.
#[derive(Default)]
pub struct State {
    /// The web app's connected folders, and the answer being waited for (on start, or a pick).
    pub folders: Vec<(u32, String)>,
    pub folders_slot: Option<Slot<Result<Vec<(u32, String)>, String>>>,
    pub folders_asked: bool,
    /// The web app's folder listings, by (folder, path), as they arrive.
    pub listings: HashMap<(u32, String), Slot<Result<Vec<(String, bool)>, String>>>,
    /// Bumped by Collapse All (every node shut) and Refresh (listings read again).
    pub generation: u32,
    pub filter: String,
}

/// The files listed: vector and raster data, tables, databases and KHGIS files. A Shapefile is
/// listed by its `.shp` (its parts are read with it).
const LISTED: &[&str] = &[
    "geojson",
    "json",
    "csv",
    "tsv",
    "txt",
    "shp",
    "zip",
    "kml",
    "kmz",
    "gpx",
    "gml",
    "fgb",
    "parquet",
    "geoparquet",
    "xlsx",
    "xls",
    "ods",
    "dxf",
    "tif",
    "tiff",
    "asc",
    "img",
    "vrt",
    "jp2",
    "nc",
    "pmtiles",
    "mbtiles",
    "gpkg",
    "duckdb",
    "ddb",
    "las",
    "laz",
    "tab",
    "mif",
    "osm",
    "pbf",
    "kgis",
    "rgis",
    "rgisx",
    "rgispkg",
    "rgismap",
    "rgismpk",
    "rgislyr",
    "rgislayout",
    "qgs",
    "qgz",
];

fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// The icon a file is drawn with, by its kind.
fn file_icon(name: &str) -> &'static str {
    match ext(name).as_str() {
        "tif" | "tiff" | "asc" | "img" | "vrt" | "jp2" | "nc" => "raster_layer",
        "csv" | "tsv" | "txt" | "xlsx" | "xls" | "ods" => "delimited",
        "gpkg" | "duckdb" | "ddb" | "gdb" | "pmtiles" | "mbtiles" => "database",
        "zip" | "kmz" => "package",
        "las" | "laz" => "point_layer",
        "kgis" | "rgis" | "rgisx" | "rgispkg" | "qgs" | "qgz" => "logo",
        "rgismap" | "rgismpk" | "rgislyr" | "rgislayout" => "new_layout",
        "gpx" | "osm" | "pbf" => "line_layer",
        _ => "polygon_layer",
    }
}

fn listed(name: &str, filter: &str) -> bool {
    !name.starts_with('.') && LISTED.contains(&ext(name).as_str()) && (filter.is_empty() || name.to_lowercase().contains(filter))
}

/// One row of the tree: an icon and a name, clickable; its response.
fn leaf(ui: &mut Ui, icon: &str, text: &str, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let ir = egui::Rect::from_min_size(egui::pos2(r.min.x + 2.0, r.min.y + 2.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, t.icon, t.accent);
    ui.painter().text(egui::pos2(ir.max.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER, text, regular(12.5), t.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TreeItem, true, text));
    resp
}

/// A node that opens: its header (icon and name) and, when open, `body`.
fn node(ui: &mut Ui, id: egui::Id, open: bool, icon: &str, text: &str, t: &Tokens, body: impl FnOnce(&mut Ui)) -> egui::Response {
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let (_, header, _) = state
        .show_header(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, icon, t.icon, t.accent);
            ui.add(egui::Label::new(egui::RichText::new(text).font(regular(12.5))).sense(Sense::click()).selectable(false))
        })
        .body(body);
    // A click on the name opens or shuts it, as the arrow does.
    if header.inner.clicked()
        && let Some(mut state) = egui::collapsing_header::CollapsingState::load(ui.ctx(), id)
    {
        state.toggle(ui);
        state.store(ui.ctx());
    }
    header.inner
}

/// Adds or opens a file on the desktop by its path, as a double-click in QGIS's Browser does.
#[cfg(not(target_arch = "wasm32"))]
fn open_path(app: &mut RustGisApp, path: &std::path::Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let p = path.display().to_string();
    let r = if rustgis_engine::project::opens_as_project(&name) {
        app.run_by_user("project.open", json!({"path": p}))
    } else if matches!(ext(&name).as_str(), "qgs" | "qgz") {
        app.run_by_user("project.importQgis", json!({"path": p}))
    } else if matches!(ext(&name).as_str(), "rgismap" | "rgismpk") {
        app.run("map.import", json!({"path": p}))
    } else if ext(&name) == "rgislayout" {
        app.run("layout.import", json!({"path": p}))
    } else {
        crate::layerchooser::add_file(app, &p)
    };
    if let Err(e) = r {
        app.session.notify(e, true);
    }
    app.map.invalidate();
}

/// A folder on the desktop, opened in place: its folders, then its files.
#[cfg(not(target_arch = "wasm32"))]
fn dir_node(
    app: &mut RustGisApp,
    ui: &mut Ui,
    path: &std::path::Path,
    label: &str,
    icon: &str,
    depth: usize,
    t: &Tokens,
) -> egui::Response {
    let id = egui::Id::unique(("qgis_browser_dir", path.to_path_buf(), app.ui.qgis.browse.generation));
    let filter = app.ui.qgis.browse.filter.to_lowercase();
    node(ui, id, false, icon, label, t, |ui| {
        let Ok(rd) = std::fs::read_dir(path) else {
            ui.label(egui::RichText::new(tl!("Can't read this folder")).small().weak());
            return;
        };
        let mut entries: Vec<(bool, String, std::path::PathBuf)> = rd
            .filter_map(Result::ok)
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                // A File Geodatabase's folder is data, added like a file.
                let dir = e.path().is_dir() && !rustgis_engine::data::source::is_gdb_name(&name);
                (dir, name, e.path())
            })
            .filter(
                |(dir, name, _)| if *dir { !name.starts_with('.') } else { listed(name, &filter) || name.to_lowercase().ends_with(".gdb") },
            )
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
        let n = entries.len();
        for (is_dir, name, p) in entries.into_iter().take(500) {
            if is_dir {
                if depth < 24 {
                    dir_node(app, ui, &p, &name, "folder", depth + 1, t);
                }
            } else {
                let icon = if name.to_lowercase().ends_with(".gdb") { "database" } else { file_icon(&name) };
                let resp = leaf(ui, icon, &name, t).on_hover_text(tl!("Double-click to add it to the map"));
                if resp.double_clicked() {
                    open_path(app, &p);
                }
                resp.context_menu(|ui| {
                    if ui.button(tl!("Add Layer to Project")).clicked() {
                        open_path(app, &p);
                        ui.close();
                    }
                });
            }
        }
        if n > 500 {
            ui.label(egui::RichText::new(crate::i18n::fmt("{n} more not listed", &[("n", &(n - 500).to_string())])).small().weak());
        }
    })
}

/// A folder the browser was let read, at `path` inside it, opened in place.
fn web_node(app: &mut RustGisApp, ui: &mut Ui, folder: u32, path: &str, label: &str, icon: &str, t: &Tokens) -> egui::Response {
    let id = egui::Id::unique(("qgis_browser_web", folder, path.to_string(), app.ui.qgis.browse.generation));
    let filter = app.ui.qgis.browse.filter.to_lowercase();
    node(ui, id, false, icon, label, t, |ui| {
        let key = (folder, path.to_string());
        let slot = match app.ui.qgis.browse.listings.get(&key) {
            Some(s) => s.clone(),
            None => {
                let slot: Slot<Result<Vec<(String, bool)>, String>> = Default::default();
                if let Some(f) = &app.services.local_folders {
                    (f.list)(folder, path.to_string(), slot.clone());
                }
                app.ui.qgis.browse.listings.insert(key.clone(), slot.clone());
                slot
            }
        };
        let got = slot.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match got {
            None => {
                ui.label(egui::RichText::new(tl!("Reading…")).small().weak());
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e).small().color(t.red));
                if ui.small_button(tl!("Try again")).clicked() {
                    app.ui.qgis.browse.listings.remove(&key);
                }
            }
            Some(Ok(mut entries)) => {
                entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
                for (name, dir) in entries.into_iter().filter(|(n, d)| *d || listed(n, &filter)).take(500) {
                    let inner = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
                    if dir {
                        web_node(app, ui, folder, &inner, &name, "folder", t);
                    } else {
                        let resp = leaf(ui, file_icon(&name), &name, t).on_hover_text(tl!("Double-click to add it to the map"));
                        let mut add = resp.double_clicked();
                        resp.context_menu(|ui| {
                            if ui.button(tl!("Add Layer to Project")).clicked() {
                                add = true;
                                ui.close();
                            }
                        });
                        if add && let Some(f) = &app.services.local_folders {
                            (f.open)(folder, path.to_string(), name.clone());
                        }
                    }
                }
            }
        }
    })
}

/// Takes the connected folders when their answer has come.
fn take_folders(app: &mut RustGisApp) {
    let b = &mut app.ui.qgis.browse;
    if let Some(slot) = &b.folders_slot {
        let got = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(r) = got {
            b.folders_slot = None;
            match r {
                Ok(list) => b.folders = list,
                Err(e) => app.session.notify(e, true),
            }
        }
    }
}

/// Add Folder…: the desktop's folder dialog (a folder connection of the project), or the
/// browser's folder picker.
fn add_folder(app: &mut RustGisApp) {
    if let Some(f) = &app.services.local_folders {
        let slot: Slot<Result<Vec<(u32, String)>, String>> = Default::default();
        (f.connect)(slot.clone());
        app.ui.qgis.browse.folders_slot = Some(slot);
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = rfd::FileDialog::new().set_title(tl!("Add Folder to Favorites")).pick_folder() {
        let _ = app.run("project.addFolder", json!({"path": dir.display().to_string()}));
    }
}

pub(crate) fn show(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens) {
    // The web app's folders connected before, asked for once.
    if !app.ui.qgis.browse.folders_asked
        && let Some(f) = &app.services.local_folders
    {
        let slot: Slot<Result<Vec<(u32, String)>, String>> = Default::default();
        (f.known)(slot.clone());
        app.ui.qgis.browse.folders_slot = Some(slot);
        app.ui.qgis.browse.folders_asked = true;
    }
    take_folders(app);
    let web = app.services.local_folders.is_some();
    let can_add = web || !cfg!(target_arch = "wasm32");
    // The toolbar.
    egui::Frame::NONE.inner_margin(egui::Margin { left: 6, right: 6, top: 4, bottom: 2 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tip = if can_add {
                tl!("Add Folder… (to Favorites)")
            } else {
                tl!("This browser can't open local folders: use Chrome or Edge")
            };
            let r = ui.add_enabled_ui(can_add, |ui| crate::widgets::icon_button(ui, "add_folder", tip)).inner;
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can_add, tl!("Add Folder…")));
            if r.on_disabled_hover_text(tip).clicked() {
                add_folder(app);
            }
            let r = crate::widgets::icon_button(ui, "update", tl!("Refresh"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Refresh")));
            if r.clicked() {
                app.ui.qgis.browse.listings.clear();
                app.ui.qgis.browse.generation = app.ui.qgis.browse.generation.wrapping_add(1);
            }
            let r = crate::widgets::icon_button(ui, "collapse_all", tl!("Collapse All"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Collapse All")));
            if r.clicked() {
                app.ui.qgis.browse.generation = app.ui.qgis.browse.generation.wrapping_add(1);
            }
        });
        let mut q = app.ui.qgis.browse.filter.clone();
        if crate::widgets::search_field(ui, &mut q, tl!("Filter files")).changed() {
            app.ui.qgis.browse.filter = q;
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("qgis_browser_tree").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| tree(app, ui, web, t));
    });
}

fn tree(app: &mut RustGisApp, ui: &mut Ui, web: bool, t: &Tokens) {
    let generation = app.ui.qgis.browse.generation;
    // Favorites: the project's folder connections (desktop), or the folders the browser may read.
    node(ui, egui::Id::unique(("qgis_browser_fav", generation)), true, "favorites", tl!("Favorites"), t, |ui| {
        if web {
            let folders = app.ui.qgis.browse.folders.clone();
            if folders.is_empty() {
                let r =
                    leaf(ui, "add_folder", tl!("Add Folder…"), t).on_hover_text(tl!("Pick a folder on this computer to browse it here"));
                if r.clicked() {
                    add_folder(app);
                }
            }
            for (id, name) in folders {
                let r = web_node(app, ui, id, "", &name, "folder", t);
                r.context_menu(|ui| {
                    if ui.button(tl!("Remove Folder from Favorites")).clicked() {
                        if let Some(f) = &app.services.local_folders {
                            (f.forget)(id);
                        }
                        app.ui.qgis.browse.folders.retain(|f| f.0 != id);
                        app.ui.qgis.browse.listings.retain(|k, _| k.0 != id);
                        ui.close();
                    }
                });
            }
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let folders = app.session.project.folders.clone();
                if folders.is_empty() && leaf(ui, "add_folder", tl!("Add Folder…"), t).clicked() {
                    add_folder(app);
                }
                for f in folders {
                    let p = std::path::PathBuf::from(&f);
                    let label = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
                    let r = dir_node(app, ui, &p, &label, "folder", 0, t).on_hover_text(&f);
                    r.context_menu(|ui| {
                        if ui.button(tl!("Remove Folder from Favorites")).clicked() {
                            let _ = app.run("project.removeFolder", json!({"path": f}));
                            ui.close();
                        }
                    });
                }
            }
            #[cfg(target_arch = "wasm32")]
            ui.label(egui::RichText::new(tl!("This browser can't open local folders: use Chrome or Edge")).small().weak());
        }
    });
    // Spatial Bookmarks.
    let marks: Vec<String> = app.session.map().bookmarks.iter().map(|b| b.name.clone()).collect();
    node(ui, egui::Id::unique(("qgis_browser_marks", generation)), false, "bookmark", tl!("Spatial Bookmarks"), t, |ui| {
        if marks.is_empty() {
            ui.label(egui::RichText::new(tl!("None yet: View › New Spatial Bookmark")).small().weak());
        }
        for m in marks {
            if leaf(ui, "bookmark", &m, t).double_clicked() {
                let _ = app.run("bookmark.go", json!({"name": m}));
            }
        }
    });
    // The desktop's folders: the project's, the home folder and the roots.
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(home) = app.session.path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf()) {
            dir_node(app, ui, &home, tl!("Project Home"), "project_home", 0, t).on_hover_text(home.display().to_string());
        }
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from) {
            dir_node(app, ui, &home, tl!("Home"), "home", 0, t).on_hover_text(home.display().to_string());
        }
        for root in roots() {
            let label = root.display().to_string();
            dir_node(app, ui, &root, &label, "drive", 0, t);
        }
    }
    // XYZ Tiles: the web basemaps, a double-click adds one.
    node(ui, egui::Id::unique(("qgis_browser_xyz", generation)), false, "xyz", tl!("XYZ Tiles"), t, |ui| {
        for (name, url, attribution, max_zoom) in rustgis_engine::sample::WEB_BASEMAPS {
            let r = leaf(ui, "basemap", name, t).on_hover_text(tl!("Double-click to add it to the map"));
            if r.double_clicked() {
                let id = name.to_ascii_lowercase().replace(' ', "-");
                let p = json!({"url": url, "name": name, "attribution": attribution, "maxZoom": max_zoom, "id": id, "kind": "raster"});
                if let Err(e) = app.run("map.addTiles", p) {
                    app.session.notify(e, true);
                }
            }
        }
    });
    // The project's maps and layouts.
    node(ui, egui::Id::unique(("qgis_browser_project", generation)), true, "logo", tl!("Project"), t, |ui| {
        let maps: Vec<String> = app.session.project.maps.iter().map(|m| m.name.clone()).collect();
        let active = app.session.project.active_map;
        for (i, m) in maps.iter().enumerate() {
            let shown = if i == active { format!("{m} ●") } else { m.clone() };
            let r = leaf(ui, "map", &shown, t);
            if r.double_clicked() || r.clicked() {
                let _ = app.run("map.activate", json!({"name": m}));
            }
            r.context_menu(|ui| crate::panes::map_menu(app, ui, m));
        }
        let layouts: Vec<String> = app.session.project.layouts.iter().map(|l| l.name.clone()).collect();
        for n in layouts {
            let r = leaf(ui, "new_layout", &n, t).on_hover_text(tl!("Double-click to open the layout"));
            if r.double_clicked() {
                let _ = app.run("layout.activate", json!({"name": n}));
            }
            r.context_menu(|ui| crate::panes::layout_menu(app, ui, &n));
        }
    });
}

/// The file system's roots: `/`, or each drive on Windows.
#[cfg(not(target_arch = "wasm32"))]
fn roots() -> Vec<std::path::PathBuf> {
    if cfg!(windows) {
        (b'C'..=b'Z').map(|d| std::path::PathBuf::from(format!("{}:\\", d as char))).filter(|p| p.is_dir()).collect()
    } else {
        vec![std::path::PathBuf::from("/")]
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    use crate::RustGisApp;

    /// A favorite folder opens on a click of its name; a double-click on a file adds it.
    #[test]
    fn favorites_open_and_a_file_is_added() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Provinces")).unwrap();
        let geojson = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"n":1},"geometry":{"type":"Point","coordinates":[104.9,11.5]}}]}"#;
        std::fs::write(dir.path().join("Provinces").join("wells.geojson"), geojson).unwrap();
        std::fs::write(dir.path().join("notes.docx"), b"x").unwrap();
        let mut app = RustGisApp::with_sample();
        let _ = app.run("ui.language", json!({"code": "en"}));
        app.ui.qgis_layout = true;
        app.run("project.addFolder", json!({"path": dir.path().display().to_string()})).unwrap();
        let name = dir.path().file_name().unwrap().to_string_lossy().to_string();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_ui_state(
            |ui, app: &mut RustGisApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        h.get_by_label("Browser").click();
        h.run_steps(3);
        assert!(h.query_by_label("Favorites").is_some());
        h.get_by_label(&name).click();
        h.run_steps(3);
        assert!(h.query_by_label("notes.docx").is_none(), "only data is listed");
        h.get_by_label("Provinces").click();
        h.run_steps(3);
        let file = h.get_by_label("wells.geojson");
        file.click();
        file.click();
        h.run_steps(4);
        assert!(h.state().session.map().layer_by_name("wells").is_some(), "added by a double-click");
    }
}
