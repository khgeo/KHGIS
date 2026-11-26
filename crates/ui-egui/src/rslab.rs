//! LEE RS Lab: KHGIS's built-in remote sensing lab, after the LEE RS Lab QGIS plugin, for
//! teaching remote sensing online, no QGIS needed. Five steps:
//!
//! 1. **Data**: a study area (a province, district or commune, the map view or a polygon layer),
//!    then imagery or another dataset over it: Sentinel-2, Landsat 8–9 or 4–7 (clouds and shadows
//!    masked, one date or a median composite of the clearest), Sentinel-1 radar, the Copernicus
//!    DEM, ESA WorldCover, Esri land cover or JRC surface water, clipped to the area.
//! 2. **Enhance**: band combinations, stretches, indices, band math, filters, principal
//!    components and a feature stack (indices, GLCM texture and other layers) for classification.
//! 3. **Classify**: k-means and ISODATA (clusters named after), seven supervised classifiers
//!    trained from polygons or a land cover map, clean-up (majority filter and sieve), class areas,
//!    accuracy points and an accuracy assessment with area-adjusted estimates.
//! 4. **Change**: a from → to matrix with gains and losses and a Sankey diagram, and image
//!    differencing.
//! 5. **Map**: the A4 layout with a title made from the area and date, and Cambodia 3D.
//!
//! The data come from Microsoft's Planetary Computer, with no login: its STAC search, and its data
//! API's `item/{bbox}` GeoTIFFs, which clip and stack an item's assets on the server and answer
//! the browser, so the web app reads them too. Each item is downloaded on one grid and the pieces
//! merged (`raster.process` "merge"), clouds masked and numbers made reflectance on the way. A
//! run of steps (downloads, merges, indices…) goes one after another ([`Chain`]), each waiting for
//! the layer the one before makes.

use egui::Ui;
use rustgis_engine::render::rustgis_tiles::cache;
use serde_json::{Value, json};

use crate::{RustGisApp, Slot};

pub(crate) type Answer = Slot<Result<Vec<u8>, String>>;

/// The Planetary Computer's STAC search and data API.
const STAC: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/search";
const DATA: &str = "https://planetarycomputer.microsoft.com/api/data/v1/item/bbox";
/// The longest side of an image downloaded, in cells: past it the cells are made larger.
pub(crate) const MAX_SIDE: u32 = 2500;
/// The most items downloaded for one image (tiles × dates).
const MAX_ITEMS: usize = 12;
/// The bands of an optical image, in order (1 to 6), as both satellites name them.
pub(crate) const BANDS: [&str; 6] = ["Blue", "Green", "Red", "NIR", "SWIR1", "SWIR2"];

/// ESA WorldCover's classes.
const WORLDCOVER: &[(u32, &str)] = &[
    (10, "Tree cover"),
    (20, "Shrubland"),
    (30, "Grassland"),
    (40, "Cropland"),
    (50, "Built-up"),
    (60, "Bare / sparse vegetation"),
    (70, "Snow and ice"),
    (80, "Permanent water bodies"),
    (90, "Herbaceous wetland"),
    (95, "Mangroves"),
    (100, "Moss and lichen"),
];
/// Esri | Impact Observatory land cover's classes.
const ESRI_LULC: &[(u32, &str)] = &[
    (1, "Water"),
    (2, "Trees"),
    (4, "Flooded vegetation"),
    (5, "Crops"),
    (7, "Built area"),
    (8, "Bare ground"),
    (9, "Snow/ice"),
    (10, "Clouds"),
    (11, "Rangeland"),
];

/// What the data step downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dataset {
    Sentinel2,
    Landsat89,
    Landsat47,
    Sentinel1,
    Dem,
    WorldCover,
    EsriLandCover,
    Water,
}

impl Dataset {
    const ALL: [Dataset; 8] = [
        Dataset::Sentinel2,
        Dataset::Landsat89,
        Dataset::Landsat47,
        Dataset::Sentinel1,
        Dataset::Dem,
        Dataset::WorldCover,
        Dataset::EsriLandCover,
        Dataset::Water,
    ];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Dataset::Sentinel2 => "Sentinel-2 (10 m, 2017–)",
            Dataset::Landsat89 => "Landsat 8–9 (30 m, 2013–)",
            Dataset::Landsat47 => "Landsat 4–7 (30 m, 1982–2022)",
            Dataset::Sentinel1 => "Sentinel-1 radar VV, VH (10 m)",
            Dataset::Dem => "Copernicus DEM (30 m elevation)",
            Dataset::WorldCover => "ESA WorldCover land cover (10 m)",
            Dataset::EsriLandCover => "Esri land cover (10 m, 2017–2023)",
            Dataset::Water => "JRC surface water occurrence (30 m)",
        }
    }
    fn collection(self) -> &'static str {
        match self {
            Dataset::Sentinel2 => "sentinel-2-l2a",
            Dataset::Landsat89 | Dataset::Landsat47 => "landsat-c2-l2",
            Dataset::Sentinel1 => "sentinel-1-rtc",
            Dataset::Dem => "cop-dem-glo-30",
            Dataset::WorldCover => "esa-worldcover",
            Dataset::EsriLandCover => "io-lulc-annual-v02",
            Dataset::Water => "jrc-gsw",
        }
    }
    /// The assets read, in band order; a quality band last when clouds are masked.
    fn assets(self, mask: bool) -> Vec<&'static str> {
        let mut a = match self {
            Dataset::Sentinel2 => vec!["B02", "B03", "B04", "B08", "B11", "B12"],
            Dataset::Landsat89 | Dataset::Landsat47 => vec!["blue", "green", "red", "nir08", "swir16", "swir22"],
            Dataset::Sentinel1 => vec!["vv", "vh"],
            Dataset::Dem => vec!["data"],
            Dataset::WorldCover => vec!["map"],
            Dataset::EsriLandCover => vec!["data"],
            Dataset::Water => vec!["occurrence"],
        };
        if mask && self.optical() {
            a.push(if self == Dataset::Sentinel2 { "SCL" } else { "qa_pixel" });
        }
        a
    }
    fn band_names(self) -> Vec<&'static str> {
        match self {
            Dataset::Sentinel2 | Dataset::Landsat89 | Dataset::Landsat47 => BANDS.to_vec(),
            Dataset::Sentinel1 => vec!["VV dB", "VH dB"],
            Dataset::Dem => vec!["Elevation"],
            Dataset::WorldCover | Dataset::EsriLandCover => vec!["Class"],
            Dataset::Water => vec!["Water occurrence %"],
        }
    }
    pub(crate) fn optical(self) -> bool {
        matches!(self, Dataset::Sentinel2 | Dataset::Landsat89 | Dataset::Landsat47)
    }
    /// Searched by date (else every item over the area).
    fn dated(self) -> bool {
        self.optical() || self == Dataset::Sentinel1
    }
    pub(crate) fn cell(self) -> f64 {
        match self {
            Dataset::Sentinel2 | Dataset::Sentinel1 | Dataset::WorldCover | Dataset::EsriLandCover => 10.0,
            _ => 30.0,
        }
    }
    pub(crate) fn classes(self) -> &'static [(u32, &'static str)] {
        match self {
            Dataset::WorldCover => WORLDCOVER,
            Dataset::EsriLandCover => ESRI_LULC,
            _ => &[],
        }
    }
    /// The stored value that is no data.
    fn nodata(self) -> Option<f64> {
        match self {
            Dataset::Water => Some(255.0),
            Dataset::Dem => None,
            _ => Some(0.0),
        }
    }
    pub(crate) fn short(self) -> &'static str {
        match self {
            Dataset::Sentinel2 => "S2",
            Dataset::Landsat89 => "L8-9",
            Dataset::Landsat47 => "L4-7",
            Dataset::Sentinel1 => "S1",
            Dataset::Dem => "DEM",
            Dataset::WorldCover => "WorldCover",
            Dataset::EsriLandCover => "EsriLC",
            Dataset::Water => "Water",
        }
    }
}

/// A date (or, for undated datasets, a year or the whole set) the search found: its items over
/// the area, and how cloudy they are.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub date: String,
    pub items: Vec<String>,
    pub cloud: f64,
    /// Stored number × scale + offset is reflectance.
    pub scale: f64,
    pub offset: f64,
}

/// One step of a run: a command, and the layer it makes (waited for before the next).
#[derive(Clone, Debug)]
pub(crate) struct Step {
    command: String,
    params: Value,
    makes: Option<String>,
    /// Run in the background on the desktop (with its progress and Cancel in the lab).
    background: bool,
    /// Its answer kept (shown in the lab).
    keep: Option<&'static str>,
}

pub(crate) fn step(command: &str, params: Value) -> Step {
    Step { command: command.into(), params, makes: None, background: false, keep: None }
}

pub(crate) fn makes(command: &str, params: Value, layer: &str) -> Step {
    Step { command: command.into(), params, makes: Some(layer.into()), background: true, keep: None }
}

/// Steps run one after another (see the module's doc).
pub(crate) struct Chain {
    title: String,
    steps: std::collections::VecDeque<Step>,
    waiting: Option<(Step, u32)>,
    done: usize,
    total: usize,
    finished: String,
}

/// The lab's choices, results and the work under way.
pub struct State {
    pub step: usize,
    /// The study area: 0 a province/district/commune, 1 the map view, 2 a polygon layer.
    pub area_kind: usize,
    pub province: String,
    pub district: String,
    pub commune: String,
    pub area_layer: String,
    pub clip: bool,
    /// The district's communes: (English name, Khmer name, kind in Khmer, feature).
    pub(crate) communes: Option<Vec<(String, String, String, Value)>>,
    pub dataset: Dataset,
    pub start: String,
    pub end: String,
    pub cloud: f64,
    pub cell: f64,
    pub mask: bool,
    pub composite: f64,
    pub slope: bool,
    pub(crate) search: Option<(Answer, Dataset)>,
    pub scenes: Vec<Scene>,
    pub scenes_of: Dataset,
    pub picked: Option<usize>,
    pub(crate) chain: Option<Chain>,
    /// LEE scripts (tab 5).
    pub lee: crate::leelab::Lee,
    /// The magic wand's tolerance and the class it draws; the spectral profiles picked.
    pub wand_tolerance: f64,
    pub wand_class: String,
    pub profiles: Vec<(String, Vec<(String, f64)>)>,
    pub hist_band: usize,
    pub fs_x: usize,
    pub fs_y: usize,
    /// Multi-season stack: the year and which seasons; the searches under way.
    pub season_year: f64,
    pub seasons: [bool; 3],
    pub(crate) season_search: Vec<(usize, String, Answer)>,
    pub(crate) season_found: Vec<(usize, Vec<Scene>)>,
    /// The image the later steps work on.
    pub image: String,
    pub rgb: [u32; 3],
    pub expression: String,
    pub filter: &'static str,
    pub filter_size: f64,
    pub components: f64,
    pub stack_indices: bool,
    pub stack_texture: bool,
    pub stack_extra: String,
    pub unsupervised: &'static str,
    pub classes: f64,
    pub min_classes: f64,
    pub class_map: String,
    pub class_labels: Vec<(f32, String)>,
    pub training_from_map: bool,
    pub training: String,
    pub class_field: String,
    pub training_map: String,
    pub method: &'static str,
    pub option: f64,
    pub per_class: f64,
    pub majority: f64,
    pub sieve: f64,
    pub points: f64,
    pub reference: String,
    pub ref_field: String,
    pub before: String,
    pub after: String,
    pub diff_method: &'static str,
    pub title: String,
    /// Answers kept to show: training, pca, crossTab, accuracy, areas, merge, sieve.
    pub results: Vec<(&'static str, Value)>,
    pub log: Vec<(bool, String)>,
}

impl Default for State {
    fn default() -> Self {
        State {
            step: 0,
            area_kind: 0,
            province: String::new(),
            district: String::new(),
            commune: String::new(),
            area_layer: String::new(),
            clip: true,
            communes: None,
            dataset: Dataset::Sentinel2,
            start: crate::stacui::days_ago(180),
            end: crate::stacui::days_ago(0),
            cloud: 30.0,
            cell: 10.0,
            mask: true,
            composite: 1.0,
            slope: true,
            search: None,
            scenes: Vec::new(),
            scenes_of: Dataset::Sentinel2,
            picked: None,
            chain: None,
            lee: Default::default(),
            wand_tolerance: 1.5,
            wand_class: String::new(),
            profiles: Vec::new(),
            hist_band: 0,
            fs_x: 2,
            fs_y: 3,
            season_year: 2025.0,
            seasons: [true, true, true],
            season_search: Vec::new(),
            season_found: Vec::new(),
            image: String::new(),
            rgb: [3, 2, 1],
            expression: "(b4 - b3) / (b4 + b3)".into(),
            filter: "median",
            filter_size: 3.0,
            components: 3.0,
            stack_indices: true,
            stack_texture: true,
            stack_extra: String::new(),
            unsupervised: "kmeans",
            classes: 6.0,
            min_classes: 4.0,
            class_map: String::new(),
            class_labels: Vec::new(),
            training_from_map: false,
            training: String::new(),
            class_field: String::new(),
            training_map: String::new(),
            method: "randomForest",
            option: 50.0,
            per_class: 1000.0,
            majority: 3.0,
            sieve: 8.0,
            points: 30.0,
            reference: String::new(),
            ref_field: String::new(),
            before: String::new(),
            after: String::new(),
            diff_method: "difference",
            title: String::new(),
            results: Vec::new(),
            log: Vec::new(),
        }
    }
}

/// The STAC search for `dataset` over `bbox`: by date and cloud for imagery.
pub(crate) fn search_url(dataset: Dataset, bbox: [f64; 4], start: &str, end: &str, cloud: f64) -> String {
    let enc = |s: &str| s.replace('%', "%25").replace(' ', "%20").replace('<', "%3C").replace('\'', "%27").replace('+', "%2B");
    let mut u =
        format!("{STAC}?collections={}&bbox={:.5},{:.5},{:.5},{:.5}&limit=200", dataset.collection(), bbox[0], bbox[1], bbox[2], bbox[3]);
    if dataset.dated() {
        u.push_str(&format!("&datetime={}T00:00:00Z/{}T23:59:59Z", start.trim(), end.trim()));
    }
    let mut filter = Vec::new();
    if dataset.optical() {
        filter.push(format!("eo:cloud_cover<{}", cloud.clamp(0.0, 100.0)));
    }
    match dataset {
        Dataset::Landsat89 => filter.push("platform IN ('landsat-8','landsat-9')".into()),
        Dataset::Landsat47 => filter.push("platform IN ('landsat-4','landsat-5','landsat-7')".into()),
        _ => {}
    }
    if !filter.is_empty() {
        u.push_str(&format!("&filter={}&filter-lang=cql2-text", enc(&filter.join(" AND "))));
    }
    if dataset.optical() {
        u.push_str(&format!("&sortby={}", enc("+eo:cloud_cover")));
    }
    u
}

/// The search's items as dates (or years): each date's items together, the clearest first.
pub(crate) fn scenes(dataset: Dataset, bytes: &[u8]) -> Result<Vec<Scene>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("the search's answer isn't JSON: {e}"))?;
    let features = v["features"].as_array().ok_or_else(|| {
        let why = v["description"].as_str().or_else(|| v["detail"].as_str()).unwrap_or("no features");
        format!("the search found nothing: {why}")
    })?;
    let mut out: Vec<(Scene, usize)> = Vec::new();
    for f in features {
        let Some(id) = f["id"].as_str() else { continue };
        let p = &f["properties"];
        let key: String = if dataset.dated() {
            p["datetime"].as_str().unwrap_or("").chars().take(10).collect()
        } else if matches!(dataset, Dataset::WorldCover | Dataset::EsriLandCover) {
            // The year, from the item's dates or its id.
            p["start_datetime"]
                .as_str()
                .map(|d| d.chars().take(4).collect())
                .or_else(|| id.split(['_', '-']).find(|t| t.len() == 4 && t.starts_with("20")).map(str::to_string))
                .unwrap_or_else(|| "—".into())
        } else {
            "—".into()
        };
        let cloud = p["eo:cloud_cover"].as_f64().unwrap_or(0.0);
        let (scale, offset) = match dataset {
            // From processing baseline 04.00 (2022) the stored numbers have 1000 added.
            Dataset::Sentinel2 => {
                let baseline = p["s2:processing_baseline"].as_str().and_then(|b| b.parse::<f64>().ok()).unwrap_or(5.0);
                (0.0001, if baseline >= 4.0 { -0.1 } else { 0.0 })
            }
            Dataset::Landsat89 | Dataset::Landsat47 => (0.0000275, -0.2),
            _ => (1.0, 0.0),
        };
        match out.iter_mut().find(|(s, _)| s.date == key) {
            Some((s, n)) => {
                if !s.items.iter().any(|i| i == id) {
                    s.items.push(id.to_string());
                    s.cloud += cloud;
                    *n += 1;
                }
            }
            None => out.push((Scene { date: key, items: vec![id.to_string()], cloud, scale, offset }, 1)),
        }
    }
    let mut out: Vec<Scene> = out
        .into_iter()
        .map(|(mut s, n)| {
            s.cloud /= n as f64;
            s
        })
        .collect();
    if dataset.optical() {
        out.sort_by(|a, b| a.cloud.total_cmp(&b.cloud).then(b.date.cmp(&a.date)));
    } else {
        out.sort_by(|a, b| b.date.cmp(&a.date));
    }
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

/// One item's image over `bbox`: its assets, clipped and stacked by the data API.
pub(crate) fn image_url(dataset: Dataset, item: &str, bbox: [f64; 4], w: u32, h: u32, mask: bool) -> String {
    let assets: String = dataset.assets(mask).iter().map(|a| format!("&assets={a}")).collect();
    format!(
        "{DATA}/{:.6},{:.6},{:.6},{:.6}/{w}x{h}.tif?collection={}&item={item}{assets}&return_mask=false",
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3],
        dataset.collection()
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

/// The study area: its box, a short name (for layer names), its place name in the interface
/// language (for titles), and its shape as a GeoJSON feature when it has one of its own.
pub(crate) struct Area {
    pub(crate) bbox: [f64; 4],
    pub(crate) short: String,
    pub(crate) place: String,
    pub(crate) shape: Option<Value>,
}

pub(crate) fn area(app: &RustGisApp) -> Result<Area, String> {
    let st = &app.ui.rslab;
    let km = crate::khdata::khmer();
    let need = || tl!("Choose a province (or the map view) first.").to_string();
    match st.area_kind {
        1 => {
            let e = rustgis_engine::cmd::workspace::view_extent(&app.session);
            if e.is_empty() {
                return Err(tl!("The map has no view yet.").to_string());
            }
            Ok(Area {
                bbox: [e.min_x.max(-180.0), e.min_y.max(-90.0), e.max_x.min(180.0), e.max_y.min(90.0)],
                short: "view".into(),
                place: String::new(),
                shape: None,
            })
        }
        2 => {
            let l = app.session.map().layer_by_name(&st.area_layer).ok_or_else(|| tl!("Choose a polygon layer.").to_string())?;
            let e = l.dataset.extent();
            if e.is_empty() {
                return Err(tl!("Choose a polygon layer.").to_string());
            }
            Ok(Area { bbox: [e.min_x, e.min_y, e.max_x, e.max_y], short: l.name.replace(' ', "_"), place: l.name.clone(), shape: None })
        }
        _ => {
            if st.province.is_empty() {
                return Err(need());
            }
            let prov = crate::khdata::provinces().into_iter().find(|p| p.0 == st.province).ok_or_else(need)?;
            let prov_place = if km {
                if prov.0 == "Phnom Penh" { format!("រាជធានី{}", prov.1) } else { format!("ខេត្ត{}", prov.1) }
            } else {
                prov.0.clone()
            };
            let commune = st.communes.as_ref().and_then(|c| c.iter().find(|c| c.0 == st.commune)).filter(|_| !st.commune.is_empty());
            let district = (!st.district.is_empty())
                .then(|| crate::khdata::districts(&st.province).into_iter().find(|d| d.0 == st.district))
                .flatten();
            let kind = |f: &Value, d: &str| f["properties"]["type_km"].as_str().filter(|k| !k.is_empty()).unwrap_or(d).to_string();
            let (name, feature, place) = match (commune, district) {
                (Some(c), Some(d)) => {
                    let place = if km {
                        format!("{}{} {}{} {prov_place}", c.2, c.1, kind(&d.3, "ស្រុក"), d.1)
                    } else {
                        format!("{} commune, {} district, {}", c.0, d.0, prov.0)
                    };
                    (c.0.clone(), c.3.clone(), place)
                }
                (_, Some(d)) => {
                    let place = if km { format!("{}{} {prov_place}", kind(&d.3, "ស្រុក"), d.1) } else { format!("{} district, {}", d.0, prov.0) };
                    (d.0.clone(), d.3.clone(), place)
                }
                _ => (prov.0.clone(), prov.3.clone(), prov_place),
            };
            let bbox = crate::khdata::bbox(&feature).ok_or_else(need)?;
            Ok(Area { bbox, short: name.replace(' ', "_"), place, shape: Some(feature) })
        }
    }
}

pub(crate) fn fetch(url: &str) -> Answer {
    let slot: Answer = Slot::default();
    let land = slot.clone();
    cache::download(url, Box::new(move |r| *land.lock().unwrap_or_else(|e| e.into_inner()) = Some(r)));
    slot
}

pub(crate) fn say(app: &mut RustGisApp, ok: bool, line: String) {
    let log = &mut app.ui.rslab.log;
    log.push((ok, line));
    if log.len() > 10 {
        log.remove(0);
    }
}

pub(crate) fn keep_result(app: &mut RustGisApp, key: &'static str, v: Value) {
    app.ui.rslab.results.retain(|r| r.0 != key);
    app.ui.rslab.results.push((key, v));
}

pub(crate) fn result<'a>(app: &'a RustGisApp, key: &str) -> Option<&'a Value> {
    app.ui.rslab.results.iter().find(|r| r.0 == key).map(|r| &r.1)
}

pub(crate) fn start_chain(app: &mut RustGisApp, title: String, steps: Vec<Step>, finished: String) {
    if app.ui.rslab.chain.is_some() || crate::analysispane::background_job(app).is_some() {
        return say(app, false, tl!("Another step is still running: wait for it, or cancel it.").to_string());
    }
    let total = steps.len();
    app.ui.rslab.chain = Some(Chain { title, steps: steps.into(), waiting: None, done: 0, total, finished });
}

pub(crate) fn layer_ready(app: &RustGisApp, name: &str) -> bool {
    app.session.map().layer_by_name(name).is_some_and(|l| l.raster.as_ref().is_none_or(|r| r.data.is_some()))
}

/// Moves the run of steps on: lands what a step made, starts the next.
pub(crate) fn drive(app: &mut RustGisApp) {
    let Some(mut chain) = app.ui.rslab.chain.take() else { return };
    if let Some((s, idle)) = chain.waiting.take() {
        if s.makes.as_deref().is_some_and(|n| layer_ready(app, n)) {
            if let Some(key) = s.keep
                && let Some(v) = app.workflow.result.clone()
            {
                keep_result(app, key, v);
            }
            chain.done += 1;
        } else {
            // A background step that stopped without its layer failed (its error was said).
            let running = crate::analysispane::background_job(app).is_some();
            if let Some(e) = app.workflow.error.take() {
                return say(app, false, e);
            }
            let idle = if s.background && !running { idle + 1 } else { 0 };
            if idle > 20 {
                return say(app, false, crate::i18n::fmt("{step} stopped: see Messages.", &[("step", &s.command)]));
            }
            chain.waiting = Some((s, idle));
            app.ui.rslab.chain = Some(chain);
            return;
        }
    }
    while let Some(s) = chain.steps.pop_front() {
        app.workflow.error = None;
        app.workflow.result = None;
        if s.background {
            crate::analysispane::run_quietly(app, &s.command, s.params.clone());
        } else {
            match app.run(&s.command, s.params.clone()) {
                Ok(v) => {
                    if let Some(key) = s.keep {
                        keep_result(app, key, v);
                    }
                }
                Err(e) => return say(app, false, e),
            }
        }
        if s.makes.is_some() {
            if let Some(e) = app.workflow.error.take() {
                return say(app, false, e);
            }
            chain.waiting = Some((s, 0));
            app.ui.rslab.chain = Some(chain);
            return;
        }
        chain.done += 1;
    }
    app.map.invalidate();
    let finished = std::mem::take(&mut chain.finished);
    if !finished.is_empty() {
        say(app, true, finished);
    }
}

pub(crate) fn search(app: &mut RustGisApp) {
    let a = match area(app) {
        Ok(a) => a,
        Err(e) => return say(app, false, e),
    };
    let st = &mut app.ui.rslab;
    let url = search_url(st.dataset, a.bbox, &st.start, &st.end, st.cloud);
    st.search = Some((fetch(&url), st.dataset));
    st.scenes.clear();
    st.picked = None;
}

/// Downloads the date picked (or a composite of the clearest dates), each item on the area's
/// grid, then merges them, clips the image to the area's shape and styles it.
pub(crate) fn download(app: &mut RustGisApp) {
    let a = match area(app) {
        Ok(a) => a,
        Err(e) => return say(app, false, e),
    };
    let st = &app.ui.rslab;
    let Some(first) = st.picked.and_then(|i| st.scenes.get(i)).cloned() else {
        return say(app, false, tl!("Search, then pick a date.").to_string());
    };
    let ds = st.scenes_of;
    let composite = if ds.dated() { st.composite.round().clamp(1.0, 8.0) as usize } else { 1 };
    // The picked date, then the clearest others, for a composite.
    let mut chosen = vec![first.clone()];
    chosen.extend(st.scenes.iter().filter(|s| s.date != first.date).take(composite - 1).cloned());
    let mask = st.mask && ds.optical();
    let (w, h, cell) = grid(a.bbox, st.cell);
    let date = if chosen.len() > 1 {
        let mut d: Vec<&str> = chosen.iter().map(|s| s.date.as_str()).collect();
        d.sort_unstable();
        format!("{}_to_{}_median", d.first().unwrap_or(&""), d.last().unwrap_or(&""))
    } else {
        first.date.replace('—', "all")
    };
    let image = fresh(app, &format!("{}_{}_{}", ds.short(), date, a.short));
    let items: Vec<(String, f64, f64)> = chosen.iter().flat_map(|s| s.items.iter().map(move |i| (i.clone(), s.scale, s.offset))).collect();
    let clip = st.clip && (a.shape.is_some() || st.area_kind == 2);
    let area_layer = if a.shape.is_some() { String::new() } else { st.area_layer.clone() };
    let slope = st.slope;
    if cell > st.cell * 1.01 {
        say(
            app,
            true,
            crate::i18n::fmt(
                "The area is large: cells of {m} m (at most {n} across).",
                &[("m", &format!("{cell:.0}")), ("n", &MAX_SIDE.to_string())],
            ),
        );
    }
    if items.len() > MAX_ITEMS {
        say(
            app,
            false,
            crate::i18n::fmt(
                "{n} items cover the area; the first {k} are read.",
                &[("n", &items.len().to_string()), ("k", &MAX_ITEMS.to_string())],
            ),
        );
    }
    let mut steps = image_steps(app, ds, &items, &a, &image, (w, h), mask, chosen.len() > 1, clip.then_some(area_layer.as_str()));
    if ds.optical() {
        steps.push(step("raster.bands", json!({"layer": image, "red": 3, "green": 2, "blue": 1})));
        steps.push(step("raster.stretch", json!({"layer": image, "type": "percentClip", "low": 2, "high": 2})));
    } else if ds == Dataset::Sentinel1 {
        steps.push(step("raster.bands", json!({"layer": image, "red": 1, "green": 2, "blue": 1})));
        steps.push(step("raster.stretch", json!({"layer": image, "type": "percentClip", "low": 2, "high": 2})));
    }
    let classes = ds.classes();
    if !classes.is_empty() {
        let names: serde_json::Map<String, Value> = classes.iter().map(|(v, n)| (v.to_string(), json!(n))).collect();
        steps.push(step("raster.classNames", json!({"layer": image, "names": names})));
        steps.push(step("raster.symbology", json!({"layer": image, "type": "unique"})));
    }
    if ds == Dataset::Dem && slope {
        let s = format!("{image}_slope");
        steps.push(makes("raster.function", json!({"layer": image, "function": "slope", "name": s}), &s));
    }
    steps.push(step("layer.zoomTo", json!({"layer": image})));
    let finished = if ds.optical() {
        crate::i18n::fmt("{name} is ready: six bands of reflectance (Blue, Green, Red, NIR, SWIR1, SWIR2).", &[("name", &image)])
    } else {
        crate::i18n::fmt("{name} is ready.", &[("name", &image)])
    };
    // Titles follow the area and the date.
    let year: String = date.chars().take(4).collect();
    let title = auto_title(&a.place, &year, !classes.is_empty(), crate::khdata::khmer());
    let st = &mut app.ui.rslab;
    if classes.is_empty() {
        st.image = image.clone();
    } else {
        st.class_map = image.clone();
        st.training_map = image.clone();
    }
    if ds.optical() {
        st.rgb = [3, 2, 1];
    }
    st.title = title;
    start_chain(app, tl!("Downloading imagery").to_string(), steps, finished);
}

/// The steps that download `items` (id, scale, offset) of `ds` over the area on a `w` × `h` grid,
/// merge them into `image` (the median when `median`; clouds masked when `mask`), clip it to the
/// area's shape when `clip` (the area's own, or a polygon layer named) and take away the pieces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn image_steps(app: &RustGisApp, ds: Dataset, items: &[(String, f64, f64)], a: &Area, image: &str, (w, h): (u32, u32), mask: bool, median: bool, clip: Option<&str>) -> Vec<Step> {
    let full = format!("{image}_full");
    let clip_layer = clip.map(str::to_string);
    let clip = clip.is_some();
    let chosen_many = median;
    let (scale, offset) = items.first().map_or((1.0, 0.0), |i| (i.1, i.2));
    let mut steps = Vec::new();
    let mut parts = Vec::new();
    for (k, (item, ..)) in items.iter().take(MAX_ITEMS).enumerate() {
        let part = format!("{image}_part{}", k + 1);
        let url = image_url(ds, item, a.bbox, w, h, mask);
        steps.push(Step {
            command: "ui.addUrl".into(),
            params: json!({"url": url, "name": part, "whole": true}),
            makes: Some(part.clone()),
            background: false,
            keep: None,
        });
        parts.push(part);
    }
    let merged = if clip { full.clone() } else { image.to_string() };
    let (first_part, rest) = parts.split_first().map(|(f, r)| (f.clone(), r.to_vec())).unwrap_or_default();
    let mut merge = json!({
        "layer": first_part, "operation": "merge", "layers": rest, "scale": scale, "offset": offset,
        "bandNames": ds.band_names(), "combine": if chosen_many { "median" } else { "first" }, "name": merged,
    });
    if let Some(n) = ds.nodata() {
        merge["nodata"] = json!(n);
    }
    if mask {
        merge["maskBand"] = json!(7);
        if ds == Dataset::Sentinel2 {
            // Scene classes: no data, saturated, cloud shadow, medium and high cloud, cirrus.
            merge["maskValues"] = json!([0, 1, 3, 8, 9, 10]);
        } else {
            // QA_PIXEL bits: fill, dilated cloud, cirrus, cloud, cloud shadow.
            merge["maskBits"] = json!([0, 1, 2, 3, 4]);
        }
    }
    if ds == Dataset::Sentinel1 {
        merge["decibels"] = json!(true);
    }
    let mut m = makes("raster.process", merge, &merged);
    m.keep = Some("merge");
    steps.push(m);
    let mut leftovers = parts.clone();
    if clip {
        let polygon = match &a.shape {
            Some(f) => {
                let layer = format!("AOI_{}", a.short);
                let fc = json!({"type": "FeatureCollection", "features": [f]});
                if app.session.map().layer_by_name(&layer).is_none() {
                    steps.push(step(
                        "layer.addDataText",
                        json!({"fileName": format!("{layer}.geojson"), "text": fc.to_string(), "name": layer}),
                    ));
                }
                layer
            }
            None => clip_layer.clone().unwrap_or_default(),
        };
        steps.push(makes(
            "raster.process",
            json!({"layer": full, "operation": "clip", "polygon": polygon, "selectedOnly": false, "name": image}),
            image,
        ));
        leftovers.push(full.clone());
    }
    steps.push(step("layer.remove", json!({"layer": leftovers})));
    steps
}

/// A map's title from its place and year, as LEE RS Lab makes it.
pub(crate) fn auto_title(place: &str, year: &str, land_cover: bool, km: bool) -> String {
    let year_ok = year.len() == 4 && year.chars().all(|c| c.is_ascii_digit());
    if km {
        let digits: String = year.chars().map(|c| c.to_digit(10).and_then(|d| char::from_u32(0x17E0 + d)).unwrap_or(c)).collect();
        let mut t = if land_cover {
            "ផែនទីការប្រើប្រាស់ដី និងគម្របដី".to_string()
        } else {
            "ផែនទីរូបភាពផ្កាយរណប".to_string()
        };
        if !place.is_empty() {
            t.push_str(&format!(" {place}"));
        }
        if year_ok {
            t.push_str(&format!(" ឆ្នាំ{digits}"));
        }
        t
    } else {
        let mut t = if land_cover { "Land use and land cover map".to_string() } else { "Satellite image map".to_string() };
        if !place.is_empty() {
            t.push_str(&format!(" of {place}"));
        }
        if year_ok {
            t.push_str(&format!(", {year}"));
        }
        t
    }
}

/// Lands the search, and moves the run of steps on.
pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    if let Some((slot, ds)) = &app.ui.rslab.search {
        let ds = *ds;
        let landed = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        match landed {
            None => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            Some(r) => {
                app.ui.rslab.search = None;
                match r.and_then(|b| scenes(ds, &b)) {
                    Ok(found) => {
                        let text = crate::i18n::fmt("{n} date(s) found.", &[("n", &found.len().to_string())]);
                        app.ui.rslab.picked = (!found.is_empty()).then_some(0);
                        app.ui.rslab.scenes = found;
                        app.ui.rslab.scenes_of = ds;
                        say(app, true, text);
                    }
                    Err(e) => say(app, false, e),
                }
            }
        }
    }
    if app.ui.rslab.chain.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(300));
        drive(app);
    }
}

pub(crate) fn want_communes(app: &mut RustGisApp) {
    let st = &mut app.ui.rslab;
    st.commune.clear();
    st.communes = (!st.district.is_empty()).then(|| crate::khdata::communes(&st.province, &st.district));
}

pub(crate) fn rasters(app: &RustGisApp) -> Vec<String> {
    app.session.map().layers.iter().filter(|l| l.is_raster()).map(|l| l.name.clone()).collect()
}

pub(crate) fn vectors(app: &RustGisApp, polygons_only: bool) -> Vec<String> {
    use rustgis_engine::data::GeometryType;
    app.session
        .map()
        .layers
        .iter()
        .filter(|l| !l.is_raster() && (!polygons_only || l.geometry_type() == GeometryType::Polygon))
        .map(|l| l.name.clone())
        .collect()
}

pub(crate) fn fields(app: &RustGisApp, layer: &str) -> Vec<String> {
    app.session.map().layer_by_name(layer).map(|l| l.dataset.fields.iter().map(|f| f.name.clone()).collect()).unwrap_or_default()
}

pub(crate) fn band_count(app: &RustGisApp, layer: &str) -> usize {
    app.session.map().layer_by_name(layer).and_then(|l| l.raster.as_ref()?.data.as_ref().map(|d| d.bands.len())).unwrap_or(0)
}

/// A name no layer has yet: `base`, or `base_2`, `base_3`…
pub(crate) fn fresh(app: &RustGisApp, base: &str) -> String {
    let taken = |n: &str| app.session.map().layer_by_name(n).is_some();
    if !taken(base) {
        return base.to_string();
    }
    (2..1000).map(|i| format!("{base}_{i}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{base}_new"))
}

/// A layer chooser. An empty choice takes the first; a choice not (or not yet) on the map is kept.
pub(crate) fn pick(ui: &mut Ui, id: &str, label: &str, value: &mut String, options: &[String]) {
    if value.is_empty()
        && let Some(first) = options.first()
    {
        value.clone_from(first);
    }
    ui.horizontal(|ui| {
        ui.label(tl!(label));
        egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(190.0).truncate().show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(value, o.clone(), o.as_str());
            }
        });
    });
}

pub(crate) fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!(text)).strong());
}

/// Runs one command that makes `layer`, as a run of one step.
pub(crate) fn run_one(app: &mut RustGisApp, title: &str, command: &str, mut p: Value, layer: &str, keep: Option<&'static str>) {
    let layer = fresh(app, layer);
    p["name"] = json!(layer);
    let layer = layer.as_str();
    let mut s = makes(command, p, layer);
    s.keep = keep;
    let done = crate::i18n::fmt("{name} is ready.", &[("name", layer)]);
    start_chain(app, tl!(title).to_string(), vec![s], done);
}

pub(crate) fn now(app: &mut RustGisApp, command: &str, p: Value) -> Option<Value> {
    match app.run(command, p) {
        Ok(v) => {
            app.map.invalidate();
            Some(v)
        }
        Err(e) => {
            say(app, false, e);
            None
        }
    }
}

pub(crate) fn area_choice(app: &mut RustGisApp, ui: &mut Ui) {
    heading(ui, "Study area");
    ui.horizontal_wrapped(|ui| {
        for (k, label) in ["Province / district / commune", "Map view", "Polygon layer"].into_iter().enumerate() {
            ui.radio_value(&mut app.ui.rslab.area_kind, k, tl!(label));
        }
    });
    match app.ui.rslab.area_kind {
        0 => {
            let provs = crate::khdata::provinces();
            let mut changed_district = false;
            {
                let st = &mut app.ui.rslab;
                let current = provs
                    .iter()
                    .find(|p| p.0 == st.province)
                    .map(|p| crate::khdata::label(&p.0, &p.1))
                    .unwrap_or_else(|| tl!("Choose…").to_string());
                egui::ComboBox::from_label(tl!("Province")).selected_text(current).width(200.0).truncate().show_ui(ui, |ui| {
                    for p in &provs {
                        if ui.selectable_label(st.province == p.0, crate::khdata::label(&p.0, &p.1)).clicked() {
                            st.province = p.0.clone();
                            st.district.clear();
                            st.commune.clear();
                            st.communes = None;
                        }
                    }
                });
                let ds = crate::khdata::districts(&st.province);
                let whole = tl!("Whole province").to_string();
                let current = ds.iter().find(|d| d.0 == st.district).map(|d| crate::khdata::label(&d.0, &d.1)).unwrap_or_else(|| whole.clone());
                ui.add_enabled_ui(!ds.is_empty(), |ui| {
                    egui::ComboBox::from_label(tl!("District")).selected_text(current).width(200.0).truncate().show_ui(ui, |ui| {
                        if ui.selectable_label(st.district.is_empty(), &whole).clicked() {
                            st.district.clear();
                            changed_district = true;
                        }
                        for d in &ds {
                            if ui.selectable_label(st.district == d.0, crate::khdata::label(&d.0, &d.1)).clicked() {
                                st.district = d.0.clone();
                                changed_district = true;
                            }
                        }
                    });
                });
            }
            if changed_district {
                want_communes(app);
            }
            let st = &mut app.ui.rslab;
            if !st.district.is_empty()
                && let Some(cs) = &st.communes
            {
                let whole = tl!("Whole district").to_string();
                let current = cs
                    .iter()
                    .find(|c| c.0 == st.commune)
                    .map(|c| crate::khdata::label(&c.0, &c.1))
                    .unwrap_or_else(|| whole.clone());
                let names: Vec<(String, String)> = cs.iter().map(|c| (c.0.clone(), crate::khdata::label(&c.0, &c.1))).collect();
                egui::ComboBox::from_label(tl!("Commune")).selected_text(current).width(200.0).truncate().show_ui(ui, |ui| {
                    if ui.selectable_label(st.commune.is_empty(), &whole).clicked() {
                        st.commune.clear();
                    }
                    for (n, shown) in &names {
                        if ui.selectable_label(st.commune == *n, shown).clicked() {
                            st.commune.clone_from(n);
                        }
                    }
                });
            }
        }
        2 => {
            let polys = vectors(app, true);
            if polys.is_empty() {
                ui.label(egui::RichText::new(tl!("Add or draw a polygon layer first.")).weak());
            } else {
                pick(ui, "rslab_area_layer", "Layer", &mut app.ui.rslab.area_layer, &polys);
            }
        }
        _ => {}
    }
    if app.ui.rslab.area_kind != 1 {
        ui.checkbox(&mut app.ui.rslab.clip, tl!("Clip to the area's shape"));
    }
}

pub(crate) fn data_step(app: &mut RustGisApp, ui: &mut Ui) {
    area_choice(app, ui);
    ui.add_space(4.0);
    let st = &mut app.ui.rslab;
    egui::ComboBox::from_label(tl!("Dataset")).selected_text(tl!(st.dataset.label())).width(230.0).truncate().show_ui(ui, |ui| {
        for d in Dataset::ALL {
            if ui.selectable_label(st.dataset == d, tl!(d.label())).clicked() && st.dataset != d {
                st.dataset = d;
                st.cell = d.cell();
                st.scenes.clear();
                st.picked = None;
            }
        }
    });
    let ds = st.dataset;
    egui::Grid::new("rslab_dates").num_columns(2).show(ui, |ui| {
        if ds.dated() {
            ui.label(tl!("From"));
            ui.add(egui::TextEdit::singleline(&mut st.start).desired_width(110.0).hint_text("YYYY-MM-DD"));
            ui.end_row();
            ui.label(tl!("To"));
            ui.add(egui::TextEdit::singleline(&mut st.end).desired_width(110.0).hint_text("YYYY-MM-DD"));
            ui.end_row();
        }
        if ds.optical() {
            ui.label(tl!("Max scene cloud cover"));
            ui.add(egui::Slider::new(&mut st.cloud, 0.0..=100.0).suffix(" %").integer());
            ui.end_row();
        }
        if ds.dated() {
            ui.label(tl!("Dates in the composite"));
            ui.add(egui::DragValue::new(&mut st.composite).range(1.0..=8.0).speed(0.05).fixed_decimals(0))
                .on_hover_text(tl!("1: the date picked. More: the median of the picked and the next clearest dates, which fills clouds"));
            ui.end_row();
        }
        ui.label(tl!("Pixel size"));
        ui.add(egui::DragValue::new(&mut st.cell).range(10.0..=1000.0).speed(1.0).suffix(" m"));
        ui.end_row();
    });
    if ds.optical() {
        ui.checkbox(&mut st.mask, tl!("Mask clouds and shadows (SCL / QA_PIXEL)"));
    }
    if ds == Dataset::Dem {
        ui.checkbox(&mut st.slope, tl!("Add slope"));
    }
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
        let optical = st.scenes_of.optical();
        egui::ScrollArea::vertical().id_salt("rslab_scenes").max_height(150.0).show(ui, |ui| {
            for (i, s) in st.scenes.iter().enumerate() {
                let text = if optical {
                    crate::i18n::fmt(
                        "{date} · cloud {c}% · {n} tile(s)",
                        &[("date", &s.date), ("c", &format!("{:.1}", s.cloud)), ("n", &s.items.len().to_string())],
                    )
                } else {
                    crate::i18n::fmt("{date} · {n} tile(s)", &[("date", &s.date), ("n", &s.items.len().to_string())])
                };
                if ui.selectable_label(st.picked == Some(i), text).clicked() {
                    st.picked = Some(i);
                }
            }
        });
        let busy = app.ui.rslab.chain.is_some();
        ui.add_enabled_ui(!busy && app.ui.rslab.picked.is_some(), |ui| {
            if ui
                .button(egui::RichText::new(tl!("Download")).strong())
                .on_hover_text(tl!("Clipped to the study area and merged into one image"))
                .clicked()
            {
                download(app);
            }
        });
    }
    if let Some(m) = result(app, "merge").and_then(|v| v["maskedPercent"].as_f64()) {
        ui.label(
            egui::RichText::new(crate::i18n::fmt("Clouds and shadows masked: {p}% of the pixels read.", &[("p", &format!("{m:.1}"))]))
                .weak(),
        );
    }
    crate::rstools::seasons_ui(app, ui);
}

/// An index: its name, what it shows, and its formula over bands 1–6 (Blue … SWIR2).
const INDICES: &[(&str, &str, &str)] = &[
    ("NDVI", "Vegetation", "(b4 - b3) / (b4 + b3)"),
    ("GNDVI", "Chlorophyll", "(b4 - b2) / (b4 + b2)"),
    ("SAVI", "Vegetation on bare soil", "1.5 * (b4 - b3) / (b4 + b3 + 0.5)"),
    ("EVI", "Dense vegetation", "2.5 * (b4 - b3) / (b4 + 6 * b3 - 7.5 * b1 + 1)"),
    ("NDWI", "Open water", "(b2 - b4) / (b2 + b4)"),
    ("MNDWI", "Water near towns", "(b2 - b5) / (b2 + b5)"),
    ("NDMI", "Moisture in plants", "(b4 - b5) / (b4 + b5)"),
    ("NDBI", "Built-up areas", "(b5 - b4) / (b5 + b4)"),
    ("BSI", "Bare soil", "((b5 + b3) - (b4 + b1)) / ((b5 + b3) + (b4 + b1))"),
    ("NBR", "Burned areas", "(b4 - b6) / (b4 + b6)"),
];

pub(crate) fn enhance_step(app: &mut RustGisApp, ui: &mut Ui) {
    let rs = rasters(app);
    pick(ui, "rslab_img", "Image", &mut app.ui.rslab.image, &rs);
    let image = app.ui.rslab.image.clone();
    let count = band_count(app, &image);
    let six = count >= 6;
    ui.add_enabled_ui(!image.is_empty(), |ui| {
        heading(ui, "Band combination");
        if six {
            ui.horizontal_wrapped(|ui| {
                for (label, rgb) in [
                    ("Natural color (3-2-1)", [3, 2, 1]),
                    ("Color infrared (4-3-2)", [4, 3, 2]),
                    ("Agriculture (5-4-1)", [5, 4, 1]),
                    ("Urban (6-5-3)", [6, 5, 3]),
                    ("Land/water (4-5-3)", [4, 5, 3]),
                    ("Vegetation (5-4-3)", [5, 4, 3]),
                ] {
                    if ui.button(tl!(label)).clicked() {
                        app.ui.rslab.rgb = rgb;
                        now(app, "raster.bands", json!({"layer": image, "red": rgb[0], "green": rgb[1], "blue": rgb[2]}));
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            let n = count.max(1) as u32;
            let st = &mut app.ui.rslab;
            for (i, c) in ["R", "G", "B"].iter().enumerate() {
                ui.label(*c);
                ui.add(egui::DragValue::new(&mut st.rgb[i]).range(1..=n));
            }
            if ui.button(tl!("Apply")).clicked() {
                let rgb = app.ui.rslab.rgb;
                now(app, "raster.bands", json!({"layer": image, "red": rgb[0], "green": rgb[1], "blue": rgb[2]}));
            }
        });
        heading(ui, "Contrast stretch");
        ui.horizontal_wrapped(|ui| {
            for (label, p) in [
                ("Min–max", json!({"type": "minMax"})),
                ("Percent clip (2%)", json!({"type": "percentClip", "low": 2, "high": 2})),
                ("Standard deviation (2)", json!({"type": "standardDeviation", "n": 2})),
                ("Histogram equalization", json!({"type": "histogramEqualize"})),
            ] {
                if ui.button(tl!(label)).clicked() {
                    let mut p = p;
                    p["layer"] = json!(image);
                    now(app, "raster.stretch", p);
                }
            }
        });
        heading(ui, "Spectral indices");
        if !six {
            ui.label(egui::RichText::new(tl!("Indices need a six-band image (Blue, Green, Red, NIR, SWIR1, SWIR2) from 1 Data.")).weak());
        }
        ui.add_enabled_ui(six, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (idx, tip, expr) in INDICES {
                    if ui.button(*idx).on_hover_text(format!("{} · {expr}", tl!(tip))).clicked() {
                        let out = format!("{image}_{idx}");
                        run_one(
                            app,
                            "Computing indices",
                            "raster.function",
                            json!({"layer": image, "function": "bandMath", "expression": expr, "name": out}),
                            &out,
                            None,
                        );
                    }
                }
            });
        });
        heading(ui, "Band math (your own formula)");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.ui.rslab.expression).desired_width(200.0).hint_text("(b4 - b3) / (b4 + b3)"));
            if ui.button(tl!("Run")).clicked() {
                let expr = app.ui.rslab.expression.clone();
                let out = format!("{image}_math");
                run_one(
                    app,
                    "Band math",
                    "raster.function",
                    json!({"layer": image, "function": "bandMath", "expression": expr, "name": out}),
                    &out,
                    None,
                );
            }
        });
        heading(ui, "Spatial filter");
        ui.horizontal_wrapped(|ui| {
            let st = &mut app.ui.rslab;
            egui::ComboBox::from_id_salt("rslab_filter").selected_text(tl!(filter_label(st.filter))).width(170.0).truncate().show_ui(
                ui,
                |ui| {
                    for f in ["mean", "median", "gaussian", "sharpen", "sobel", "laplacian", "emboss", "min", "max"] {
                        ui.selectable_value(&mut st.filter, f, tl!(filter_label(f)));
                    }
                },
            );
            ui.label(tl!("window"));
            ui.add(egui::DragValue::new(&mut st.filter_size).range(3.0..=15.0).speed(0.1).fixed_decimals(0));
            if ui.button(tl!("Apply filter")).clicked() {
                let (f, size) = (st.filter, (st.filter_size.round() as u64) | 1);
                let out = format!("{image}_{f}");
                run_one(app, "Filtering", "raster.filter", json!({"layer": image, "filter": f, "size": size, "name": out}), &out, None);
            }
        });
        heading(ui, "Principal Component Analysis");
        ui.horizontal(|ui| {
            ui.label(tl!("Components"));
            ui.add(egui::DragValue::new(&mut app.ui.rslab.components).range(1.0..=count.max(1) as f64).speed(0.05).fixed_decimals(0));
            if ui.add_enabled(count >= 2, egui::Button::new(tl!("Run PCA"))).clicked() {
                let k = app.ui.rslab.components.round() as u64;
                let out = format!("{image}_pca");
                run_one(
                    app,
                    "Principal Component Analysis",
                    "raster.pca",
                    json!({"layer": image, "components": k, "name": out}),
                    &out,
                    Some("pca"),
                );
            }
        });
        if let Some(v) = result(app, "pca").cloned() {
            pca_table(ui, &v);
        }
        heading(ui, "Feature stack for classification");
        ui.label(egui::RichText::new(tl!("Adds layers to the image's bands, ready to classify.")).weak());
        let others: Vec<String> = rs.iter().filter(|n| **n != image).cloned().collect();
        let st = &mut app.ui.rslab;
        ui.add_enabled(six, egui::Checkbox::new(&mut st.stack_indices, tl!("Indices (NDVI, NDWI, NDBI)")));
        ui.checkbox(&mut st.stack_texture, tl!("Texture of the NIR band (GLCM contrast, homogeneity, entropy)"));
        ui.horizontal(|ui| {
            ui.label(tl!("Another layer"));
            let none = tl!("None").to_string();
            egui::ComboBox::from_id_salt("rslab_stack_extra")
                .selected_text(if st.stack_extra.is_empty() { none.clone() } else { st.stack_extra.clone() })
                .width(180.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut st.stack_extra, String::new(), none.as_str());
                    for o in &others {
                        ui.selectable_value(&mut st.stack_extra, o.clone(), o.as_str());
                    }
                })
                .response
                .on_hover_text(tl!("Another date, Sentinel-1 radar or elevation (put on the image's grid)"));
        });
        if ui.button(egui::RichText::new(tl!("Build feature stack")).strong()).clicked() {
            feature_stack(app, &image, six, count);
        }
    });
    heading(ui, "Look at the image");
    crate::rstools::profile_ui(app, ui);
    ui.add_space(4.0);
    crate::rstools::plots_ui(app, ui);
}

pub(crate) fn filter_label(f: &str) -> &'static str {
    match f {
        "mean" => "Mean (low-pass)",
        "median" => "Median (removes speckle/noise)",
        "gaussian" => "Gaussian (low-pass)",
        "sharpen" => "High-pass sharpen",
        "sobel" => "Sobel edges",
        "laplacian" => "Laplacian edges",
        "emboss" => "Emboss",
        "min" => "Minimum",
        _ => "Maximum",
    }
}

pub(crate) fn pca_table(ui: &mut Ui, v: &Value) {
    let explained: Vec<f64> = v["explained"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    let mut cumulative = 0.0;
    egui::Grid::new("rslab_pca").striped(true).show(ui, |ui| {
        for h in ["Component", "Explained %", "Cumulative %"] {
            ui.label(egui::RichText::new(tl!(h)).strong());
        }
        ui.end_row();
        for (i, e) in explained.iter().enumerate() {
            cumulative += e;
            ui.label(format!("PC{}", i + 1));
            ui.label(format!("{e:.1}"));
            ui.label(format!("{cumulative:.1}"));
            ui.end_row();
        }
    });
}

pub(crate) fn layer_band_names(app: &RustGisApp, layer: &str) -> Vec<String> {
    app.session
        .map()
        .layer_by_name(layer)
        .and_then(|l| {
            l.raster.as_ref()?.data.as_ref().map(|d| {
                d.bands
                    .iter()
                    .enumerate()
                    .map(|(i, b)| if b.name.is_empty() { format!("{layer}_{}", i + 1) } else { b.name.clone() })
                    .collect()
            })
        })
        .unwrap_or_default()
}

pub(crate) fn feature_stack(app: &mut RustGisApp, image: &str, six: bool, count: usize) {
    let (indices, texture, extra) = (app.ui.rslab.stack_indices, app.ui.rslab.stack_texture, app.ui.rslab.stack_extra.clone());
    let mut steps = Vec::new();
    let mut layers = Vec::new();
    let mut names = layer_band_names(app, image);
    if indices && six {
        for idx in ["NDVI", "NDWI", "NDBI"] {
            let Some((_, _, expr)) = INDICES.iter().find(|i| i.0 == idx) else { continue };
            let out = fresh(app, &format!("{image}_{idx}_fs"));
            steps.push(makes("raster.function", json!({"layer": image, "function": "bandMath", "expression": expr, "name": out}), &out));
            layers.push(out);
            names.push(idx.to_string());
        }
    }
    if texture {
        let out = fresh(app, &format!("{image}_texture_fs"));
        let band = if count >= 4 { 4 } else { 1 };
        steps.push(makes("raster.texture", json!({"layer": image, "band": band, "window": 5, "name": out}), &out));
        layers.push(out);
        names.extend(["contrast", "homogeneity", "entropy"].map(String::from));
    }
    if !extra.is_empty() {
        names.extend(layer_band_names(app, &extra));
        layers.push(extra);
    }
    if layers.is_empty() {
        return say(app, false, tl!("nothing ticked yet").to_string());
    }
    let out = fresh(app, &format!("{image}_features"));
    steps.push(makes(
        "raster.process",
        json!({"layer": image, "operation": "stack", "layers": layers, "bandNames": names, "name": out}),
        &out,
    ));
    let temporary: Vec<String> = layers.iter().filter(|l| l.ends_with("_fs")).cloned().collect();
    if !temporary.is_empty() {
        steps.push(step("layer.remove", json!({"layer": temporary})));
    }
    let done = crate::i18n::fmt("{name} is ready.", &[("name", &out)]);
    app.ui.rslab.image = out;
    start_chain(app, tl!("Building feature stack").to_string(), steps, done);
}

/// A classifier: id (as `raster.classifySupervised` takes it), name, and its option (name,
/// default, least, most).
type Option4 = Option<(&'static str, f64, f64, f64)>;
const CLASSIFIERS: &[(&str, &str, Option4)] = &[
    ("randomForest", "Random Forest", Some(("Number of trees", 50.0, 5.0, 300.0))),
    ("knn", "k-Nearest Neighbours", Some(("k neighbours", 5.0, 1.0, 25.0))),
    ("maxLikelihood", "Maximum Likelihood", None),
    ("mahalanobis", "Mahalanobis Distance", None),
    ("minDistance", "Minimum Distance", None),
    ("sam", "Spectral Angle Mapper", None),
    ("parallelepiped", "Parallelepiped", Some(("Box size (± n std. dev.)", 2.0, 0.5, 5.0))),
];

pub(crate) fn classify_step(app: &mut RustGisApp, ui: &mut Ui) {
    let rs = rasters(app);
    pick(ui, "rslab_cimg", "Image", &mut app.ui.rslab.image, &rs);
    let image = app.ui.rslab.image.clone();
    let bands: Vec<usize> = (1..=band_count(app, &image)).collect();
    heading(ui, "Unsupervised (K-means, ISODATA)");
    ui.horizontal_wrapped(|ui| {
        let st = &mut app.ui.rslab;
        ui.radio_value(&mut st.unsupervised, "kmeans", "K-means");
        ui.radio_value(&mut st.unsupervised, "isodata", "ISODATA");
        if st.unsupervised == "isodata" {
            ui.label(tl!("min"));
            ui.add(egui::DragValue::new(&mut st.min_classes).range(2.0..=20.0).speed(0.05).fixed_decimals(0));
            ui.label(tl!("max"));
        } else {
            ui.label(tl!("Classes"));
        }
        ui.add(egui::DragValue::new(&mut st.classes).range(2.0..=30.0).speed(0.05).fixed_decimals(0));
        if ui.add_enabled(!image.is_empty(), egui::Button::new(tl!("Run clustering"))).clicked() {
            let (k, m, method) = (st.classes.round() as u64, st.min_classes.round() as u64, st.unsupervised);
            let out = fresh(app, &format!("{image}_{method}{k}"));
            app.ui.rslab.class_map = out.clone();
            let p = json!({"layer": image, "method": method, "classes": k, "minClasses": m.min(k), "bands": bands, "name": out});
            run_one(app, "Clustering", "raster.classify", p, &out, None);
        }
    });
    heading(ui, "Supervised — classes & training");
    {
        let st = &mut app.ui.rslab;
        ui.horizontal(|ui| {
            ui.radio_value(&mut st.training_from_map, false, tl!("Training polygons"));
            ui.radio_value(&mut st.training_from_map, true, tl!("From a land cover map"));
        });
    }
    if app.ui.rslab.training_from_map {
        let maps: Vec<String> = rs.iter().filter(|n| **n != image).cloned().collect();
        pick(ui, "rslab_tmap", "Land cover", &mut app.ui.rslab.training_map, &maps);
        ui.label(
            egui::RichText::new(tl!("Download ESA WorldCover or Esri land cover in 1 Data: its classes teach the classifier.")).weak(),
        );
    } else {
        if ui
            .button(tl!("Create training layer (polygons)"))
            .on_hover_text(tl!("Then draw polygons with Add Polygon Feature and type each one's class in its name field"))
            .clicked()
        {
            if app.session.map().layer_by_name("training").is_none() {
                now(app, "layer.template", json!({"template": "polygons", "name": "training"}));
            }
            app.ui.rslab.training = "training".into();
            app.ui.rslab.class_field = "name".into();
            now(app, "edit.enable", json!({"on": true}));
        }
        let polys = vectors(app, true);
        if !polys.is_empty() {
            pick(ui, "rslab_train", "Training layer", &mut app.ui.rslab.training, &polys);
            let fs = fields(app, &app.ui.rslab.training.clone());
            pick(ui, "rslab_field", "Class field", &mut app.ui.rslab.class_field, &fs);
        }
    }
    heading(ui, "Supervised — classify");
    {
        let st = &mut app.ui.rslab;
        let current = CLASSIFIERS.iter().find(|c| c.0 == st.method).map_or("", |c| c.1);
        egui::ComboBox::from_label(tl!("Classifier")).selected_text(tl!(current)).width(200.0).truncate().show_ui(ui, |ui| {
            for (id, label, opt) in CLASSIFIERS {
                if ui.selectable_label(st.method == *id, tl!(label)).clicked() {
                    st.method = id;
                    if let Some((_, d, ..)) = opt {
                        st.option = *d;
                    }
                }
            }
        });
        if let Some((_, _, Some((label, _, lo, hi)))) = CLASSIFIERS.iter().find(|c| c.0 == st.method) {
            ui.horizontal(|ui| {
                ui.label(tl!(label));
                let decimals = if st.method == "parallelepiped" { 1 } else { 0 };
                ui.add(egui::DragValue::new(&mut st.option).range(*lo..=*hi).speed(0.1).fixed_decimals(decimals));
            });
        }
        ui.horizontal(|ui| {
            ui.label(tl!("Training pixels per class (most)"));
            ui.add(egui::DragValue::new(&mut st.per_class).range(50.0..=20_000.0).speed(10.0).fixed_decimals(0));
        });
    }
    if !app.ui.rslab.training_from_map {
        crate::rstools::wand_ui(app, ui);
    }
    let st = &app.ui.rslab;
    let ready = !image.is_empty()
        && if st.training_from_map { !st.training_map.is_empty() } else { !st.training.is_empty() && !st.class_field.is_empty() };
    if ui.add_enabled(ready, egui::Button::new(egui::RichText::new(tl!("Classify")).strong())).clicked() {
        let st = &app.ui.rslab;
        let out = fresh(app, &format!("{image}_{}", st.method));
        let mut p = json!({"layer": image, "method": st.method, "option": st.option, "perClass": st.per_class.round() as u64, "bands": bands, "name": out});
        if st.training_from_map {
            p["trainingRaster"] = json!(st.training_map);
        } else {
            p["training"] = json!(st.training);
            p["classField"] = json!(st.class_field);
        }
        app.ui.rslab.class_map = out.clone();
        run_one(app, "Classifying", "raster.classifySupervised", p, &out, Some("training"));
    }
    if let Some(v) = result(app, "training").cloned() {
        training_result(ui, &v);
    }
    accuracy_section(app, ui, &rs);
    let class_map = app.ui.rslab.class_map.clone();
    if !class_map.is_empty()
        && app.session.map().layer_by_name(&class_map).is_some()
        && ui.button(tl!("Class map to polygons")).on_hover_text(tl!("Each patch of one class becomes a polygon with its class and hectares")).clicked()
    {
        let out = fresh(app, &format!("{class_map}_polygons"));
        if let Some(v) = now(app, "raster.toPolygons", json!({"layer": class_map, "minPixels": 4, "name": out})) {
            say(app, true, crate::i18n::fmt("{n} polygons made.", &[("n", &v["polygons"].to_string())]));
        }
    }
}

pub(crate) fn training_result(ui: &mut Ui, v: &Value) {
    if let Some(a) = v["trainingAccuracy"].as_f64() {
        ui.label(crate::i18n::fmt("Training accuracy (optimistic): {p}%", &[("p", &format!("{:.1}", a * 100.0))]));
    }
    let mut imp: Vec<(String, f64)> = v["importance"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| Some((x["band"].as_str()?.to_string(), x["importance"].as_f64()?))).collect())
        .unwrap_or_default();
    if imp.is_empty() {
        return;
    }
    imp.sort_by(|a, b| b.1.total_cmp(&a.1));
    let most = imp.first().map_or(1.0, |m| m.1).max(1e-9);
    ui.label(egui::RichText::new(tl!("Most important bands")).strong());
    for (n, v) in imp.iter().take(8) {
        ui.horizontal(|ui| {
            ui.label(n.as_str());
            let (r, _) = ui.allocate_exact_size(egui::vec2(120.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(
                egui::Rect::from_min_size(r.min, egui::vec2(120.0 * (*v / most) as f32, 10.0)),
                2.0,
                egui::Color32::from_rgb(0x08, 0x91, 0xB2),
            );
            ui.label(format!("{:.0}%", v * 100.0));
        });
    }
}

pub(crate) fn accuracy_section(app: &mut RustGisApp, ui: &mut Ui, rs: &[String]) {
    heading(ui, "Classified map");
    pick(ui, "rslab_cmap", "Map", &mut app.ui.rslab.class_map, rs);
    let map = app.ui.rslab.class_map.clone();
    if map.is_empty() {
        return;
    }
    if ui.button(tl!("Class area (ha)")).on_hover_text(tl!("Each class's area; name the classes (clusters) here")).clicked()
        && let Some(v) = now(app, "raster.classAreas", json!({"layer": map}))
    {
        app.ui.rslab.class_labels = v["classes"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| Some((c["value"].as_f64()? as f32, c["class"].as_str()?.to_string()))).collect())
            .unwrap_or_default();
        keep_result(app, "areas", v);
    }
    if let Some(v) = result(app, "areas").cloned() {
        egui::Grid::new("rslab_areas").striped(true).show(ui, |ui| {
            for h in ["Value", "Class", "Area (ha)"] {
                ui.label(egui::RichText::new(tl!(h)).strong());
            }
            ui.end_row();
            let rows = v["classes"].as_array().cloned().unwrap_or_default();
            for (i, c) in rows.iter().enumerate() {
                ui.label(c["value"].to_string());
                if let Some(l) = app.ui.rslab.class_labels.get_mut(i) {
                    ui.add(egui::TextEdit::singleline(&mut l.1).desired_width(130.0));
                } else {
                    ui.label(c["class"].as_str().unwrap_or(""));
                }
                ui.label(format!("{:.1}", c["hectares"].as_f64().unwrap_or(0.0)));
                ui.end_row();
            }
        });
        if ui.button(tl!("Save class names")).clicked() {
            let names: serde_json::Map<String, Value> = app.ui.rslab.class_labels.iter().map(|(v, n)| (format!("{v}"), json!(n))).collect();
            if now(app, "raster.classNames", json!({"layer": map, "names": names})).is_some() {
                say(app, true, tl!("Class names saved.").to_string());
            }
        }
    }
    heading(ui, "Accuracy / clean-up");
    ui.horizontal_wrapped(|ui| {
        let st = &mut app.ui.rslab;
        ui.label(tl!("Majority filter (0 = off)"));
        ui.add(egui::DragValue::new(&mut st.majority).range(0.0..=7.0).speed(0.05).fixed_decimals(0));
        ui.label(tl!("Minimum mapping unit"));
        ui.add(egui::DragValue::new(&mut st.sieve).range(1.0..=1000.0).speed(0.5).fixed_decimals(0).suffix(" px"));
        if ui.button(tl!("Clean up")).clicked() {
            let maj = match st.majority.round() as u64 {
                0 => 0,
                1..=4 => 3,
                5..=6 => 5,
                _ => 7,
            };
            let out = format!("{map}_clean");
            let p = json!({"layer": map, "minPixels": st.sieve.round() as u64, "majority": maj, "name": out});
            run_one(app, "Cleaning up", "raster.sieve", p, &out, Some("sieve"));
        }
    });
    if let Some(n) = result(app, "sieve").and_then(|v| v["changed"].as_u64()) {
        ui.label(egui::RichText::new(crate::i18n::fmt("Sieve: {n} pixels changed.", &[("n", &n.to_string())])).weak());
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Points per class"));
        ui.add(egui::DragValue::new(&mut app.ui.rslab.points).range(5.0..=500.0).speed(0.5).fixed_decimals(0));
        if ui
            .button(tl!("Generate validation points"))
            .on_hover_text(tl!("Stratified random points: fill ref_class with the true class (attribute table), then compute accuracy"))
            .clicked()
        {
            let per = app.ui.rslab.points.round() as u64;
            let name = format!("{map}_validation");
            if now(app, "raster.samplePoints", json!({"layer": map, "perClass": per, "name": name})).is_some() {
                app.ui.rslab.reference = name;
                app.ui.rslab.ref_field = "ref_class".into();
                let _ = app.run("table.open", json!({}));
            }
        }
    });
    let refs = vectors(app, false);
    if !refs.is_empty() {
        pick(ui, "rslab_ref", "Reference layer", &mut app.ui.rslab.reference, &refs);
        let fs = fields(app, &app.ui.rslab.reference.clone());
        pick(ui, "rslab_reff", "Reference class field", &mut app.ui.rslab.ref_field, &fs);
        if ui.button(egui::RichText::new(tl!("Compute accuracy")).strong()).clicked() {
            let (r, f) = (app.ui.rslab.reference.clone(), app.ui.rslab.ref_field.clone());
            if let Some(v) = now(app, "raster.accuracy", json!({"layer": map, "reference": r, "classField": f})) {
                keep_result(app, "accuracy", v);
            }
        }
    }
    if let Some(v) = result(app, "accuracy").cloned() {
        accuracy_tables(ui, &v);
    }
}

pub(crate) fn accuracy_tables(ui: &mut Ui, v: &Value) {
    let classes: Vec<String> =
        v["classes"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let f = |k: &str| v[k].as_f64().unwrap_or(0.0);
    ui.label(crate::i18n::fmt(
        "Overall accuracy {oa}% · kappa {k} · area-weighted {w}% · {n} samples",
        &[
            ("oa", &format!("{:.1}", f("overall") * 100.0)),
            ("k", &format!("{:.3}", f("kappa"))),
            ("w", &format!("{:.1}", f("weightedOverall") * 100.0)),
            ("n", &v["samples"].to_string()),
        ],
    ));
    let arr = |k: &str| -> Vec<f64> { v[k].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default() };
    let (users, producers, mapped, est, ci) =
        (arr("users"), arr("producers"), arr("mappedHectares"), arr("estimatedHectares"), arr("ci95Hectares"));
    egui::ScrollArea::horizontal().id_salt("rslab_acc").show(ui, |ui| {
        egui::Grid::new("rslab_matrix").striped(true).show(ui, |ui| {
            ui.label(egui::RichText::new(tl!("Map / Ref")).strong());
            for c in &classes {
                ui.label(egui::RichText::new(c).strong());
            }
            ui.label(egui::RichText::new(tl!("User's acc.")).strong());
            ui.end_row();
            for (i, c) in classes.iter().enumerate() {
                ui.label(egui::RichText::new(c).strong());
                for j in 0..classes.len() {
                    let n = v["matrix"][i][j].as_u64().unwrap_or(0).to_string();
                    ui.label(if i == j { egui::RichText::new(n).strong() } else { egui::RichText::new(n) });
                }
                ui.label(format!("{:.0}%", users.get(i).copied().unwrap_or(0.0) * 100.0));
                ui.end_row();
            }
            ui.label(egui::RichText::new(tl!("Producer's acc.")).strong());
            for j in 0..classes.len() {
                ui.label(format!("{:.0}%", producers.get(j).copied().unwrap_or(0.0) * 100.0));
            }
            ui.end_row();
        });
    });
    ui.label(egui::RichText::new(tl!("Area-adjusted estimates")).strong());
    egui::Grid::new("rslab_est").striped(true).show(ui, |ui| {
        for h in ["Class", "Mapped (ha)", "Estimated (ha)", "± 95% CI (ha)"] {
            ui.label(egui::RichText::new(tl!(h)).strong());
        }
        ui.end_row();
        for (i, c) in classes.iter().enumerate() {
            ui.label(c);
            for x in [&mapped, &est, &ci] {
                ui.label(format!("{:.0}", x.get(i).copied().unwrap_or(0.0)));
            }
            ui.end_row();
        }
    });
}

/// A colour for the `i`th class of a diagram.
pub(crate) fn class_color(i: usize) -> egui::Color32 {
    const C: [(u8, u8, u8); 10] = [
        (0x2E, 0x86, 0xC1),
        (0x27, 0xAE, 0x60),
        (0xF3, 0x9C, 0x12),
        (0xC0, 0x39, 0x2B),
        (0x8E, 0x44, 0xAD),
        (0x16, 0xA0, 0x85),
        (0xD3, 0x54, 0x00),
        (0x7F, 0x8C, 0x8D),
        (0xF1, 0xC4, 0x0F),
        (0x2C, 0x3E, 0x50),
    ];
    let (r, g, b) = C[i % C.len()];
    egui::Color32::from_rgb(r, g, b)
}

/// The Sankey diagram of a from → to matrix: date 1's classes on the left, date 2's on the right,
/// bands as wide as the area that went from one to the other.
pub(crate) fn sankey(ui: &mut Ui, rows: &[(String, String, f64)]) {
    let mut left: Vec<(String, f64)> = Vec::new();
    let mut right: Vec<(String, f64)> = Vec::new();
    for (a, b, ha) in rows {
        match left.iter_mut().find(|x| x.0 == *a) {
            Some(x) => x.1 += ha,
            None => left.push((a.clone(), *ha)),
        }
        match right.iter_mut().find(|x| x.0 == *b) {
            Some(x) => x.1 += ha,
            None => right.push((b.clone(), *ha)),
        }
    }
    let total: f64 = left.iter().map(|x| x.1).sum();
    if total <= 0.0 {
        return;
    }
    let width = ui.available_width().clamp(260.0, 420.0);
    let height = 260.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let gap = 4.0;
    let usable = |n: usize| height - gap * n.saturating_sub(1) as f32;
    let (scale_l, scale_r) = (usable(left.len()) / total as f32, usable(right.len()) / total as f32);
    let (lx, rx, bar) = (rect.left() + 100.0, rect.right() - 100.0, 10.0);
    // Labels shortened to fit their side.
    let short = |s: &str| -> String {
        if s.chars().count() > 15 { format!("{}…", s.chars().take(14).collect::<String>()) } else { s.to_string() }
    };
    let mut names: Vec<&str> = left.iter().map(|x| x.0.as_str()).collect();
    for r in &right {
        if !names.contains(&r.0.as_str()) {
            names.push(&r.0);
        }
    }
    let colour_of = |name: &str| class_color(names.iter().position(|x| *x == name).unwrap_or(0));
    let font = egui::FontId::proportional(11.0);
    let text = ui.visuals().text_color();
    let mut ly: Vec<f32> = Vec::new();
    let mut y = rect.top();
    for (name, ha) in &left {
        let h = (*ha as f32 * scale_l).max(1.0);
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(lx - bar, y), egui::vec2(bar, h)), 1.0, colour_of(name));
        p.text(egui::pos2(lx - bar - 4.0, y + h / 2.0), egui::Align2::RIGHT_CENTER, short(name), font.clone(), text);
        ly.push(y);
        y += h + gap;
    }
    let mut ry: Vec<f32> = Vec::new();
    let mut y = rect.top();
    for (name, ha) in &right {
        let h = (*ha as f32 * scale_r).max(1.0);
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(rx, y), egui::vec2(bar, h)), 1.0, colour_of(name));
        p.text(egui::pos2(rx + bar + 4.0, y + h / 2.0), egui::Align2::LEFT_CENTER, short(name), font.clone(), text);
        ry.push(y);
        y += h + gap;
    }
    for (a, b, ha) in rows {
        let (Some(i), Some(j)) = (left.iter().position(|x| x.0 == *a), right.iter().position(|x| x.0 == *b)) else { continue };
        let (hl, hr) = (*ha as f32 * scale_l, *ha as f32 * scale_r);
        let (y0, y1) = (ly[i], ry[j]);
        ly[i] += hl;
        ry[j] += hr;
        let c = colour_of(a).gamma_multiply(if a == b { 0.25 } else { 0.55 });
        // The band in vertical strips (each a convex quad) along a smooth S.
        const N: usize = 24;
        let at = |t: f32, from: f32, to: f32| {
            let s = t * t * (3.0 - 2.0 * t);
            egui::pos2(lx + (rx - lx) * t, from + (to - from) * s)
        };
        for k in 0..N {
            let (t0, t1) = (k as f32 / N as f32, (k + 1) as f32 / N as f32);
            let quad = vec![at(t0, y0, y1), at(t1, y0, y1), at(t1, y0 + hl, y1 + hr), at(t0, y0 + hl, y1 + hr)];
            p.add(egui::Shape::convex_polygon(quad, c, egui::Stroke::NONE));
        }
    }
}

pub(crate) fn change_step(app: &mut RustGisApp, ui: &mut Ui) {
    let rs = rasters(app);
    heading(ui, "From → to: compare two classified maps");
    // Two different maps to start with: the one below the top, and the top one.
    if app.ui.rslab.before.is_empty() && app.ui.rslab.after.is_empty() && rs.len() >= 2 {
        app.ui.rslab.before.clone_from(&rs[1]);
        app.ui.rslab.after.clone_from(&rs[0]);
    }
    pick(ui, "rslab_before", "Date 1 (before)", &mut app.ui.rslab.before, &rs);
    pick(ui, "rslab_after", "Date 2 (after)", &mut app.ui.rslab.after, &rs);
    let (before, after) = (app.ui.rslab.before.clone(), app.ui.rslab.after.clone());
    let ok = !before.is_empty() && !after.is_empty() && before != after;
    if ui.add_enabled(ok, egui::Button::new(egui::RichText::new(tl!("Detect change")).strong())).clicked() {
        let out = format!("{before}_to_{after}");
        run_one(app, "Land-cover change", "raster.crossTab", json!({"from": before, "to": after, "name": out}), &out, Some("crossTab"));
    }
    if let Some(v) = result(app, "crossTab").cloned() {
        change_result(ui, &v);
    }
    heading(ui, "Image differencing (index or band)");
    let st = &mut app.ui.rslab;
    ui.horizontal(|ui| {
        ui.label(tl!("Method"));
        for (m, label) in [("difference", "Difference"), ("ratio", "Ratio"), ("percent", "Percent")] {
            ui.radio_value(&mut st.diff_method, m, tl!(label));
        }
    });
    let method = st.diff_method;
    if ui.add_enabled(ok, egui::Button::new(tl!("Run"))).clicked() {
        let out = format!("change_{method}");
        run_one(
            app,
            "Change detection",
            "raster.change",
            json!({"before": before, "after": after, "method": method, "name": out}),
            &out,
            None,
        );
    }
}

pub(crate) fn change_result(ui: &mut Ui, v: &Value) {
    let rows: Vec<(String, String, f64)> = v["matrix"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| Some((r["fromClass"].as_str()?.to_string(), r["toClass"].as_str()?.to_string(), r["hectares"].as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    ui.label(crate::i18n::fmt("Changed area: {ha} ha", &[("ha", &format!("{:.1}", v["changedHectares"].as_f64().unwrap_or(0.0)))]));
    ui.label(egui::RichText::new(tl!("Sankey diagram")).strong());
    sankey(ui, &rows);
    let mut classes: Vec<String> = Vec::new();
    for (a, b, _) in &rows {
        for c in [a, b] {
            if !classes.contains(c) {
                classes.push(c.clone());
            }
        }
    }
    ui.label(egui::RichText::new(tl!("Gains and losses per class")).strong());
    egui::ScrollArea::horizontal().id_salt("rslab_gain_scroll").show(ui, |ui| {
        egui::Grid::new("rslab_gain").striped(true).show(ui, |ui| {
            for h in ["Class", "Date 1 (ha)", "Date 2 (ha)", "Gain", "Loss", "Net"] {
                ui.label(egui::RichText::new(tl!(h)).strong());
            }
            ui.end_row();
            for c in &classes {
                let sum = |f: &dyn Fn(&(String, String, f64)) -> bool| rows.iter().filter(|r| f(r)).map(|r| r.2).sum::<f64>();
                let d1 = sum(&|r| r.0 == *c);
                let d2 = sum(&|r| r.1 == *c);
                let gain = sum(&|r| r.1 == *c && r.0 != *c);
                let loss = sum(&|r| r.0 == *c && r.1 != *c);
                ui.label(c);
                for x in [d1, d2, gain, loss] {
                    ui.label(format!("{x:.1}"));
                }
                ui.label(format!("{:+.1}", gain - loss));
                ui.end_row();
            }
        })
    });
    ui.label(egui::RichText::new(tl!("Largest changes (from → to)")).strong());
    let mut changes: Vec<&(String, String, f64)> = rows.iter().filter(|r| r.0 != r.1).collect();
    changes.sort_by(|a, b| b.2.total_cmp(&a.2));
    for (a, b, ha) in changes.iter().take(8) {
        ui.label(format!("{a} → {b}: {ha:.1} ha"));
    }
}

pub(crate) fn map_step(app: &mut RustGisApp, ui: &mut Ui) {
    heading(ui, "Map layout (print)");
    let km = crate::khdata::khmer();
    if app.ui.rslab.title.is_empty() {
        let place = area(app).map(|a| a.place).unwrap_or_default();
        app.ui.rslab.title = auto_title(&place, "", false, km);
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Title"));
        ui.add(egui::TextEdit::singleline(&mut app.ui.rslab.title).desired_width(250.0));
    });
    ui.horizontal(|ui| {
        let place = area(app).map(|a| a.place).unwrap_or_default();
        let year: String = app.ui.rslab.image.split('_').nth(1).map(|d| d.chars().take(4).collect()).unwrap_or_default();
        if ui.button(tl!("Image title")).clicked() {
            app.ui.rslab.title = auto_title(&place, &year, false, km);
        }
        if ui.button(tl!("Land cover title")).clicked() {
            app.ui.rslab.title = auto_title(&place, &year, true, km);
        }
    });
    if ui
        .button(egui::RichText::new(tl!("Create map layout")).strong())
        .on_hover_text(tl!("An A4 map of the view: title, legend, scale bar, north arrow, inset and sources"))
        .clicked()
    {
        let title = app.ui.rslab.title.clone();
        match app.run(
            "khgis.autoLayout",
            json!({"lang": crate::khdata::lang(), "title": title, "author": "Geoinformatics and Research Lab, RUPP"}),
        ) {
            Ok(_) => say(app, true, tl!("Map layout made: Publish › Export to save it as PDF.").to_string()),
            Err(e) => say(app, false, e),
        }
    }
    if ui
        .button(egui::RichText::new(tl!("PDF report")).strong())
        .on_hover_text(tl!("An A4 report: the map, legend, class areas (table and chart), accuracy and change, saved as PDF"))
        .clicked()
    {
        crate::rstools::report(app);
    }
    heading(ui, "3D");
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
        for (i, label) in ["1 Data", "2 Enhance", "3 Classify", "4 Change", "5 LEE", "6 Map"].into_iter().enumerate() {
            if ui.add(egui::Button::selectable(app.ui.rslab.step == i, tl!(label))).clicked() {
                app.ui.rslab.step = i;
            }
        }
    });
    // The run under way: its step, the background work's progress, and Cancel.
    let mut cancel = false;
    let job = crate::analysispane::background_job(app);
    if let Some(c) = &app.ui.rslab.chain {
        ui.horizontal_wrapped(|ui| {
            ui.spinner();
            let mut text = format!("{} ({}/{})", c.title, (c.done + 1).min(c.total), c.total);
            if let Some((name, Some(f))) = job {
                text.push_str(&format!(" · {} {:.0}%", tl!(name), f * 100.0));
            }
            ui.label(text);
            cancel = ui.small_button(tl!("Cancel")).clicked();
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    } else if let Some((name, done)) = job {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(match done {
                Some(f) => format!("{} {:.0}%", tl!(name), f * 100.0),
                None => tl!(name).to_string(),
            });
            cancel = ui.small_button(tl!("Cancel")).clicked();
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    if cancel {
        crate::analysispane::cancel_job(app);
        app.ui.rslab.chain = None;
        say(app, false, tl!("cancelled").to_string());
    }
    ui.separator();
    egui::ScrollArea::vertical().id_salt("rslab_step").auto_shrink([false, true]).show(ui, |ui| {
        match app.ui.rslab.step {
            0 => data_step(app, ui),
            1 => enhance_step(app, ui),
            2 => classify_step(app, ui),
            3 => change_step(app, ui),
            4 => crate::leelab::pane(app, ui),
            _ => map_step(app, ui),
        }
        ui.add_space(6.0);
        for (ok, line) in &app.ui.rslab.log {
            ui.label(egui::RichText::new(format!("{} {line}", if *ok { "✓" } else { "✗" })).color(if *ok { t.text_dim } else { t.red }));
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(tl!("Data: Copernicus Sentinel-1/2 and DEM (ESA), Landsat (USGS/NASA), ESA WorldCover, Esri | Impact Observatory land cover, JRC Global Surface Water, via Microsoft Planetary Computer; administrative boundaries 2024. After the LEE RS Lab plugin, Geoinformatics and Research Lab, RUPP."))
                .small()
                .color(t.text_dim),
        );
    });
}

/// Whether the lab is making something (a run of steps, or a workflow in the background).
pub(crate) fn busy(app: &RustGisApp) -> bool {
    app.ui.rslab.chain.is_some() || crate::analysispane::background_job(app).is_some()
}

/// The study area's box.
pub(crate) fn area_bbox(app: &RustGisApp) -> Result<[f64; 4], String> {
    area(app).map(|a| a.bbox)
}

/// The steps that make an image of `ds` from `chosen` over the study area (clouds masked, the
/// median of several dates, clipped to the area's shape), named for a script: its name and the
/// steps.
pub(crate) fn composite_steps(app: &RustGisApp, ds: Dataset, chosen: &[Scene], cell: f64, median: bool) -> Result<(String, Vec<Step>), String> {
    let a = area(app)?;
    let st = &app.ui.rslab;
    let (w, h, _) = grid(a.bbox, cell.max(ds.cell()));
    let items: Vec<(String, f64, f64)> = chosen.iter().flat_map(|s| s.items.iter().map(move |i| (i.clone(), s.scale, s.offset))).collect();
    let image = fresh(app, &format!("lee_{}", ds.short().replace('-', "")));
    let clip_layer = if a.shape.is_some() { String::new() } else { st.area_layer.clone() };
    let clip = st.clip && (a.shape.is_some() || st.area_kind == 2);
    let mut steps = image_steps(app, ds, &items, &a, &image, (w, h), ds.optical(), median && chosen.len() > 1, clip.then_some(clip_layer.as_str()));
    let classes = ds.classes();
    if !classes.is_empty() {
        let names: serde_json::Map<String, Value> = classes.iter().map(|(v, n)| (v.to_string(), json!(n))).collect();
        steps.push(step("raster.classNames", json!({"layer": image, "names": names})));
        steps.push(step("raster.symbology", json!({"layer": image, "type": "unique"})));
    }
    Ok((image, steps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_asks_for_clear_scenes_of_the_area() {
        let u = search_url(Dataset::Landsat89, [104.8, 11.5, 105.0, 11.7], "2025-01-01", "2025-03-31", 20.0);
        assert!(u.starts_with(STAC), "{u}");
        assert!(u.contains("collections=landsat-c2-l2&bbox=104.80000,11.50000,105.00000,11.70000"), "{u}");
        assert!(u.contains("datetime=2025-01-01T00:00:00Z/2025-03-31T23:59:59Z"), "{u}");
        assert!(u.contains("filter=eo:cloud_cover%3C20%20AND%20platform%20IN%20(%27landsat-8%27,%27landsat-9%27)"), "{u}");
        assert!(u.ends_with("sortby=%2Beo:cloud_cover"), "{u}");
        let dem = search_url(Dataset::Dem, [104.8, 11.5, 105.0, 11.7], "", "", 20.0);
        assert!(!dem.contains("datetime") && !dem.contains("filter"), "{dem}");
    }

    #[test]
    fn items_are_grouped_by_date_clearest_first() {
        let fc = json!({"features": [
            {"id": "a1", "properties": {"datetime": "2025-02-28T03:17:21Z", "eo:cloud_cover": 2.0, "s2:processing_baseline": "05.11"}},
            {"id": "a2", "properties": {"datetime": "2025-02-28T03:17:21Z", "eo:cloud_cover": 4.0, "s2:processing_baseline": "05.11"}},
            {"id": "b1", "properties": {"datetime": "2021-02-08T03:19:31Z", "eo:cloud_cover": 1.0, "s2:processing_baseline": "02.12"}},
        ]});
        let s = scenes(Dataset::Sentinel2, fc.to_string().as_bytes()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].date.as_str(), s[0].offset), ("2021-02-08", 0.0));
        assert_eq!((s[1].items.len(), s[1].cloud, s[1].offset), (2, 3.0, -0.1));
        assert!(scenes(Dataset::Sentinel2, br#"{"description": "bad"}"#).is_err());
        let wc = json!({"features": [{"id": "ESA_WorldCover_10m_2021_v200_N09E102", "properties": {}}, {"id": "ESA_WorldCover_10m_2020_v100_N09E102", "properties": {}}]});
        let s = scenes(Dataset::WorldCover, wc.to_string().as_bytes()).unwrap();
        assert_eq!(s.iter().map(|s| s.date.as_str()).collect::<Vec<_>>(), ["2021", "2020"]);
    }

    #[test]
    fn images_fit_the_area_and_the_limit() {
        let (w, h, cell) = grid([104.85, 11.52, 104.95, 11.62], 10.0);
        assert!((1080..1100).contains(&w) && (1100..1120).contains(&h) && cell == 10.0, "{w} {h} {cell}");
        let (w, h, cell) = grid([102.0, 11.0, 104.0, 13.0], 10.0);
        assert!(w <= MAX_SIDE && h == MAX_SIDE && cell > 80.0, "{w} {h} {cell}");
        let u = image_url(Dataset::Sentinel2, "S2C_X", [104.85, 11.52, 104.95, 11.62], w, h, true);
        assert!(u.contains("/104.850000,11.520000,104.950000,11.620000/"), "{u}");
        assert!(u.contains("&assets=B02&assets=B03&assets=B04&assets=B08&assets=B11&assets=B12&assets=SCL"), "{u}");
        assert!(image_url(Dataset::Landsat89, "L", [0.0, 0.0, 1.0, 1.0], 9, 9, true).contains("&assets=qa_pixel"));
    }

    #[test]
    fn cambodia_3d_takes_at_most_four_km() {
        let u = cambodia3d_url([104.0, 11.0, 106.0, 13.0]);
        let b: Vec<f64> = u.split("aoi=").nth(1).unwrap().split(',').map(|v| v.parse().unwrap()).collect();
        assert!((b[3] - b[1]) * 111.32 <= 4.01 && (b[2] - b[0]) * 111.32 * 12f64.to_radians().cos() <= 4.01, "{u}");
        assert!(((b[0] + b[2]) / 2.0 - 105.0).abs() < 1e-4);
    }

    #[test]
    fn communes_are_bundled_with_khmer_names() {
        let c = crate::khdata::communes("Kandal", "Kien Svay");
        assert!(c.len() >= 5, "{}", c.len());
        assert!(c.iter().all(|c| !c.1.is_empty() && !c.2.is_empty()));
    }

    #[test]
    fn titles_follow_the_place_and_year() {
        assert_eq!(auto_title("Kandal", "2025", true, false), "Land use and land cover map of Kandal, 2025");
        assert_eq!(auto_title("ខេត្តកណ្ដាល", "2025", true, true), "ផែនទីការប្រើប្រាស់ដី និងគម្របដី ខេត្តកណ្ដាល ឆ្នាំ២០២៥");
        assert_eq!(auto_title("", "view", false, false), "Satellite image map");
    }
}
