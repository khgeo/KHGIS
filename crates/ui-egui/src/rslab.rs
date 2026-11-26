//! LEE RS Lab: KHGIS's built-in remote sensing lab, after the LEE RS Lab QGIS plugin, for
//! teaching basic remote sensing online, no QGIS needed. Five steps: 1 Data (a study area from
//! Cambodia's provinces and districts or the map view, a Sentinel-2 or Landsat 8/9 search by date
//! and cloud, and the scene downloaded clipped to the area as one six-band reflectance image),
//! 2 Enhance (band combinations and spectral indices), 3 Classify (k-means or supervised),
//! 4 Change (two dates compared) and 5 Map (the A4 layout, and the area in Cambodia 3D).
//!
//! The search and the images come from Microsoft's Planetary Computer, with no login: its STAC
//! search, and its data API's `item/{bbox}` GeoTIFFs, which clip and stack a scene's bands on the
//! server (and answer the browser, so the web app reads them too). A scene's tiles are downloaded
//! on one grid and merged into one image (`raster.process` "merge"), its stored numbers made
//! reflectance on the way.

use egui::Ui;
use rustgis_engine::render::rustgis_tiles::cache;
use serde_json::{Value, json};

use crate::{RustGisApp, Slot};

type Answer = Slot<Result<Vec<u8>, String>>;

/// The Planetary Computer's STAC search and data API.
const STAC: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/search";
const DATA: &str = "https://planetarycomputer.microsoft.com/api/data/v1/item/bbox";
/// The longest side of an image downloaded, in cells: past it the cells are made larger.
pub(crate) const MAX_SIDE: u32 = 2500;
/// The most tiles of one date downloaded for an area.
const MAX_TILES: usize = 6;
/// The bands of an image, in order (1 to 6), as both satellites name them.
pub(crate) const BANDS: [&str; 6] = ["Blue", "Green", "Red", "NIR", "SWIR1", "SWIR2"];

/// A satellite the lab searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    Sentinel2,
    Landsat,
}

impl Sensor {
    fn collection(self) -> &'static str {
        match self {
            Sensor::Sentinel2 => "sentinel-2-l2a",
            Sensor::Landsat => "landsat-c2-l2",
        }
    }
    /// The assets read, blue to SWIR 2.
    fn assets(self) -> [&'static str; 6] {
        match self {
            Sensor::Sentinel2 => ["B02", "B03", "B04", "B08", "B11", "B12"],
            Sensor::Landsat => ["blue", "green", "red", "nir08", "swir16", "swir22"],
        }
    }
    fn cell(self) -> f64 {
        match self {
            Sensor::Sentinel2 => 10.0,
            Sensor::Landsat => 30.0,
        }
    }
    fn short(self) -> &'static str {
        match self {
            Sensor::Sentinel2 => "S2",
            Sensor::Landsat => "L8-9",
        }
    }
}

/// A date the search found: its tiles over the area, and how cloudy they are.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub date: String,
    pub items: Vec<String>,
    pub cloud: f64,
    /// Stored number × scale + offset is reflectance.
    pub scale: f64,
    pub offset: f64,
}

/// What a scene's download waits for: its tiles' layers, then the image they are merged into.
struct Loading {
    parts: Vec<String>,
    image: String,
    scale: f64,
    offset: f64,
    merging: bool,
    /// Polls since the merge stopped running without its image: it failed (its error was said).
    idle: u32,
}

/// The lab's choices and the work under way.
pub struct State {
    pub step: usize,
    /// The study area: a province (and district), or the map's view.
    pub use_view: bool,
    pub province: String,
    pub district: String,
    pub sensor: Sensor,
    pub start: String,
    pub end: String,
    pub cloud: f64,
    pub cell: f64,
    search: Option<(Answer, Sensor)>,
    pub scenes: Vec<Scene>,
    pub scenes_of: Sensor,
    pub picked: Option<usize>,
    loading: Option<Loading>,
    /// The image the later steps work on.
    pub image: String,
    pub classes: f64,
    pub training: String,
    pub class_field: String,
    pub before: String,
    pub after: String,
    pub method: &'static str,
    pub log: Vec<(bool, String)>,
}

impl Default for State {
    fn default() -> Self {
        State {
            step: 0,
            use_view: false,
            province: String::new(),
            district: String::new(),
            sensor: Sensor::Sentinel2,
            start: crate::stacui::days_ago(180),
            end: crate::stacui::days_ago(0),
            cloud: 20.0,
            cell: 10.0,
            search: None,
            scenes: Vec::new(),
            scenes_of: Sensor::Sentinel2,
            picked: None,
            loading: None,
            image: String::new(),
            classes: 6.0,
            training: String::new(),
            class_field: String::new(),
            before: String::new(),
            after: String::new(),
            method: "difference",
            log: Vec::new(),
        }
    }
}

/// The STAC search for scenes over `bbox` between two dates, the cloudiest left out.
pub(crate) fn search_url(sensor: Sensor, bbox: [f64; 4], start: &str, end: &str, cloud: f64) -> String {
    let mut filter = format!("eo:cloud_cover<{}", cloud.clamp(0.0, 100.0));
    if sensor == Sensor::Landsat {
        filter.push_str(" AND platform IN ('landsat-8','landsat-9')");
    }
    let enc = |s: &str| s.replace('%', "%25").replace(' ', "%20").replace('<', "%3C").replace('\'', "%27").replace('+', "%2B");
    format!(
        "{STAC}?collections={}&bbox={:.5},{:.5},{:.5},{:.5}&datetime={}T00:00:00Z/{}T23:59:59Z&limit=100&filter={}&filter-lang=cql2-text&sortby={}",
        sensor.collection(),
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3],
        start.trim(),
        end.trim(),
        enc(&filter),
        enc("+eo:cloud_cover"),
    )
}

/// The search's items as dates: each date's tiles together, the clearest dates first.
pub(crate) fn scenes(sensor: Sensor, bytes: &[u8]) -> Result<Vec<Scene>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("the search's answer isn't JSON: {e}"))?;
    let features = v["features"].as_array().ok_or_else(|| {
        let why = v["description"].as_str().or_else(|| v["detail"].as_str()).unwrap_or("no features");
        format!("the search found nothing: {why}")
    })?;
    let mut out: Vec<(Scene, usize)> = Vec::new();
    for f in features {
        let (Some(id), Some(when)) = (f["id"].as_str(), f["properties"]["datetime"].as_str()) else { continue };
        let date: String = when.chars().take(10).collect();
        let cloud = f["properties"]["eo:cloud_cover"].as_f64().unwrap_or(0.0);
        let (scale, offset) = match sensor {
            // From processing baseline 04.00 (2022) the stored numbers have 1000 added.
            Sensor::Sentinel2 => {
                let baseline = f["properties"]["s2:processing_baseline"].as_str().and_then(|b| b.parse::<f64>().ok()).unwrap_or(5.0);
                (0.0001, if baseline >= 4.0 { -0.1 } else { 0.0 })
            }
            Sensor::Landsat => (0.0000275, -0.2),
        };
        match out.iter_mut().find(|(s, _)| s.date == date) {
            Some((s, n)) => {
                if !s.items.iter().any(|i| i == id) {
                    s.items.push(id.to_string());
                    s.cloud += cloud;
                    *n += 1;
                }
            }
            None => out.push((Scene { date, items: vec![id.to_string()], cloud, scale, offset }, 1)),
        }
    }
    let mut out: Vec<Scene> = out
        .into_iter()
        .map(|(mut s, n)| {
            s.cloud /= n as f64;
            s
        })
        .collect();
    out.sort_by(|a, b| a.cloud.total_cmp(&b.cloud).then(b.date.cmp(&a.date)));
    Ok(out)
}

/// The image's size over `bbox` with cells of about `cell` metres: its columns and rows, and
/// the cell size it gets (larger when the area would be more than [`MAX_SIDE`] cells across).
pub(crate) fn grid(bbox: [f64; 4], cell: f64) -> (u32, u32, f64) {
    let lat = ((bbox[1] + bbox[3]) / 2.0).to_radians().cos().abs().max(0.1);
    let (w_m, h_m) = ((bbox[2] - bbox[0]) * 111_320.0 * lat, (bbox[3] - bbox[1]) * 111_320.0);
    let cell = cell.max(1.0);
    let longest = w_m.max(h_m) / cell;
    let cell = if longest > f64::from(MAX_SIDE) { w_m.max(h_m) / f64::from(MAX_SIDE) } else { cell };
    let side = |m: f64| ((m / cell).round() as u32).clamp(1, MAX_SIDE);
    (side(w_m), side(h_m), cell)
}

/// One tile's image over `bbox`: its six bands, clipped and stacked by the data API.
pub(crate) fn image_url(sensor: Sensor, item: &str, bbox: [f64; 4], w: u32, h: u32) -> String {
    let assets: String = sensor.assets().iter().map(|a| format!("&assets={a}")).collect();
    format!(
        "{DATA}/{:.6},{:.6},{:.6},{:.6}/{w}x{h}.tif?collection={}&item={item}{assets}&return_mask=false",
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3],
        sensor.collection()
    )
}

/// The Cambodia 3D page for the middle of `bbox`, at most 4 × 4 km (the page builds up to 20 km²).
pub(crate) fn cambodia3d_url(bbox: [f64; 4]) -> String {
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
    let lat = cy.to_radians().cos().abs().max(0.1);
    let half_x = ((bbox[2] - bbox[0]) / 2.0).min(2_000.0 / (111_320.0 * lat)).max(0.0005);
    let half_y = ((bbox[3] - bbox[1]) / 2.0).clamp(0.0005, 2_000.0 / 111_320.0);
    format!("https://khgeo.github.io/cambodia-3d/#aoi={:.5},{:.5},{:.5},{:.5}", cx - half_x, cy - half_y, cx + half_x, cy + half_y)
}

/// The study area's box: the district or province chosen, or the map's view.
fn area(app: &RustGisApp) -> Result<([f64; 4], String), String> {
    let st = &app.ui.rslab;
    if st.use_view {
        let e = rustgis_engine::cmd::workspace::view_extent(&app.session);
        if e.is_empty() {
            return Err(tl!("The map has no view yet.").to_string());
        }
        return Ok(([e.min_x.max(-180.0), e.min_y.max(-90.0), e.max_x.min(180.0), e.max_y.min(90.0)], "view".into()));
    }
    if st.province.is_empty() {
        return Err(tl!("Choose a province (or the map view) first.").to_string());
    }
    let unit = if st.district.is_empty() {
        crate::khdata::provinces().into_iter().find(|p| p.0 == st.province)
    } else {
        crate::khdata::districts(&st.province).into_iter().find(|d| d.0 == st.district)
    };
    let (name, _, _, f) = unit.ok_or_else(|| tl!("Choose a province (or the map view) first.").to_string())?;
    let b = crate::khdata::bbox(&f).ok_or_else(|| tl!("Choose a province (or the map view) first.").to_string())?;
    Ok((b, name))
}

fn fetch(url: &str) -> Answer {
    let slot: Answer = Slot::default();
    let land = slot.clone();
    cache::download(url, Box::new(move |r| *land.lock().unwrap_or_else(|e| e.into_inner()) = Some(r)));
    slot
}

fn say(app: &mut RustGisApp, ok: bool, line: String) {
    let log = &mut app.ui.rslab.log;
    log.push((ok, line));
    if log.len() > 12 {
        log.remove(0);
    }
}

fn search(app: &mut RustGisApp) {
    let (bbox, _) = match area(app) {
        Ok(a) => a,
        Err(e) => return say(app, false, e),
    };
    let st = &mut app.ui.rslab;
    let url = search_url(st.sensor, bbox, &st.start, &st.end, st.cloud);
    st.search = Some((fetch(&url), st.sensor));
    st.scenes.clear();
    st.picked = None;
}

/// Downloads the scene picked, each tile on the area's grid, to be merged when they are in.
fn download(app: &mut RustGisApp) {
    let (bbox, place) = match area(app) {
        Ok(a) => a,
        Err(e) => return say(app, false, e),
    };
    let st = &app.ui.rslab;
    let Some(scene) = st.picked.and_then(|i| st.scenes.get(i)).cloned() else {
        return say(app, false, tl!("Search, then pick a date.").to_string());
    };
    let sensor = st.scenes_of;
    let (w, h, cell) = grid(bbox, st.cell);
    if cell > st.cell * 1.01 {
        let text = crate::i18n::fmt(
            "The area is large: cells of {m} m (at most {n} across).",
            &[("m", &format!("{cell:.0}")), ("n", &MAX_SIDE.to_string())],
        );
        say(app, true, text);
    }
    let place = if place == "view" { tl!("view").to_string() } else { place.replace(' ', "_") };
    let image = format!("{}_{}_{place}", sensor.short(), scene.date);
    let mut parts = Vec::new();
    for (i, item) in scene.items.iter().take(MAX_TILES).enumerate() {
        let part = format!("{image}_part{}", i + 1);
        let url = image_url(sensor, item, bbox, w, h);
        if let Err(e) = app.run("ui.addUrl", json!({"url": url, "name": part, "whole": true})) {
            return say(app, false, e);
        }
        parts.push(part);
    }
    if scene.items.len() > MAX_TILES {
        let text = crate::i18n::fmt(
            "{n} tiles cover the area; the first {k} are read.",
            &[("n", &scene.items.len().to_string()), ("k", &MAX_TILES.to_string())],
        );
        say(app, false, text);
    }
    app.ui.rslab.loading = Some(Loading { parts, image, scale: scene.scale, offset: scene.offset, merging: false, idle: 0 });
}

fn raster_exists(app: &RustGisApp, name: &str) -> bool {
    app.session.map().layer_by_name(name).is_some_and(|l| l.raster.as_ref().is_some_and(|r| r.data.is_some()))
}

/// Lands the search, merges a scene's tiles once they are in, and styles the image made.
pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    if let Some((slot, sensor)) = &app.ui.rslab.search {
        let sensor = *sensor;
        let landed = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        match landed {
            None => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            Some(r) => {
                app.ui.rslab.search = None;
                match r.and_then(|b| scenes(sensor, &b)) {
                    Ok(found) => {
                        let text = crate::i18n::fmt("{n} date(s) found.", &[("n", &found.len().to_string())]);
                        app.ui.rslab.picked = (!found.is_empty()).then_some(0);
                        app.ui.rslab.scenes = found;
                        app.ui.rslab.scenes_of = sensor;
                        say(app, true, text);
                    }
                    Err(e) => say(app, false, e),
                }
            }
        }
    }
    let Some(l) = &app.ui.rslab.loading else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(300));
    if !l.merging {
        if !l.parts.iter().all(|p| raster_exists(app, p)) {
            return;
        }
        let (first, rest) = l.parts.split_first().map(|(f, r)| (f.clone(), r.to_vec())).unwrap_or_default();
        let p = json!({
            "layer": first, "operation": "merge", "layers": rest, "nodata": 0, "scale": l.scale, "offset": l.offset,
            "bandNames": BANDS, "name": l.image,
        });
        if let Some(l) = app.ui.rslab.loading.as_mut() {
            l.merging = true;
        }
        crate::analysispane::run_quietly(app, "raster.process", p);
        return;
    }
    if !raster_exists(app, &l.image) {
        if crate::analysispane::background_job(app).is_none()
            && let Some(l) = app.ui.rslab.loading.as_mut()
        {
            l.idle += 1;
            if l.idle > 20 {
                app.ui.rslab.loading = None;
                say(app, false, tl!("The tiles could not be merged: see Messages.").to_string());
            }
        }
        return;
    }
    let (image, parts) = (l.image.clone(), l.parts.clone());
    app.ui.rslab.loading = None;
    let _ = app.run("layer.remove", json!({"layer": parts}));
    let _ = app.run("raster.bands", json!({"layer": image, "red": 3, "green": 2, "blue": 1}));
    // Clouds and bright roofs would leave the rest dark under a min–max stretch.
    let _ = app.run("raster.stretch", json!({"layer": image, "type": "percentClip", "low": 2, "high": 2}));
    let _ = app.run("layer.zoomTo", json!({"layer": image}));
    app.ui.rslab.image = image.clone();
    say(
        app,
        true,
        crate::i18n::fmt("{name} is ready: six bands of reflectance (Blue, Green, Red, NIR, SWIR1, SWIR2).", &[("name", &image)]),
    );
    app.map.invalidate();
}

fn rasters(app: &RustGisApp) -> Vec<String> {
    app.session.map().layers.iter().filter(|l| l.is_raster()).map(|l| l.name.clone()).collect()
}

fn pick(ui: &mut Ui, id: &str, label: &str, value: &mut String, options: &[String]) {
    if (value.is_empty() || !options.contains(value))
        && let Some(first) = options.first()
    {
        value.clone_from(first);
    }
    ui.horizontal(|ui| {
        ui.label(tl!(label));
        egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(190.0).show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(value, o.clone(), o.as_str());
            }
        });
    });
}

/// Runs a step's command: slow ones in the background on the desktop, their progress and Cancel
/// shown in the lab (see [`pane`]).
fn run(app: &mut RustGisApp, id: &str, p: Value) {
    if let Some((name, _)) = crate::analysispane::background_job(app) {
        let text = crate::i18n::fmt("{tool} is still running: wait for it, or cancel it.", &[("tool", tl_raw!(name))]);
        return say(app, false, text);
    }
    crate::analysispane::run_quietly(app, id, p);
    app.map.invalidate();
}

fn data_step(app: &mut RustGisApp, ui: &mut Ui) {
    ui.label(egui::RichText::new(tl!("Study area")).strong());
    ui.horizontal(|ui| {
        ui.radio_value(&mut app.ui.rslab.use_view, false, tl!("Province or district"));
        ui.radio_value(&mut app.ui.rslab.use_view, true, tl!("Map view"));
    });
    if !app.ui.rslab.use_view {
        let provs = crate::khdata::provinces();
        let st = &mut app.ui.rslab;
        let current = provs
            .iter()
            .find(|p| p.0 == st.province)
            .map(|p| crate::khdata::label(&p.0, &p.1))
            .unwrap_or_else(|| tl!("Choose…").to_string());
        egui::ComboBox::from_label(tl!("Province")).selected_text(current).width(200.0).show_ui(ui, |ui| {
            for p in &provs {
                if ui.selectable_label(st.province == p.0, crate::khdata::label(&p.0, &p.1)).clicked() {
                    st.province = p.0.clone();
                    st.district.clear();
                }
            }
        });
        let ds = crate::khdata::districts(&st.province);
        let whole = tl!("Whole province").to_string();
        let current = if st.district.is_empty() { whole.clone() } else { st.district.clone() };
        ui.add_enabled_ui(!ds.is_empty(), |ui| {
            egui::ComboBox::from_label(tl!("District")).selected_text(current).width(200.0).show_ui(ui, |ui| {
                if ui.selectable_label(st.district.is_empty(), &whole).clicked() {
                    st.district.clear();
                }
                for d in &ds {
                    if ui.selectable_label(st.district == d.0, crate::khdata::label(&d.0, &d.1)).clicked() {
                        st.district = d.0.clone();
                    }
                }
            });
        });
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Satellite")).strong());
    let st = &mut app.ui.rslab;
    ui.horizontal(|ui| {
        for (s, label) in [(Sensor::Sentinel2, "Sentinel-2 (10 m)"), (Sensor::Landsat, "Landsat 8/9 (30 m)")] {
            if ui.radio(st.sensor == s, tl!(label)).clicked() && st.sensor != s {
                st.sensor = s;
                st.cell = s.cell();
            }
        }
    });
    egui::Grid::new("rslab_dates").num_columns(2).show(ui, |ui| {
        ui.label(tl!("From"));
        ui.add(egui::TextEdit::singleline(&mut st.start).desired_width(110.0).hint_text("YYYY-MM-DD"));
        ui.end_row();
        ui.label(tl!("To"));
        ui.add(egui::TextEdit::singleline(&mut st.end).desired_width(110.0).hint_text("YYYY-MM-DD"));
        ui.end_row();
        ui.label(tl!("Cloud cover"));
        ui.add(egui::Slider::new(&mut st.cloud, 0.0..=100.0).suffix(" %").integer());
        ui.end_row();
        ui.label(tl!("Cell size"));
        ui.add(egui::DragValue::new(&mut st.cell).range(10.0..=500.0).speed(1.0).suffix(" m"));
        ui.end_row();
    });
    let searching = st.search.is_some();
    ui.horizontal(|ui| {
        if ui.add_enabled(!searching, egui::Button::new(egui::RichText::new(tl!("Search")).strong())).clicked() {
            search(app);
        }
        if searching {
            ui.spinner();
        }
    });
    if !app.ui.rslab.scenes.is_empty() {
        let st = &mut app.ui.rslab;
        egui::ScrollArea::vertical().id_salt("rslab_scenes").max_height(150.0).show(ui, |ui| {
            for (i, s) in st.scenes.iter().enumerate() {
                let text = crate::i18n::fmt(
                    "{date} · cloud {c}% · {n} tile(s)",
                    &[("date", &s.date), ("c", &format!("{:.1}", s.cloud)), ("n", &s.items.len().to_string())],
                );
                if ui.selectable_label(st.picked == Some(i), text).clicked() {
                    st.picked = Some(i);
                }
            }
        });
        let busy = app.ui.rslab.loading.is_some();
        ui.add_enabled_ui(!busy && app.ui.rslab.picked.is_some(), |ui| {
            if ui
                .button(egui::RichText::new(tl!("Download Image")).strong())
                .on_hover_text(tl!("Clipped to the study area, as one six-band reflectance image"))
                .clicked()
            {
                download(app);
            }
        });
    }
    if let Some(l) = &app.ui.rslab.loading {
        let text = if l.merging {
            tl!("Merging the tiles…").to_string()
        } else {
            let left = l.parts.iter().filter(|p| !raster_exists(app, p)).count();
            crate::i18n::fmt("Downloading… {n} left", &[("n", &left.to_string())])
        };
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(text);
            cancel = ui.small_button(tl!("Cancel")).clicked();
        });
        if cancel {
            app.ui.rslab.loading = None;
        }
    }
}

fn enhance_step(app: &mut RustGisApp, ui: &mut Ui) {
    let rs = rasters(app);
    pick(ui, "rslab_img", "Image", &mut app.ui.rslab.image, &rs);
    let image = app.ui.rslab.image.clone();
    ui.add_enabled_ui(!image.is_empty(), |ui| {
        ui.label(egui::RichText::new(tl!("Band combination")).strong());
        ui.horizontal_wrapped(|ui| {
            for (label, [r, g, b]) in [
                ("Natural color (3-2-1)", [3, 2, 1]),
                ("Color infrared (4-3-2)", [4, 3, 2]),
                ("Agriculture (5-4-1)", [5, 4, 1]),
                ("Urban (6-5-3)", [6, 5, 3]),
                ("Land/water (4-5-3)", [4, 5, 3]),
            ] {
                if ui.button(tl!(label)).clicked() {
                    let _ = app.run("raster.bands", json!({"layer": image, "red": r, "green": g, "blue": b}));
                    app.map.invalidate();
                }
            }
        });
        ui.label(egui::RichText::new(tl!("Spectral indices")).strong());
        ui.horizontal_wrapped(|ui| {
            for (idx, tip) in [
                ("ndvi", "Vegetation (NIR, Red)"),
                ("ndwi", "Water (Green, NIR)"),
                ("ndbi", "Built-up (SWIR1, NIR)"),
                ("savi", "Vegetation on bare soil"),
                ("evi", "Dense vegetation"),
                ("nbr", "Burned areas (NIR, SWIR2)"),
            ] {
                if ui.button(idx.to_uppercase()).on_hover_text(tl!(tip)).clicked() {
                    let swir = if idx == "nbr" { 6 } else { 5 };
                    let p = json!({"layer": image, "index": idx, "blue": 1, "green": 2, "red": 3, "nir": 4, "swir": swir, "name": format!("{image}_{idx}")});
                    run(app, "raster.index", p);
                }
            }
        });
    });
}

fn classify_step(app: &mut RustGisApp, ui: &mut Ui) {
    let rs = rasters(app);
    pick(ui, "rslab_cimg", "Image", &mut app.ui.rslab.image, &rs);
    let image = app.ui.rslab.image.clone();
    ui.label(egui::RichText::new(tl!("Unsupervised (K-means)")).strong());
    ui.horizontal(|ui| {
        ui.label(tl!("Classes"));
        ui.add(egui::DragValue::new(&mut app.ui.rslab.classes).range(2.0..=20.0).speed(0.1).fixed_decimals(0));
        if ui.add_enabled(!image.is_empty(), egui::Button::new(tl!("Classify"))).clicked() {
            let k = app.ui.rslab.classes.round() as u64;
            run(app, "raster.classify", json!({"layer": image, "classes": k, "name": format!("{image}_kmeans{k}")}));
        }
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Supervised (training polygons)")).strong());
    let map = app.session.map();
    let polygons: Vec<String> = map
        .layers
        .iter()
        .filter(|l| !l.is_raster() && l.geometry_type() == rustgis_engine::data::GeometryType::Polygon)
        .map(|l| l.name.clone())
        .collect();
    if polygons.is_empty() {
        ui.label(egui::RichText::new(tl!("Draw or add a polygon layer of training areas with a class field.")).weak());
        return;
    }
    pick(ui, "rslab_train", "Training", &mut app.ui.rslab.training, &polygons);
    let fields: Vec<String> = app
        .session
        .map()
        .layer_by_name(&app.ui.rslab.training)
        .map(|l| l.dataset.fields.iter().map(|f| f.name.clone()).collect())
        .unwrap_or_default();
    pick(ui, "rslab_field", "Class field", &mut app.ui.rslab.class_field, &fields);
    let (training, field) = (app.ui.rslab.training.clone(), app.ui.rslab.class_field.clone());
    if ui.add_enabled(!image.is_empty() && !field.is_empty(), egui::Button::new(tl!("Classify"))).clicked() {
        run(
            app,
            "raster.classifySupervised",
            json!({"layer": image, "training": training, "classField": field, "name": format!("{image}_supervised")}),
        );
    }
}

fn change_step(app: &mut RustGisApp, ui: &mut Ui) {
    ui.label(egui::RichText::new(tl!("Compare two dates: download two images (or make an index of each), then pick them here.")).weak());
    let rs = rasters(app);
    pick(ui, "rslab_before", "Before", &mut app.ui.rslab.before, &rs);
    pick(ui, "rslab_after", "After", &mut app.ui.rslab.after, &rs);
    ui.horizontal(|ui| {
        ui.label(tl!("Method"));
        for (m, label) in [("difference", "Difference"), ("ratio", "Ratio"), ("percent", "Percent")] {
            ui.radio_value(&mut app.ui.rslab.method, m, tl!(label));
        }
    });
    let st = &app.ui.rslab;
    let (before, after, method) = (st.before.clone(), st.after.clone(), st.method);
    if ui.add_enabled(!before.is_empty() && !after.is_empty() && before != after, egui::Button::new(tl!("Detect Change"))).clicked() {
        run(app, "raster.change", json!({"before": before, "after": after, "method": method, "name": format!("change_{method}")}));
    }
}

fn map_step(app: &mut RustGisApp, ui: &mut Ui) {
    if ui
        .button(egui::RichText::new(tl!("Auto Map Layout")).strong())
        .on_hover_text(tl!("An A4 map of the view: title, legend, scale bar, north arrow, inset and sources"))
        .clicked()
    {
        match app.run("khgis.autoLayout", json!({"lang": crate::khdata::lang()})) {
            Ok(_) => say(app, true, tl!("Map layout made: Publish › Export to save it as PDF.").to_string()),
            Err(e) => say(app, false, e),
        }
    }
    ui.add_space(4.0);
    if ui
        .button(tl!("Open in Cambodia 3D"))
        .on_hover_text(tl!("The middle of the map view as a 3D landscape with trees and houses (up to 4 × 4 km)"))
        .clicked()
        && let Err(e) = app.run("ui.cambodia3d", json!({}))
    {
        say(app, false, e);
    }
}

/// The lab's panel.
pub fn pane(app: &mut RustGisApp, ui: &mut Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        for (i, label) in ["1 Data", "2 Enhance", "3 Classify", "4 Change", "5 Map"].into_iter().enumerate() {
            if ui.add(egui::Button::selectable(app.ui.rslab.step == i, tl!(label))).clicked() {
                app.ui.rslab.step = i;
            }
        }
    });
    if let Some((name, done)) = crate::analysispane::background_job(app) {
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.spinner();
            let text = match done {
                Some(f) => format!("{} {:.0}%", tl!(name), f * 100.0),
                None => tl!(name).to_string(),
            };
            ui.label(text);
            cancel = ui.small_button(tl!("Cancel")).clicked();
        });
        if cancel {
            crate::analysispane::cancel_job(app);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    ui.separator();
    egui::ScrollArea::vertical().id_salt("rslab_step").auto_shrink([false, true]).show(ui, |ui| {
        match app.ui.rslab.step {
            0 => data_step(app, ui),
            1 => enhance_step(app, ui),
            2 => classify_step(app, ui),
            3 => change_step(app, ui),
            _ => map_step(app, ui),
        }
        ui.add_space(6.0);
        for (ok, line) in &app.ui.rslab.log {
            ui.label(egui::RichText::new(format!("{} {line}", if *ok { "✓" } else { "✗" })).color(if *ok { t.text_dim } else { t.red }));
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(tl!("Imagery: Copernicus Sentinel-2 (ESA) and Landsat (USGS/NASA), served by Microsoft Planetary Computer. After the LEE RS Lab plugin, Geoinformatics and Research Lab, RUPP."))
                .small()
                .color(t.text_dim),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_asks_for_clear_scenes_of_the_area() {
        let u = search_url(Sensor::Landsat, [104.8, 11.5, 105.0, 11.7], "2025-01-01", "2025-03-31", 20.0);
        assert!(u.starts_with(STAC), "{u}");
        assert!(u.contains("collections=landsat-c2-l2&bbox=104.80000,11.50000,105.00000,11.70000"), "{u}");
        assert!(u.contains("datetime=2025-01-01T00:00:00Z/2025-03-31T23:59:59Z"), "{u}");
        assert!(u.contains("filter=eo:cloud_cover%3C20%20AND%20platform%20IN%20(%27landsat-8%27,%27landsat-9%27)"), "{u}");
        assert!(u.ends_with("sortby=%2Beo:cloud_cover"), "{u}");
    }

    #[test]
    fn items_are_grouped_by_date_clearest_first() {
        let fc = json!({"features": [
            {"id": "a1", "properties": {"datetime": "2025-02-28T03:17:21Z", "eo:cloud_cover": 2.0, "s2:processing_baseline": "05.11"}},
            {"id": "a2", "properties": {"datetime": "2025-02-28T03:17:21Z", "eo:cloud_cover": 4.0, "s2:processing_baseline": "05.11"}},
            {"id": "b1", "properties": {"datetime": "2021-02-08T03:19:31Z", "eo:cloud_cover": 1.0, "s2:processing_baseline": "02.12"}},
        ]});
        let s = scenes(Sensor::Sentinel2, fc.to_string().as_bytes()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].date.as_str(), s[0].offset), ("2021-02-08", 0.0));
        assert_eq!((s[1].items.len(), s[1].cloud, s[1].offset), (2, 3.0, -0.1));
        assert!(scenes(Sensor::Sentinel2, br#"{"description": "bad"}"#).is_err());
    }

    #[test]
    fn images_fit_the_area_and_the_limit() {
        let (w, h, cell) = grid([104.85, 11.52, 104.95, 11.62], 10.0);
        assert!((1080..1100).contains(&w) && (1100..1120).contains(&h) && cell == 10.0, "{w} {h} {cell}");
        let (w, h, cell) = grid([102.0, 11.0, 104.0, 13.0], 10.0);
        assert!(w <= MAX_SIDE && h == MAX_SIDE && cell > 80.0, "{w} {h} {cell}");
        let u = image_url(Sensor::Sentinel2, "S2C_X", [104.85, 11.52, 104.95, 11.62], w, h);
        assert!(
            u.contains("/104.850000,11.520000,104.950000,11.620000/")
                && u.contains("&assets=B02&assets=B03&assets=B04&assets=B08&assets=B11&assets=B12"),
            "{u}"
        );
    }

    #[test]
    fn cambodia_3d_takes_at_most_four_km() {
        let u = cambodia3d_url([104.0, 11.0, 106.0, 13.0]);
        let b: Vec<f64> = u.split("aoi=").nth(1).unwrap().split(',').map(|v| v.parse().unwrap()).collect();
        assert!((b[3] - b[1]) * 111.32 <= 4.01 && (b[2] - b[0]) * 111.32 * 12f64.to_radians().cos() <= 4.01, "{u}");
        assert!(((b[0] + b[2]) / 2.0 - 105.0).abs() < 1e-4);
    }
}
