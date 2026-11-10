//! KHGIS's Cambodia Data pane: choose a province (and, if you like, a district) and the data for
//! it loads by itself: its boundary and districts at once (bundled), then main roads, rivers and
//! water, schools, health facilities and villages from OpenStreetMap (Overpass), each clipped to
//! the area, and the Copernicus 30 m elevation model. Layer names follow the interface language.
//!
//! Downloads go off the frame one at a time (the tile cache's downloads: a thread natively,
//! fetch on the web), as the OpenStreetMap dialog's do; each answer is handed to `osm.download`.

use egui::Ui;
use rustgis_engine::render::rustgis_tiles::cache;
use serde_json::{Value, json};

use crate::{RustGisApp, Slot};

type Answer = Slot<Result<Vec<u8>, String>>;

/// An OpenStreetMap theme: id, English name, Khmer name, Overpass tag filters.
const THEMES: &[(&str, &str, &str, &str)] = &[
    ("roads", "Main roads", "ផ្លូវធំៗ", "highway=trunk, highway=primary, highway=secondary, highway=tertiary"),
    ("water", "Rivers and water", "ទន្លេ និងប្រភពទឹក", "waterway=river, waterway=canal, waterway=stream, natural=water"),
    ("schools", "Schools", "សាលារៀន", "amenity=school, amenity=college, amenity=university, amenity=kindergarten"),
    ("health", "Health facilities", "មណ្ឌលសុខភាព", "amenity=hospital, amenity=clinic, amenity=doctors, healthcare=centre"),
    ("places", "Villages and towns", "ភូមិ និងទីប្រជុំជន", "place=village, place=town, place=city, place=hamlet"),
    ("worship", "Pagodas and places of worship", "វត្តអារាម", "amenity=place_of_worship"),
];

/// The pane's choices and the work under way.
pub struct State {
    pub province: String,
    pub district: String,
    pub themes: Vec<bool>,
    pub dem: bool,
    /// What is still to fetch: (theme index, its request).
    queue: Vec<usize>,
    waiting: Option<(usize, Answer)>,
    /// The area's layer name and its box, while loading.
    area: Option<(String, [f64; 4])>,
    pub log: Vec<(bool, String)>,
    /// Make the map layout when the loading ends (the "Province profile" workflow).
    pub then_layout: bool,
    /// Workflow distances, in km.
    pub flood_km: f64,
    pub school_km: f64,
    pub health_km: f64,
}

impl Default for State {
    fn default() -> Self {
        State {
            province: String::new(),
            district: String::new(),
            themes: vec![true, true, true, true, true, false],
            dem: true,
            queue: Vec::new(),
            waiting: None,
            area: None,
            log: Vec::new(),
            then_layout: false,
            flood_km: 1.0,
            school_km: 3.0,
            health_km: 5.0,
        }
    }
}

fn khmer() -> bool {
    crate::i18n::current().code() == "km"
}

/// Bundled units: (English name, Khmer name, province, GeoJSON feature).
fn units(text: &str) -> Vec<(String, String, String, Value)> {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mut out: Vec<(String, String, String, Value)> = v["features"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let p = &f["properties"];
            let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
            (s("name"), s("name_km"), s("province"), f.clone())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn provinces() -> Vec<(String, String, String, Value)> {
    units(rustgis_engine::sample::CAMBODIA_PROVINCES)
}

fn districts(province: &str) -> Vec<(String, String, String, Value)> {
    units(rustgis_engine::sample::CAMBODIA_DISTRICTS).into_iter().filter(|d| d.2 == province).collect()
}

fn label(en: &str, km: &str) -> String {
    if khmer() && !km.is_empty() { format!("{km} ({en})") } else { en.to_string() }
}

/// The box round a GeoJSON feature's coordinates.
fn bbox(f: &Value) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    fn walk(v: &Value, b: &mut [f64; 4]) {
        match v.as_array() {
            Some(a) if a.len() >= 2 && a[0].is_number() => {
                let (x, y) = (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0));
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
            Some(a) => a.iter().for_each(|c| walk(c, b)),
            None => {}
        }
    }
    walk(&f["geometry"]["coordinates"], &mut b);
    (b[0] < b[2]).then_some(b)
}

fn fetch(url: &str) -> Answer {
    let slot: Answer = Slot::default();
    let land = slot.clone();
    cache::download(url, Box::new(move |r| *land.lock().unwrap_or_else(|e| e.into_inner()) = Some(r)));
    slot
}

/// The Copernicus GLO-30 tiles (1° each, cloud-optimised GeoTIFFs on AWS) covering a box.
fn dem_tiles(b: [f64; 4]) -> Vec<String> {
    let mut out = Vec::new();
    for lat in (b[1].floor() as i32)..=(b[3].floor() as i32) {
        for lon in (b[0].floor() as i32)..=(b[2].floor() as i32) {
            let ns = if lat >= 0 { format!("N{:02}", lat) } else { format!("S{:02}", -lat) };
            let ew = if lon >= 0 { format!("E{:03}", lon) } else { format!("W{:03}", -lon) };
            let name = format!("Copernicus_DSM_COG_10_{ns}_00_{ew}_00_DEM");
            out.push(format!("https://copernicus-dem-30m.s3.amazonaws.com/{name}/{name}.tif"));
        }
    }
    out
}

/// Adds GeoJSON as a layer of that name (replacing one left from an earlier load).
fn add_geojson(app: &mut RustGisApp, name: &str, fc: &Value) -> Result<(), String> {
    if app.session.map().layer_by_name(name).is_some() {
        let _ = app.run("layer.remove", json!({"layer": name}));
    }
    app.run("layer.addDataText", json!({"fileName": "area.geojson", "text": fc.to_string(), "name": name})).map(|_| ())
}

/// Starts loading: the area now, then the downloads in turn.
pub fn load(app: &mut RustGisApp) -> Result<(), String> {
    let st = &app.ui.khdata;
    let Some(p) = provinces().into_iter().find(|p| p.0 == st.province) else {
        return Err(crate::i18n::t("Choose a province first.").into());
    };
    let unit = if st.district.is_empty() { None } else { districts(&p.0).into_iter().find(|d| d.0 == st.district) };
    let (en, km, feature) = match &unit {
        Some(d) => (d.0.clone(), d.1.clone(), d.3.clone()),
        None => (p.0.clone(), p.1.clone(), p.3.clone()),
    };
    let area_name = if khmer() && !km.is_empty() { km.clone() } else { en.clone() };
    let b = bbox(&feature).ok_or("the area has no shape")?;
    // The area, then its districts (for a province).
    let fc = json!({"type": "FeatureCollection", "features": [feature]});
    add_geojson(app, &area_name, &fc)?;
    let _ = app.run("layer.noFill", json!({"layer": area_name}));
    let _ = app.run("layer.outline", json!({"layer": area_name, "color": "#22D3EE", "width": 2.2}));
    if unit.is_none() {
        let ds: Vec<Value> = districts(&p.0).into_iter().map(|d| d.3).collect();
        let n = if khmer() { format!("ស្រុក/ខណ្ឌ · {area_name}") } else { format!("Districts · {area_name}") };
        let fc = json!({"type": "FeatureCollection", "features": ds});
        if add_geojson(app, &n, &fc).is_ok() {
            let _ = app.run("layer.noFill", json!({"layer": n}));
            let _ = app.run("layer.outline", json!({"layer": n, "color": "#CFFAFE88", "width": 0.8}));
        }
    }
    let _ = app.run("layer.zoomTo", json!({"layer": area_name}));
    let st = &mut app.ui.khdata;
    st.log = vec![(true, crate::i18n::fmt("Added {name}", &[("name", &area_name)]))];
    st.queue = (0..THEMES.len()).filter(|i| st.themes.get(*i).copied().unwrap_or(false)).collect();
    st.area = Some((area_name.clone(), b));
    if st.dem {
        for url in dem_tiles(b) {
            let name = if khmer() { "រយៈកម្ពស់ (Copernicus 30 ម)".to_string() } else { "Elevation (Copernicus 30 m)".to_string() };
            match app.run("layer.addUrl", json!({"url": url, "name": name})) {
                Ok(_) => app.ui.khdata.log.push((true, name)),
                Err(e) => app.ui.khdata.log.push((false, e)),
            }
        }
    }
    next(app);
    Ok(())
}

/// Asks for the next theme in the queue.
fn next(app: &mut RustGisApp) {
    let st = &mut app.ui.khdata;
    if st.waiting.is_some() {
        return;
    }
    let Some((_, b)) = st.area.clone() else { return };
    if st.queue.is_empty() {
        st.area = None;
        if std::mem::take(&mut st.then_layout) {
            auto_layout(app);
        }
        return;
    }
    let i = st.queue.remove(0);
    let tags = THEMES[i].3;
    match rustgis_engine::render::rustgis_tiles::overpass::Query::new(b, None, Some(tags)).and_then(|q| q.url(None)) {
        Ok(url) => st.waiting = Some((i, fetch(&url))),
        Err(e) => {
            st.log.push((false, format!("{}: {e}", THEMES[i].1)));
            next(app);
        }
    }
}

/// Lands an answer: adds the theme's layers, clipped to the area, and asks for the next.
pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    let Some((_, slot)) = &app.ui.khdata.waiting else { return };
    let Some(r) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    };
    let Some((i, _)) = app.ui.khdata.waiting.take() else { return };
    let Some((area, b)) = app.ui.khdata.area.clone() else { return };
    let (_, en, km, tags) = THEMES[i];
    let theme = if khmer() { km } else { en };
    let name = format!("{theme} · {area}");
    let added = r.and_then(|bytes| String::from_utf8(bytes).map_err(|_| "the answer isn't text".to_string())).and_then(|text| {
        app.run("osm.download", json!({"bbox": b, "tags": tags, "name": name, "answer": text}))
    });
    match added {
        Ok(v) => {
            // Clip each layer to the area; keep the clipped one under the same name.
            let ids: Vec<u64> = v["layers"].as_array().into_iter().flatten().filter_map(|l| l.as_u64().or_else(|| l["id"].as_u64())).collect();
            let names: Vec<String> = ids
                .iter()
                .filter_map(|id| app.session.map().layers.iter().find(|l| u64::from(l.id) == *id).map(|l| l.name.clone()))
                .collect();
            let mut n = 0usize;
            for lname in names {
                let clipped = format!("{lname} ✂");
                let ok = app
                    .run("gp.run", json!({"tool": "clip", "params": {"input": lname, "clip": area, "output": clipped}}))
                    .is_ok();
                if ok {
                    let _ = app.run("layer.remove", json!({"layer": lname}));
                    let _ = app.run("layer.rename", json!({"layer": clipped, "name": lname}));
                }
                n += app.session.map().layers.iter().find(|l| l.name == lname).map_or(0, |l| l.dataset.features.len());
            }
            app.ui.khdata.log.push((true, crate::i18n::fmt("{theme}: {n} features", &[("theme", theme), ("n", &n.to_string())])));
        }
        Err(e) => app.ui.khdata.log.push((false, format!("{theme}: {e}"))),
    }
    app.map.invalidate();
    next(app);
}

fn lang() -> &'static str {
    if khmer() { "km" } else { "en" }
}

/// Makes the one-click map layout and opens it.
pub fn auto_layout(app: &mut RustGisApp) {
    match app.run("khgis.autoLayout", json!({"lang": lang()})) {
        Ok(_) => app.ui.khdata.log.push((true, crate::i18n::t("Map layout made: Publish › Export to save it as PDF.").to_string())),
        Err(e) => app.ui.khdata.log.push((false, e)),
    }
}

fn workflow(app: &mut RustGisApp, id: &str, km: f64) {
    match app.run("khgis.recipe", json!({"id": id, "distance": km, "lang": lang()})) {
        Ok(v) => app.ui.khdata.log.push((true, v["summary"].as_str().unwrap_or("").to_string())),
        Err(e) => app.ui.khdata.log.push((false, e)),
    }
    app.map.invalidate();
}

/// The one-click workflows.
fn workflows(app: &mut RustGisApp, ui: &mut Ui, busy: bool) {
    ui.add_space(6.0);
    ui.separator();
    ui.label(egui::RichText::new(tl!("One-click workflows")).strong());
    ui.add_enabled_ui(!busy, |ui| {
        if ui.button(tl!("Province profile: load data, then make the map")).clicked() {
            app.ui.khdata.then_layout = true;
            if let Err(e) = load(app) {
                app.ui.khdata.then_layout = false;
                app.ui.khdata.log.push((false, e));
            }
        }
        ui.horizontal(|ui| {
            if ui.button(tl!("Flood exposure")).on_hover_text(tl!("Villages near rivers and water")).clicked() {
                let km = app.ui.khdata.flood_km;
                workflow(app, "floodExposure", km);
            }
            ui.add(egui::DragValue::new(&mut app.ui.khdata.flood_km).range(0.1..=20.0).speed(0.1).suffix(" km"));
        });
        ui.horizontal(|ui| {
            if ui.button(tl!("School access")).on_hover_text(tl!("Villages far from any school")).clicked() {
                let km = app.ui.khdata.school_km;
                workflow(app, "schoolAccess", km);
            }
            ui.add(egui::DragValue::new(&mut app.ui.khdata.school_km).range(0.5..=30.0).speed(0.1).suffix(" km"));
        });
        ui.horizontal(|ui| {
            if ui.button(tl!("Health access")).on_hover_text(tl!("Villages far from any health facility")).clicked() {
                let km = app.ui.khdata.health_km;
                workflow(app, "healthAccess", km);
            }
            ui.add(egui::DragValue::new(&mut app.ui.khdata.health_km).range(0.5..=50.0).speed(0.1).suffix(" km"));
        });
        if ui.button(tl!("Schools per district")).clicked() {
            workflow(app, "districtSchools", 0.0);
        }
        if ui.button(egui::RichText::new(tl!("Auto Map Layout")).strong()).on_hover_text(tl!("An A4 map of the view: title, legend, scale bar, north arrow, inset and sources")).clicked() {
            auto_layout(app);
        }
    });
}

/// The pane.
pub fn pane(app: &mut RustGisApp, ui: &mut Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Choose an area and KHGIS loads its data: boundaries, roads, water, schools, health facilities, villages and elevation.")).color(t.text_dim));
    let provs = provinces();
    let current = provs.iter().find(|p| p.0 == app.ui.khdata.province).map(|p| label(&p.0, &p.1)).unwrap_or_else(|| tl!("Choose…").to_string());
    egui::ComboBox::from_label(tl!("Province")).selected_text(current).width(220.0).show_ui(ui, |ui| {
        for p in &provs {
            if ui.selectable_label(app.ui.khdata.province == p.0, label(&p.0, &p.1)).clicked() {
                app.ui.khdata.province = p.0.clone();
                app.ui.khdata.district.clear();
            }
        }
    });
    let ds = districts(&app.ui.khdata.province);
    let whole = tl!("Whole province").to_string();
    let current = if app.ui.khdata.district.is_empty() { whole.clone() } else { app.ui.khdata.district.clone() };
    ui.add_enabled_ui(!ds.is_empty(), |ui| {
        egui::ComboBox::from_label(tl!("District")).selected_text(current).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(app.ui.khdata.district.is_empty(), &whole).clicked() {
                app.ui.khdata.district.clear();
            }
            for d in &ds {
                if ui.selectable_label(app.ui.khdata.district == d.0, &d.0).clicked() {
                    app.ui.khdata.district = d.0.clone();
                }
            }
        });
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Data to load")).strong());
    for (i, (_, en, km, _)) in THEMES.iter().enumerate() {
        let text = if khmer() { km.to_string() } else { en.to_string() };
        if let Some(on) = app.ui.khdata.themes.get_mut(i) {
            ui.checkbox(on, text);
        }
    }
    ui.checkbox(&mut app.ui.khdata.dem, tl!("Elevation (Copernicus 30 m)"));
    ui.add_space(4.0);
    let busy = app.ui.khdata.waiting.is_some() || app.ui.khdata.area.is_some();
    ui.add_enabled_ui(!busy && !app.ui.khdata.province.is_empty(), |ui| {
        if ui.button(egui::RichText::new(tl!("Load Data")).strong()).clicked()
            && let Err(e) = load(app)
        {
            app.ui.khdata.log.push((false, e));
        }
    });
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            let left = app.ui.khdata.queue.len() + usize::from(app.ui.khdata.waiting.is_some());
            ui.label(crate::i18n::fmt("Downloading… {n} left", &[("n", &left.to_string())]));
        });
    }
    workflows(app, ui, busy);
    for (ok, line) in &app.ui.khdata.log {
        ui.label(egui::RichText::new(format!("{} {line}", if *ok { "✓" } else { "✗" })).color(if *ok { t.text_dim } else { t.red }));
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(tl!("Sources: boundaries from geoBoundaries and features from OpenStreetMap (© OpenStreetMap contributors, ODbL); elevation from Copernicus GLO-30 (© ESA)."))
            .small()
            .color(t.text_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provinces_and_districts_are_bundled() {
        let p = provinces();
        assert_eq!(p.len(), 25);
        assert!(p.iter().all(|p| !p.1.is_empty()), "every province has its Khmer name");
        assert!(districts("Battambang").len() >= 10);
        let b = bbox(&p.iter().find(|p| p.0 == "Kandal").unwrap().3).unwrap();
        assert!(b[0] > 104.0 && b[2] < 106.0 && b[1] > 10.5 && b[3] < 12.5, "{b:?}");
    }

    #[test]
    fn dem_tiles_cover_the_box() {
        let t = dem_tiles([104.4, 11.2, 105.3, 11.9]);
        assert_eq!(t.len(), 4);
        assert!(t[0].ends_with("Copernicus_DSM_COG_10_N11_00_E104_00_DEM/Copernicus_DSM_COG_10_N11_00_E104_00_DEM.tif"));
    }

    #[test]
    fn every_theme_is_a_narrow_query_over_a_large_province() {
        let b = bbox(&provinces().into_iter().find(|p| p.0 == "Mondulkiri").unwrap().3).unwrap();
        for (_, en, _, tags) in THEMES {
            assert!(rustgis_engine::render::rustgis_tiles::overpass::Query::new(b, None, Some(tags)).is_ok(), "{en}");
        }
    }
}
