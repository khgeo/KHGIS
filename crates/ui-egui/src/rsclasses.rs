//! LEE RS Lab's classes, as the plugin has them: one class list (id, name, colour) for the whole
//! Classify tab, from a scheme or typed; labelling clusters after K-means or ISODATA (each
//! cluster's area, a hint from its spectrum, a class to merge it into, an example pixel to look
//! at, a mixed one split, their spectra plotted) and the labelled map made from them; the class
//! colours put on a class map; validation points labelled one by one with a button per class; a
//! bar chart of the class areas; and the error matrix saved as CSV.

use egui::{Color32, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::RustGisApp;
use crate::rslab::{fresh, keep_result, now, result, say};

/// A class: its id (the value in the map), name and colour.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassDef {
    pub id: u32,
    pub name: String,
    pub color: [u8; 3],
}

/// The schemes to start from: (name, classes as (id, name, #rrggbb)).
pub const SCHEMES: [(&str, &[(u32, &str, &str)]); 4] = [
    (
        "RS Lab default (6 classes)",
        &[
            (1, "Water", "#2b83ba"),
            (2, "Built-up", "#d7191c"),
            (3, "Rice / cropland", "#f2e35b"),
            (4, "Forest", "#1a7f3c"),
            (5, "Shrub / grassland", "#9bd06b"),
            (6, "Bare soil", "#c2a079"),
        ],
    ),
    (
        "LULC level 1 (8 classes)",
        &[
            (1, "Forest", "#1a7431"),
            (2, "Agriculture", "#f2d43d"),
            (3, "Barren", "#b08a5a"),
            (4, "Built-up", "#d7191c"),
            (5, "Water", "#2b7cb5"),
            (6, "Grassland", "#a3d977"),
            (7, "Shrubland", "#8f9e3d"),
            (8, "Other", "#9e9e9e"),
        ],
    ),
    (
        "Auto 7 (from ESA WorldCover)",
        &[
            (1, "Water", "#0000ff"),
            (2, "Vegetation", "#228b22"),
            (3, "Built-up", "#ff0000"),
            (4, "Bare soil", "#d2b48c"),
            (5, "Wetland", "#00ced1"),
            (6, "Cropland", "#ffa500"),
            (7, "Grassland", "#7fff00"),
        ],
    ),
    (
        "ESA WorldCover (11 classes)",
        &[
            (10, "Tree cover", "#006400"),
            (20, "Shrubland", "#ffbb22"),
            (30, "Grassland", "#ffff4c"),
            (40, "Cropland", "#f096ff"),
            (50, "Built-up", "#fa0000"),
            (60, "Bare / sparse vegetation", "#b4b4b4"),
            (70, "Snow and ice", "#f0f0f0"),
            (80, "Permanent water", "#0064c8"),
            (90, "Herbaceous wetland", "#0096a0"),
            (95, "Mangroves", "#00cf75"),
            (100, "Moss and lichen", "#fae6a0"),
        ],
    ),
];

/// A colour for clusters and classes with none: the plugin's palette.
const PALETTE: [&str; 20] = [
    "#2b83ba", "#d7191c", "#f2e35b", "#1a7f3c", "#9bd06b", "#c2a079", "#984ea3", "#ff7f00", "#a6cee3", "#fb9a99", "#b15928", "#6a3d9a",
    "#33a02c", "#e31a1c", "#1f78b4", "#ffff99", "#8dd3c7", "#bebada", "#80b1d3", "#fdb462",
];

pub fn parse_hex(t: &str) -> Option<[u8; 3]> {
    let h = t.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    Some([c(0)?, c(2)?, c(4)?])
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn palette(i: usize) -> [u8; 3] {
    parse_hex(PALETTE[i % PALETTE.len()]).unwrap_or([128, 128, 128])
}

pub fn scheme(i: usize) -> Vec<ClassDef> {
    SCHEMES
        .get(i)
        .map(|s| s.1)
        .unwrap_or_default()
        .iter()
        .map(|(id, n, c)| ClassDef { id: *id, name: (*n).to_string(), color: parse_hex(c).unwrap_or([128, 128, 128]) })
        .collect()
}

fn col(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// A coloured button for a class, its text dark or light as the colour needs.
fn class_button(ui: &mut Ui, c: &ClassDef, selected: bool) -> egui::Response {
    let fill = col(c.color);
    let light = u32::from(c.color[0]) * 299 + u32::from(c.color[1]) * 587 + u32::from(c.color[2]) * 114 > 140_000;
    let text = egui::RichText::new(format!("{} {}", c.id, tl!(&c.name))).color(if light { Color32::BLACK } else { Color32::WHITE });
    let text = if selected { text.strong() } else { text };
    let stroke = if selected { egui::Stroke::new(2.5, Color32::BLACK) } else { egui::Stroke::new(1.0, Color32::from_gray(120)) };
    ui.add(egui::Button::new(text).fill(fill).stroke(stroke).corner_radius(4.0))
}

// ------------------------------------------------------------------------------------ the list

/// "Classes (legend)": the one class list for clusters, training, accuracy and change.
pub fn classes_ui(app: &mut RustGisApp, ui: &mut Ui) {
    egui::CollapsingHeader::new(egui::RichText::new(format!("🎨 {}", tl!("Classes (legend)"))).strong()).id_salt("rslab_classes").default_open(false).show(ui, |ui| {
        ui.label(
            egui::RichText::new(tl!("One class list for the whole tab: cluster labelling, training, classification, accuracy and change. Click a colour to change it."))
                .weak(),
        );
        {
        let st = &mut app.ui.rslab;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("rslab_scheme").selected_text(tl!(SCHEMES[st.scheme.min(SCHEMES.len() - 1)].0)).width(220.0).truncate().show_ui(ui, |ui| {
                for (i, (name, _)) in SCHEMES.iter().enumerate() {
                    ui.selectable_value(&mut st.scheme, i, tl!(name));
                }
            });
            if ui.button(tl!("Use scheme")).clicked() {
                st.class_list = scheme(st.scheme);
            }
        });
        let mut remove = None;
        egui::Grid::new("rslab_class_grid").num_columns(4).spacing([6.0, 3.0]).show(ui, |ui| {
            for h in ["ID", "Name", "Colour", ""] {
                ui.label(egui::RichText::new(tl!(h)).small().strong());
            }
            ui.end_row();
            for (i, c) in st.class_list.iter_mut().enumerate() {
                ui.add(egui::DragValue::new(&mut c.id).range(0..=255));
                ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(150.0));
                ui.color_edit_button_srgb(&mut c.color);
                if ui.small_button("✕").on_hover_text(tl!("Remove")).clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            st.class_list.remove(i);
        }
        }
        ui.horizontal(|ui| {
            if ui.button(tl!("Add")).clicked() {
                let st = &mut app.ui.rslab;
                let id = st.class_list.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                let n = st.class_list.len();
                st.class_list.push(ClassDef { id, name: format!("Class {id}"), color: palette(n) });
            }
            if ui.button(tl!("Save…")).clicked() {
                let text = classes_csv(&app.ui.rslab.class_list);
                save_text(app, "classes.csv", text);
            }
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button(tl!("Open…")).clicked()
                && let Some(path) = crate::dialogs::pick_open(&["csv"], "CSV")
            {
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| read_classes_csv(&t)) {
                    Ok(c) => app.ui.rslab.class_list = c,
                    Err(e) => say(app, false, e),
                }
            }
        });
    });
}

pub fn classes_csv(classes: &[ClassDef]) -> String {
    let mut out = String::from("id,name,colour\n");
    for c in classes {
        let name = if c.name.contains([',', '"']) { format!("\"{}\"", c.name.replace('"', "\"\"")) } else { c.name.clone() };
        out.push_str(&format!("{},{},{}\n", c.id, name, hex(c.color)));
    }
    out
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn read_classes_csv(text: &str) -> Result<Vec<ClassDef>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}');
        // id, name (quoted when it holds a comma), colour.
        let (id, rest) = line.split_once(',').unwrap_or((line, ""));
        let Ok(id) = id.trim().parse::<u32>() else { continue };
        let (name, colour) = if let Some(q) = rest.trim_start().strip_prefix('"') {
            let end = q.find("\",").unwrap_or(q.len().saturating_sub(1));
            (q.get(..end).unwrap_or("").replace("\"\"", "\""), q.get(end + 2..).unwrap_or(""))
        } else {
            let (n, c) = rest.split_once(',').unwrap_or((rest, ""));
            (n.trim().to_string(), c)
        };
        let n = out.len();
        out.push(ClassDef { id, name, color: parse_hex(colour).unwrap_or_else(|| palette(n)) });
    }
    if out.is_empty() { Err(tl!("No classes in the file (id,name,colour per line).").to_string()) } else { Ok(out) }
}

/// Saves text where the user picks (on the desktop), or downloads it (in the browser).
fn save_text(app: &mut RustGisApp, name: &str, text: String) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ext = name.rsplit('.').next().unwrap_or("csv");
        if let Some(path) = crate::dialogs::pick_save(name, &[ext], &ext.to_uppercase()) {
            match std::fs::write(&path, text) {
                Ok(()) => say(app, true, crate::i18n::fmt("Saved {file}", &[("file", &path)])),
                Err(e) => say(app, false, e.to_string()),
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let r = match app.services.download.as_ref() {
            Some(d) => d(name, text.into_bytes()),
            None => Err("this page can't download files".into()),
        };
        if let Err(e) = r {
            say(app, false, e);
        }
    }
}

// ------------------------------------------------------------------------------------ clusters

/// A cluster, as the table lists it.
#[derive(Clone, Debug)]
pub struct ClusterRow {
    pub value: f64,
    pub hectares: f64,
    pub percent: f64,
    pub means: Vec<f64>,
    pub examples: Vec<[f64; 2]>,
    pub shown: usize,
    pub hint: &'static str,
    /// The class it goes into (an id of the list).
    pub to: Option<u32>,
}

/// The clusters of a class map, waiting for their classes.
#[derive(Clone, Debug, Default)]
pub struct Clusters {
    pub map: String,
    pub image: String,
    pub bands: Vec<String>,
    pub rows: Vec<ClusterRow>,
    pub selected: Option<usize>,
}

/// The clusters from `raster.clusterStats`' answer, their classes kept when the map is the same.
fn from_stats(v: &Value, before: Option<&Clusters>) -> Clusters {
    let bands: Vec<String> = v["bands"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
    let rows = v["clusters"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let means: Vec<f64> = c["means"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
            let value = c["value"].as_f64().unwrap_or(0.0);
            let to = before.and_then(|b| b.rows.iter().find(|r| r.value == value)).and_then(|r| r.to);
            ClusterRow {
                value,
                hectares: c["hectares"].as_f64().unwrap_or(0.0),
                percent: c["percent"].as_f64().unwrap_or(0.0),
                hint: hint(&bands, &means),
                means,
                examples: c["examples"].as_array().into_iter().flatten().filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?])).collect(),
                shown: 0,
                to,
            }
        })
        .collect();
    Clusters { map: v["layer"].as_str().unwrap_or("").into(), image: v["image"].as_str().unwrap_or("").into(), bands, rows, selected: None }
}

/// What a cluster may be, from its mean spectrum (the plugin's rules): a key of [`HINTS`].
pub fn hint(bands: &[String], means: &[f64]) -> &'static str {
    use std::collections::HashMap;
    let mut m: HashMap<String, f64> = HashMap::new();
    for (b, v) in bands.iter().zip(means) {
        let l = b.to_ascii_lowercase().replace(' ', "_");
        m.insert(l.clone(), *v);
        // The satellites' own names, and the dB radar bands.
        let alias = match l.as_str() {
            "b02" | "b2" | "sr_b2" => "blue",
            "b03" | "b3" | "sr_b3" => "green",
            "b04" | "b4" | "sr_b4" => "red",
            "b08" | "b8" | "nir08" | "sr_b5" => "nir",
            "b11" | "swir16" | "sr_b6" => "swir1",
            "b12" | "swir22" | "sr_b7" => "swir2",
            "vv_db" => "vv",
            "vh_db" => "vh",
            _ => "",
        };
        if !alias.is_empty() {
            m.entry(alias.into()).or_insert(*v);
        }
    }
    let nd = |a: f64, b: f64| if (a + b).abs() > 1e-9 { (a - b) / (a + b) } else { 0.0 };
    let g = |k: &str| m.get(k).copied();
    // A multi-season stack: the dry season (late dry, else early dry) and the wet one.
    let season = |s: &str, band: &str| g(&format!("{s}_{band}"));
    let dry = |band: &str| season("dry2", band).or_else(|| season("dry1", band));
    let ndvi_of = |get: &dyn Fn(&str) -> Option<f64>| get("ndvi").or_else(|| Some(nd(get("nir")?, get("red")?)));
    let mndwi_of = |get: &dyn Fn(&str) -> Option<f64>| get("mndwi").or_else(|| Some(nd(get("green")?, get("swir1")?)));
    if let (Some(nd_dry), Some(nd_wet)) = (ndvi_of(&dry), ndvi_of(&|b| season("wet", b))) {
        let amp = (nd_wet - nd_dry).abs();
        let mw_d = mndwi_of(&dry).unwrap_or(-0.5);
        let mw_g = mndwi_of(&|b| season("wet", b)).unwrap_or(-0.5);
        let ndbi = dry("swir1").zip(dry("nir")).map_or(-0.2, |(s, n)| nd(s, n));
        return if mw_d > 0.05 && nd_wet < 0.3 {
            "water"
        } else if nd_wet < 0.1 && mw_g > 0.0 && nd_dry >= 0.45 {
            "flooded"
        } else if mw_g > 0.0 && nd_dry < 0.4 {
            "wetland"
        } else if nd_wet < 0.4 && ndbi > -0.05 {
            "built"
        } else if nd_wet < 0.25 && nd_dry < 0.25 {
            "bare"
        } else if nd_dry < 0.35 && amp >= 0.2 {
            "crop"
        } else if nd_dry < 0.3 && nd_wet < 0.5 {
            "grass"
        } else if nd_dry >= 0.72 || (nd_dry >= 0.5 && amp < 0.25) {
            "orchard"
        } else if nd_dry < 0.5 && amp >= 0.15 {
            "deciduous"
        } else {
            "shrub"
        };
    }
    let Some(ndvi) = ndvi_of(&g) else { return "none" };
    let mw = mndwi_of(&g);
    let nb = g("ndbi").or_else(|| Some(nd(g("swir1")?, g("nir")?)));
    let vv = g("vv");
    if mw.is_some_and(|w| w > 0.1) && ndvi < 0.25 {
        "water"
    } else if ndvi < 0.25 {
        if vv.is_some_and(|v| v > -8.0) || nb.is_some_and(|b| b > 0.05) { "built" } else { "bare" }
    } else if ndvi >= 0.65 {
        "forest"
    } else if ndvi >= 0.45 {
        "shrub"
    } else if ndvi >= 0.3 {
        "crop"
    } else {
        "grass"
    }
}

/// A hint's words, and the class names it fits (tried in order).
pub const HINTS: [(&str, &str, &[&str]); 13] = [
    ("water", "water (open, all year)", &["water"]),
    ("flooded", "flooded in the wet season, green in the dry: flooded forest / recession rice?", &["flood", "wetland", "mangrove"]),
    ("wetland", "flooded after the rains: wetland / seasonal water?", &["wetland", "flood"]),
    ("built", "bright, rough: built-up?", &["built", "urban", "settle"]),
    ("bare", "no vegetation: bare soil / sand?", &["bare", "barren", "sand"]),
    ("crop", "bare in the dry season, green after the rains: crop / rice?", &["rice", "crop", "agri", "paddy"]),
    ("grass", "low greenness: grass / open land?", &["grass", "rangeland", "shrub /", "vegetation"]),
    ("orchard", "green all year: tree crop / evergreen?", &["orchard", "plantation", "tree", "forest", "agri"]),
    ("deciduous", "leafless in the dry season: deciduous forest?", &["deciduous", "forest", "tree", "vegetation"]),
    ("shrub", "medium greenness: shrubland?", &["shrub", "vegetation"]),
    ("forest", "dense vegetation: forest?", &["forest", "tree", "vegetation"]),
    ("rice", "rice?", &["rice", "paddy", "crop"]),
    ("none", "", &[]),
];

fn hint_text(key: &str) -> &'static str {
    HINTS.iter().find(|h| h.0 == key).map_or("", |h| h.1)
}

/// The class of the list a hint fits, by its name.
pub fn suggest(key: &str, classes: &[ClassDef]) -> Option<u32> {
    let words = HINTS.iter().find(|h| h.0 == key)?.2;
    words.iter().find_map(|w| classes.iter().find(|c| c.name.to_lowercase().contains(w)).map(|c| c.id))
}

/// The steps after "Run clustering": its statistics, so the table fills in.
pub fn stats_step(map: &str, image: &str) -> crate::rslab::Step {
    crate::rslab::keep_step("raster.clusterStats", json!({"layer": map, "image": image}), "clusters")
}

/// The colour a class map draws a value in.
fn drawn_colour(app: &RustGisApp, layer: &str, value: f64) -> Option<Color32> {
    let l = app.session.map().layer_by_name(layer)?;
    match &l.raster.as_ref()?.style.renderer {
        rustgis_engine::map::raster::RasterRenderer::UniqueValues { values, .. } => {
            values.iter().find(|(v, _)| *v == value).map(|(_, c)| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 255))
        }
        _ => None,
    }
}

/// The cluster table under "Run clustering", once there are clusters.
pub fn clusters_ui(app: &mut RustGisApp, ui: &mut Ui) {
    // New statistics (a clustering finished, or a cluster split): the table, its classes kept.
    if let Some(v) = result(app, "clusters").cloned() {
        let layer = v["layer"].as_str().unwrap_or("");
        let changed =
            app.ui.rslab.clusters.as_ref().is_none_or(|c| c.map != layer || c.rows.len() != v["clusters"].as_array().map_or(0, Vec::len));
        if changed && !layer.is_empty() {
            let before = app.ui.rslab.clusters.take();
            app.ui.rslab.clusters = Some(from_stats(&v, before.as_ref()));
        }
    }
    let Some(cl) = app.ui.rslab.clusters.clone() else { return };
    if app.session.map().layer_by_name(&cl.map).is_none() {
        app.ui.rslab.clusters = None;
        return;
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Label the clusters: choose a class for each cluster — clusters with the same class are merged. Click a cluster to see an example of it on the map.")).weak());
    let classes = app.ui.rslab.class_list.clone();
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut look: Option<usize> = None;
    // Each cluster on two lines, to fit the panel: its colour, number, area and class; then its
    // hint and spectrum.
    for (i, row) in cl.rows.iter().enumerate() {
        let swatch = drawn_colour(app, &cl.map, row.value).unwrap_or_else(|| col(palette(i)));
        let sel = cl.selected == Some(i);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, swatch);
            if ui
                .add_sized([30.0, 18.0], egui::Button::selectable(sel, format!("{}", row.value as i64)))
                .on_hover_text(tl!("Show an example pixel of this cluster on the map (click again for another)"))
                .clicked()
            {
                look = Some(i);
            }
            ui.add_sized(
                [110.0, 18.0],
                egui::Label::new(egui::RichText::new(format!("{:.1} ha · {:.1}%", row.hectares, row.percent)).small()),
            );
            let current =
                row.to.and_then(|id| classes.iter().find(|c| c.id == id)).map_or("→ —".to_string(), |c| format!("→ {}", tl!(&c.name)));
            let mut chosen = row.to;
            egui::ComboBox::from_id_salt(("rslab_to", i)).selected_text(current).width(130.0).truncate().show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "—");
                for c in &classes {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, col(c.color));
                        ui.selectable_value(&mut chosen, Some(c.id), format!("{} {}", c.id, tl!(&c.name)));
                    });
                }
            });
            if chosen != row.to
                && let Some(r) = app.ui.rslab.clusters.as_mut().and_then(|c| c.rows.get_mut(i))
            {
                r.to = chosen;
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(48.0);
            sparkline(ui, &row.means, swatch, &t);
            ui.label(egui::RichText::new(tl!(hint_text(row.hint))).small().color(t.text_dim));
        });
    }
    if let Some(i) = look {
        show_example(app, i);
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tl!("Suggest classes from hints"))
            .on_hover_text(tl!("Fills the empty → class boxes from the hints. Check them on the basemap."))
            .clicked()
        {
            let classes = app.ui.rslab.class_list.clone();
            let mut n = 0;
            if let Some(c) = app.ui.rslab.clusters.as_mut() {
                for r in c.rows.iter_mut().filter(|r| r.to.is_none()) {
                    if let Some(id) = suggest(r.hint, &classes) {
                        r.to = Some(id);
                        n += 1;
                    }
                }
            }
            say(app, true, crate::i18n::fmt("{n} clusters filled from hints — check them on the basemap.", &[("n", &n.to_string())]));
        }
    });
    ui.horizontal_wrapped(|ui| {
        let selected = app.ui.rslab.clusters.as_ref().and_then(|c| c.selected);
        if ui
            .add_enabled(selected.is_some(), egui::Button::new(tl!("Split selected cluster into")))
            .on_hover_text(tl!("Re-clusters a mixed cluster into 2–6 sub-clusters (click a cluster first)."))
            .clicked()
            && let Some(i) = selected
        {
            split(app, i);
        }
        ui.add(egui::DragValue::new(&mut app.ui.rslab.split_n).range(2.0..=6.0).speed(0.05).fixed_decimals(0));
    });
    ui.horizontal_wrapped(|ui| {
        let on = app.ui.rslab.show_spectra;
        if ui.add(egui::Button::selectable(on, tl!("Plot cluster spectra"))).clicked() {
            app.ui.rslab.show_spectra = !on;
        }
        if ui.button(egui::RichText::new(tl!("Create labelled map")).strong()).clicked() {
            labelled_map(app);
        }
    });
    if app.ui.rslab.show_spectra
        && let Some(cl) = app.ui.rslab.clusters.clone()
    {
        let series: Vec<(String, Color32, Vec<f64>)> = cl
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    format!("{} {}", tl!("Cluster"), r.value as i64),
                    drawn_colour(app, &cl.map, r.value).unwrap_or_else(|| col(palette(i))),
                    r.means.clone(),
                )
            })
            .collect();
        spectra_chart(ui, &cl.bands, &series, &t);
    }
}

/// A small line of a spectrum, scaled to its own range.
fn sparkline(ui: &mut Ui, v: &[f64], c: Color32, t: &crate::theme::Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(60.0, 14.0), egui::Sense::hover());
    if v.len() < 2 {
        return;
    }
    let (lo, hi) = v.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, x| (a.0.min(*x), a.1.max(*x)));
    let span = (hi - lo).max(1e-9);
    let pts: Vec<egui::Pos2> = v
        .iter()
        .enumerate()
        .map(|(i, x)| {
            egui::pos2(
                rect.left() + rect.width() * i as f32 / (v.len() - 1) as f32,
                rect.bottom() - 2.0 - (rect.height() - 4.0) * ((x - lo) / span) as f32,
            )
        })
        .collect();
    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(0.5, t.border), egui::StrokeKind::Inside);
    ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.5, c)));
}

/// Several spectra on one chart: bands across, values up.
pub fn spectra_chart(ui: &mut Ui, bands: &[String], series: &[(String, Color32, Vec<f64>)], t: &crate::theme::Tokens) {
    let all = series.iter().flat_map(|s| s.2.iter().copied());
    let (lo, hi) = all.fold((f64::INFINITY, f64::NEG_INFINITY), |a, x| (a.0.min(x), a.1.max(x)));
    if !lo.is_finite() || bands.is_empty() {
        return;
    }
    let (lo, hi) = if hi > lo { (lo.min(0.0), hi) } else { (lo - 1.0, hi + 1.0) };
    // The legend under the plot, two to a line.
    let shown = series.len().min(16);
    let legend_h = shown.div_ceil(2) as f32 * 13.0 + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2((ui.available_width() - 16.0).clamp(200.0, 460.0), 180.0 + legend_h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let plot = egui::Rect::from_min_max(rect.min + vec2(38.0, 6.0), egui::pos2(rect.max.x - 6.0, rect.max.y - 34.0 - legend_h));
    p.rect_stroke(plot, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let n = bands.len();
    let x = |i: usize| plot.left() + plot.width() * (i as f32 + 0.5) / n as f32;
    let y = |v: f64| plot.bottom() - plot.height() * ((v - lo) / (hi - lo)) as f32;
    let font = egui::FontId::proportional(9.5);
    for (i, b) in bands.iter().enumerate() {
        let label: String = b.chars().take(9).collect();
        p.text(egui::pos2(x(i), plot.bottom() + 3.0 + (i % 2) as f32 * 11.0), egui::Align2::CENTER_TOP, label, font.clone(), t.text_dim);
    }
    for v in [lo, (lo + hi) / 2.0, hi] {
        p.text(egui::pos2(plot.left() - 3.0, y(v)), egui::Align2::RIGHT_CENTER, format!("{v:.2}"), font.clone(), t.text_dim);
    }
    for (k, (label, c, vals)) in series.iter().enumerate() {
        let pts: Vec<egui::Pos2> = vals.iter().enumerate().take(n).map(|(i, v)| egui::pos2(x(i), y(*v))).collect();
        p.add(egui::Shape::line(pts, egui::Stroke::new(1.8, *c)));
        if k < shown {
            let lx = rect.left() + 8.0 + (k % 2) as f32 * rect.width() / 2.0;
            let ly = plot.bottom() + 30.0 + (k / 2) as f32 * 13.0;
            p.line_segment([egui::pos2(lx, ly + 5.0), egui::pos2(lx + 14.0, ly + 5.0)], egui::Stroke::new(2.5, *c));
            let text: String = label.chars().take(28).collect();
            p.text(egui::pos2(lx + 18.0, ly), egui::Align2::LEFT_TOP, text, font.clone(), t.text);
        }
    }
}

/// Zooms to an example pixel of cluster `i` (the next one each time), on imagery to judge it by.
fn show_example(app: &mut RustGisApp, i: usize) {
    let Some(c) = app.ui.rslab.clusters.as_mut() else { return };
    c.selected = Some(i);
    let Some(row) = c.rows.get_mut(i) else { return };
    if row.examples.is_empty() {
        return say(app, false, tl!("No pixels in this cluster.").to_string());
    }
    let k = row.shown % row.examples.len();
    row.shown += 1;
    let (p, n, value) = (row.examples[k], row.examples.len(), row.value as i64);
    let d = 0.004;
    let _ = app.run("map.zoomToExtent", json!({"extent": [p[0] - d, p[1] - d, p[0] + d, p[1] + d]}));
    // Imagery under it, as the plugin does, unless it is there already.
    let imagery = app.session.map().tiles.iter().any(|t| !t.overlay && t.name.to_lowercase().contains("imagery"));
    if !imagery
        && let Some((name, url, attribution, max_zoom)) = rustgis_engine::sample::WEB_BASEMAPS.iter().find(|b| b.0.contains("Imagery"))
    {
        let _ = app.run(
            "map.addTiles",
            json!({"url": url, "name": name, "attribution": attribution, "maxZoom": max_zoom, "replace": true, "kind": "raster"}),
        );
    }
    app.map.flash = Some(rustgis_engine::geom::Coord::new(p[0], p[1]));
    say(
        app,
        true,
        crate::i18n::fmt(
            "Cluster {c}: example {k} of {n}",
            &[("c", &value.to_string()), ("k", &(k + 1).to_string()), ("n", &n.to_string())],
        ),
    );
}

fn split(app: &mut RustGisApp, i: usize) {
    let Some(c) = app.ui.rslab.clusters.clone() else { return };
    let Some(row) = c.rows.get(i) else { return };
    let n = app.ui.rslab.split_n.round().clamp(2.0, 6.0) as u64;
    let out = fresh(app, &format!("{}_split", c.map));
    let p = json!({"layer": c.map, "image": c.image, "value": row.value, "classes": n, "name": out});
    if let Some(v) = now(app, "raster.splitCluster", p) {
        let made = v["newValues"].as_array().map_or(0, Vec::len);
        if let Some(stats) = now(app, "raster.clusterStats", json!({"layer": out, "image": c.image})) {
            // The new map takes the old one's place, its other clusters' classes kept.
            let mut next = from_stats(&stats, Some(&c));
            next.selected = None;
            app.ui.rslab.clusters = Some(next);
            keep_result(app, "clusters", stats);
            let _ = app.run("layer.visibility", json!({"layer": c.map, "visible": false}));
        }
        app.ui.rslab.class_map = out;
        say(
            app,
            true,
            crate::i18n::fmt("Cluster {c} split into {n}.", &[("c", &(row.value as i64).to_string()), ("n", &made.to_string())]),
        );
    }
}

/// The clusters merged into their classes: a class map with the list's names and colours.
pub fn labelled_map(app: &mut RustGisApp) {
    let Some(c) = app.ui.rslab.clusters.clone() else { return };
    let classes = app.ui.rslab.class_list.clone();
    let mut map = Map::new();
    let (mut names, mut colors) = (Map::new(), Map::new());
    for r in &c.rows {
        if let Some(id) = r.to {
            map.insert(format!("{}", r.value as i64), json!(id));
            if let Some(cd) = classes.iter().find(|x| x.id == id) {
                names.insert(id.to_string(), json!(cd.name));
                colors.insert(id.to_string(), json!(hex(cd.color)));
            }
        }
    }
    if map.is_empty() {
        return say(app, false, tl!("Choose a class for each cluster first.").to_string());
    }
    let left = c.rows.iter().filter(|r| r.to.is_none()).count();
    let out = fresh(app, &format!("{}_labelled", c.image));
    if now(app, "raster.recode", json!({"layer": c.map, "map": map, "names": names, "colors": colors, "name": out})).is_some() {
        app.ui.rslab.class_map = out.clone();
        let _ = app.run("layer.visibility", json!({"layer": c.map, "visible": false}));
        if let Some(v) = now(app, "raster.classAreas", json!({"layer": out})) {
            app.ui.rslab.class_labels = v["classes"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| Some((c["value"].as_f64()? as f32, c["class"].as_str()?.to_string()))).collect())
                .unwrap_or_default();
            keep_result(app, "areas", v);
        }
        let mut text = crate::i18n::fmt("{name} made: the clusters merged into their classes.", &[("name", &out)]);
        if left > 0 {
            text.push(' ');
            text.push_str(&crate::i18n::fmt("{n} cluster(s) with no class are left out (no data).", &[("n", &left.to_string())]));
        }
        say(app, true, text);
    }
}

/// Puts the class list's colours (and ids' names) on a class map, matching its classes by name.
pub fn apply_colours(app: &mut RustGisApp, map: &str) {
    let Some(r) = app.session.map().layer_by_name(map).and_then(|l| l.raster.as_ref()?.data.clone()) else { return };
    let names: Vec<String> = r.bands.first().map(|b| b.classes.clone()).unwrap_or_default();
    let classes = app.ui.rslab.class_list.clone();
    let mut colors = Map::new();
    for (v, n) in names.iter().enumerate() {
        if n.is_empty() {
            continue;
        }
        if let Some(c) = classes.iter().find(|c| c.name.eq_ignore_ascii_case(n.trim())) {
            colors.insert(v.to_string(), json!(hex(c.color)));
        }
    }
    // A map whose classes have no names: by value, as ids.
    if colors.is_empty() {
        for c in &classes {
            if (c.id as usize) < 100_000
                && rustgis_engine::analysis::rslab::counts(&r, 0).is_ok_and(|cs| cs.iter().any(|(v, _)| *v as u32 == c.id))
            {
                colors.insert(c.id.to_string(), json!(hex(c.color)));
            }
        }
    }
    if colors.is_empty() {
        return say(app, false, tl!("None of the map's classes is in the class list (by name or id).").to_string());
    }
    let n = colors.len();
    if now(app, "raster.classNames", json!({"layer": map, "names": {}, "colors": colors})).is_some() {
        say(app, true, crate::i18n::fmt("{n} classes coloured from the class list.", &[("n", &n.to_string())]));
    }
}

// ------------------------------------------------------------------- training signatures

/// "Training signatures": each training class's mean spectrum, to see which classes separate.
pub fn signatures_ui(app: &mut RustGisApp, ui: &mut Ui, image: &str) {
    let (training, field) = (app.ui.rslab.training.clone(), app.ui.rslab.class_field.clone());
    if ui
        .add_enabled(!image.is_empty() && !training.is_empty() && !field.is_empty(), egui::Button::new(tl!("Training signatures")))
        .on_hover_text(tl!("Each training class's mean spectrum: classes whose lines lie close together will be confused"))
        .clicked()
        && let Some(v) = now(app, "raster.signatures", json!({"layer": image, "training": training, "classField": field}))
    {
        keep_result(app, "signatures", v);
    }
    let Some(v) = result(app, "signatures").cloned() else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    let bands: Vec<String> = v["bands"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
    let classes = app.ui.rslab.class_list.clone();
    let series: Vec<(String, Color32, Vec<f64>)> = v["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, c)| {
            let name = c["class"].as_str().unwrap_or("").to_string();
            let colour =
                classes.iter().find(|x| x.name.eq_ignore_ascii_case(name.trim())).map_or_else(|| col(palette(i)), |x| col(x.color));
            let label = format!("{name} ({})", c["pixels"]);
            (label, colour, c["means"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect())
        })
        .collect();
    spectra_chart(ui, &bands, &series, &t);
}

// ---------------------------------------------------------------------- the classes' areas

/// The class areas as bars, in the map's colours.
pub fn area_bars(app: &RustGisApp, ui: &mut Ui, map: &str, rows: &[Value]) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let most = rows.iter().filter_map(|c| c["hectares"].as_f64()).fold(0.0, f64::max).max(1e-9);
    for (i, c) in rows.iter().enumerate() {
        let ha = c["hectares"].as_f64().unwrap_or(0.0);
        let value = c["value"].as_f64().unwrap_or(0.0);
        let colour = drawn_colour(app, map, value).unwrap_or_else(|| col(palette(i)));
        let room = (ui.available_width() - 190.0).clamp(60.0, 220.0);
        ui.horizontal(|ui| {
            ui.add_sized([100.0, 14.0], egui::Label::new(egui::RichText::new(c["class"].as_str().unwrap_or("")).small()).truncate());
            let w = room * (ha / most) as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(room, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(w.max(1.0), 12.0)), 2.0, colour);
            ui.label(egui::RichText::new(format!("{ha:.1} ha")).small().color(t.text_dim));
        });
    }
}

// ------------------------------------------------------------------- labelling the points

/// The validation points: (row, lon, lat, label, map class).
fn points(app: &RustGisApp, layer: &str, field: &str) -> Option<Vec<(usize, f64, f64, String, String)>> {
    let l = app.session.map().layer_by_name(layer)?;
    let fi = l.dataset.field_index(field)?;
    let mi = l.dataset.field_index("map_class");
    Some(
        l.dataset
            .features
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let c = match f.geometry.as_ref()? {
                    rustgis_engine::data::Geometry::Point(c) => *c,
                    g => {
                        let e = g.extent();
                        rustgis_engine::geom::Coord::new((e.min_x + e.max_x) / 2.0, (e.min_y + e.max_y) / 2.0)
                    }
                };
                let text =
                    |k: Option<usize>| k.and_then(|k| f.attrs.get(k)).map(|v| v.to_string()).filter(|s| s != "NULL").unwrap_or_default();
                Some((i, c.x, c.y, text(Some(fi)), text(mi)))
            })
            .collect(),
    )
}

/// The classes to label with: the map's own (by name), then the list's.
fn label_classes(app: &RustGisApp, map: &str) -> Vec<ClassDef> {
    let mut out: Vec<ClassDef> = Vec::new();
    if let Some(r) = app.session.map().layer_by_name(map).and_then(|l| l.raster.as_ref()?.data.clone()) {
        let names: Vec<String> = r.bands.first().map(|b| b.classes.clone()).unwrap_or_default();
        for (v, n) in names.iter().enumerate().filter(|(_, n)| !n.is_empty()) {
            let colour = drawn_colour(app, map, v as f64).map_or([150, 150, 150], |c| [c.r(), c.g(), c.b()]);
            out.push(ClassDef { id: v as u32, name: n.clone(), color: colour });
        }
    }
    if out.is_empty() {
        out = app.ui.rslab.class_list.clone();
    }
    out
}

/// "Label the points": one point at a time, zoomed to, labelled with a button per class.
pub fn label_points_ui(app: &mut RustGisApp, ui: &mut Ui, map: &str) {
    let (layer, field) = (app.ui.rslab.reference.clone(), app.ui.rslab.ref_field.clone());
    let Some(pts) = points(app, &layer, &field) else { return };
    if pts.is_empty()
        || app.session.map().layer_by_name(&layer).is_some_and(|l| l.geometry_type() != rustgis_engine::data::GeometryType::Point)
    {
        return;
    }
    egui::CollapsingHeader::new(egui::RichText::new(tl!("Label the points")).strong())
        .id_salt("rslab_label_points")
        .default_open(true)
        .show(ui, |ui| {
            let done = pts.iter().filter(|p| !p.3.trim().is_empty()).count();
            let i = app.ui.rslab.lb_row.filter(|i| *i < pts.len());
            let names: Vec<ClassDef> = label_classes(app, map);
            let mut text = match i.and_then(|i| pts.get(i)) {
                Some(p) => crate::i18n::fmt(
                    "Point {i} / {n} · label: {label}",
                    &[("i", &(p.0 + 1).to_string()), ("n", &pts.len().to_string()), ("label", if p.3.is_empty() { "—" } else { &p.3 })],
                ),
                None => tl!("Look at each point on high-resolution imagery and press its class.").to_string(),
            };
            if app.ui.rslab.lb_hint
                && let Some(p) = i.and_then(|i| pts.get(i))
            {
                text.push_str(&format!(" · {}: {}", tl!("map"), p.4));
            }
            text.push_str(&format!(" · {} {done}/{}", tl!("done"), pts.len()));
            ui.label(text);
            let mut go: Option<usize> = None;
            let mut assign: Option<Option<String>> = None;
            ui.horizontal_wrapped(|ui| {
                if ui.button("◀").clicked() {
                    go = Some(i.map_or(pts.len() - 1, |i| (i + pts.len() - 1) % pts.len()));
                }
                if ui.button("▶").clicked() {
                    go = Some(i.map_or(0, |i| (i + 1) % pts.len()));
                }
                if ui.button(tl!("Next unlabelled")).clicked() {
                    go = next_empty(&pts, i);
                    if go.is_none() {
                        say(app, true, tl!("All points are labelled — compute the accuracy.").to_string());
                    }
                }
                if ui.add_enabled(i.is_some(), egui::Button::new(tl!("Clear label"))).clicked() {
                    assign = Some(None);
                }
            });
            ui.horizontal_wrapped(|ui| {
                for c in &names {
                    let current = i.and_then(|i| pts.get(i)).is_some_and(|p| p.3.trim().eq_ignore_ascii_case(c.name.trim()));
                    if ui.add_enabled_ui(i.is_some(), |ui| class_button(ui, c, current)).inner.clicked() {
                        assign = Some(Some(c.name.clone()));
                    }
                }
            });
            ui.checkbox(&mut app.ui.rslab.lb_hint, tl!("Show the map class (hint) — off for blind labelling"));
            if let (Some(a), Some(i)) = (assign, i) {
                let _ = app.run("edit.enable", json!({"on": true}));
                if let Some(row) = pts.get(i).map(|p| p.0)
                    && let Err(e) = app
                        .run("edit.setValue", json!({"layer": layer, "row": row, "field": field, "value": a.clone().unwrap_or_default()}))
                {
                    say(app, false, e);
                } else if a.is_some() {
                    // On to the next point with no label.
                    let mut pts = pts.clone();
                    if let Some(p) = pts.get_mut(i) {
                        p.3 = "x".into();
                    }
                    go = next_empty(&pts, Some(i));
                }
            }
            if let Some(g) = go {
                app.ui.rslab.lb_row = Some(g);
                if let Some(p) = pts.get(g) {
                    let d = 0.0025;
                    let _ = app.run("map.zoomToExtent", json!({"extent": [p.1 - d, p.2 - d, p.1 + d, p.2 + d]}));
                    let _ = app.run("select.rows", json!({"layer": layer, "rows": [p.0]}));
                    app.map.flash = Some(rustgis_engine::geom::Coord::new(p.1, p.2));
                }
            }
        });
}

fn next_empty(pts: &[(usize, f64, f64, String, String)], from: Option<usize>) -> Option<usize> {
    let n = pts.len();
    let start = from.map_or(0, |i| i + 1);
    (0..n).map(|k| (start + k) % n).find(|i| pts.get(*i).is_some_and(|p| p.3.trim().is_empty()))
}

// ---------------------------------------------------------------------- the error matrix

/// The accuracy assessment's error matrix and figures as CSV.
pub fn accuracy_csv(v: &Value) -> String {
    let classes: Vec<String> = v["classes"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect();
    let q = |s: &str| if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = format!("map \\ reference,{},total\n", classes.iter().map(|c| q(c)).collect::<Vec<_>>().join(","));
    for (i, c) in classes.iter().enumerate() {
        let row: Vec<u64> = (0..classes.len()).map(|j| v["matrix"][i][j].as_u64().unwrap_or(0)).collect();
        out.push_str(&format!("{},{},{}\n", q(c), row.iter().map(u64::to_string).collect::<Vec<_>>().join(","), row.iter().sum::<u64>()));
    }
    for k in ["overall", "kappa", "weightedOverall", "samples"] {
        if let Some(x) = v[k].as_f64() {
            out.push_str(&format!("{k},{x}\n"));
        }
    }
    out
}

/// The from → to table as CSV: each pair of classes and its area.
pub fn change_csv(v: &Value) -> String {
    let q = |s: &str| if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = String::from("from,to,hectares,changed\n");
    for r in v["matrix"].as_array().into_iter().flatten() {
        let (a, b) = (r["fromClass"].as_str().unwrap_or(""), r["toClass"].as_str().unwrap_or(""));
        out.push_str(&format!("{},{},{:.2},{}\n", q(a), q(b), r["hectares"].as_f64().unwrap_or(0.0), u8::from(a != b)));
    }
    out
}

pub fn export_change(app: &mut RustGisApp, v: &Value) {
    save_text(app, "change.csv", change_csv(v));
}

/// Saves the LEE script, or opens one (on the desktop).
pub fn save_script(app: &mut RustGisApp) {
    let code = app.ui.rslab.lee.code.clone();
    save_text(app, "script.py", code);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open_script(app: &mut RustGisApp) {
    if let Some(path) = crate::dialogs::pick_open(&["py", "txt"], "Python") {
        match std::fs::read_to_string(&path) {
            Ok(t) => app.ui.rslab.lee.code = t,
            Err(e) => say(app, false, e.to_string()),
        }
    }
}

pub fn export_accuracy(app: &mut RustGisApp) {
    if let Some(v) = result(app, "accuracy").cloned() {
        save_text(app, "accuracy.csv", accuracy_csv(&v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_read_the_spectrum() {
        let bands: Vec<String> = ["Blue", "Green", "Red", "NIR", "SWIR1", "SWIR2"].iter().map(|s| s.to_string()).collect();
        assert_eq!(hint(&bands, &[0.05, 0.06, 0.04, 0.03, 0.01, 0.01]), "water");
        assert_eq!(hint(&bands, &[0.03, 0.06, 0.03, 0.40, 0.18, 0.08]), "forest");
        assert_eq!(hint(&bands, &[0.12, 0.14, 0.18, 0.22, 0.30, 0.28]), "built");
        // A multi-season stack: bare in the dry season, green in the wet one.
        let seasons: Vec<String> = ["dry2_Red", "dry2_NIR", "wet_Red", "wet_NIR"].iter().map(|s| s.to_string()).collect();
        assert_eq!(hint(&seasons, &[0.15, 0.20, 0.04, 0.35]), "crop");
        let classes = scheme(0);
        assert_eq!(suggest("water", &classes), Some(1));
        assert_eq!(suggest("crop", &classes), Some(3));
        assert_eq!(suggest("none", &classes), None);
    }

    #[test]
    fn class_lists_round_trip_through_csv() {
        let mut c = scheme(1);
        c[0].name = "Forest, dense".into();
        let back = read_classes_csv(&classes_csv(&c)).unwrap();
        assert_eq!(back, c);
        assert!(read_classes_csv("nothing here").is_err());
        assert_eq!(parse_hex("#1A7f3C"), Some([0x1a, 0x7f, 0x3c]));
    }

    /// Clusters labelled with the list's classes become a class map with their names and colours.
    #[test]
    fn clusters_become_a_labelled_map() {
        use rustgis_engine::{Project, Session};
        let dir = std::env::temp_dir().join(format!("khgis-clusters-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = RustGisApp::new(Session::new(Project::default()));
        let mut names = Vec::new();
        for (file, low, high) in [("red.asc", 5, 30), ("nir.asc", 3, 60)] {
            let rows: String =
                (0..10).map(|_| (0..10).map(|c| if c < 5 { low } else { high }.to_string()).collect::<Vec<_>>().join(" ") + "\n").collect();
            let path = dir.join(file);
            std::fs::write(&path, format!("ncols 10\nnrows 10\nxllcorner 104\nyllcorner 11\ncellsize 0.001\n{rows}")).unwrap();
            let id = app.run("layer.addData", json!({"path": path})).unwrap()["layer"].as_u64().unwrap();
            names.push(app.session.map().layer(id as u32).unwrap().name.clone());
        }
        app.run(
            "raster.process",
            json!({"layer": names[0], "operation": "stack", "layers": [names[1]], "bandNames": ["Red", "NIR"], "name": "img"}),
        )
        .unwrap();
        app.run("raster.classify", json!({"layer": "img", "classes": 2, "name": "km"})).unwrap();
        let stats = app.run("raster.clusterStats", json!({"layer": "km", "image": "img"})).unwrap();
        keep_result(&mut app, "clusters", stats.clone());
        let mut cl = from_stats(&stats, None);
        assert_eq!(cl.rows.len(), 2);
        assert_eq!(cl.bands, ["Red", "NIR"]);
        // Low NIR is water; high NIR the crop (the class list's 1 and 3).
        for r in &mut cl.rows {
            r.to = Some(if r.means[1] > 30.0 { 3 } else { 1 });
        }
        app.ui.rslab.clusters = Some(cl);
        labelled_map(&mut app);
        let out = app.ui.rslab.class_map.clone();
        assert!(out.ends_with("_labelled"), "{out}: {:?}", app.ui.rslab.log);
        let areas = app.run("raster.classAreas", json!({"layer": out})).unwrap();
        let got: Vec<(String, u64)> = areas["classes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["class"].as_str().unwrap().to_string(), c["pixels"].as_u64().unwrap()))
            .collect();
        assert_eq!(got, [("Water".to_string(), 50), ("Rice / cropland".to_string(), 50)]);
        assert_eq!(drawn_colour(&app, &out, 1.0), Some(col(parse_hex("#2b83ba").unwrap())));
        // Unlabelled points are found in turn.
        let pts = vec![(0, 0.0, 0.0, "Water".into(), String::new()), (1, 0.0, 0.0, String::new(), String::new())];
        assert_eq!(next_empty(&pts, Some(1)), Some(1));
        assert_eq!(next_empty(&pts, None), Some(1));
    }

    #[test]
    fn the_error_matrix_saves_as_csv() {
        let v = json!({"classes": ["Water", "Rice"], "matrix": [[8, 2], [1, 9]], "overall": 0.85, "kappa": 0.7});
        let csv = accuracy_csv(&v);
        assert!(csv.starts_with("map \\ reference,Water,Rice,total\nWater,8,2,10\nRice,1,9,10\n"), "{csv}");
        assert!(csv.contains("overall,0.85"));
        let ch = change_csv(&json!({"matrix": [{"fromClass": "Forest", "toClass": "Rice, wet", "hectares": 12.345}]}));
        assert_eq!(ch, "from,to,hectares,changed\nForest,\"Rice, wet\",12.35,1\n");
    }
}
