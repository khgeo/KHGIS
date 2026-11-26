//! LEE RS Lab's tools beside its steps: the magic wand for training areas, the spectral profile
//! tool, histogram and feature-space plots, the multi-season stack and the PDF report.

use egui::{Color32, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::RustGisApp;
use crate::rslab::{Dataset, Scene};

const WAND: &str = "rs_wand";
const PROFILE: &str = "rs_profile";

fn armed(app: &RustGisApp, purpose: &str) -> bool {
    app.workflow.pick.as_ref().is_some_and(|p| p.purpose == purpose)
}

fn arm(app: &mut RustGisApp, purpose: &'static str, on: bool) {
    app.workflow.pick = on.then(|| crate::analysispane::Pick { purpose, needed: 1, points: Vec::new() });
}

/// Lands map clicks for the wand and the profile, and the multi-season searches.
pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    let point = app
        .workflow
        .pick
        .as_mut()
        .filter(|p| (p.purpose == WAND || p.purpose == PROFILE) && !p.points.is_empty())
        .map(|p| (p.purpose, p.points.remove(0)));
    if let Some((purpose, c)) = point {
        if purpose == WAND {
            wand_at(app, c.x, c.y);
        } else {
            profile_at(app, c.x, c.y);
        }
        ctx.request_repaint();
    }
    seasons_poll(app, ctx);
}

// ------------------------------------------------------------------------------- magic wand

/// The magic wand's controls (3 Classify, training polygons).
pub fn wand_ui(app: &mut RustGisApp, ui: &mut Ui) {
    let on = armed(app, WAND);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add(egui::Button::selectable(on, tl!("Magic wand (click-to-grow)")))
            .on_hover_text(tl!("Click the map: the pixels like the one clicked, around it, become a training polygon of this class"))
            .clicked()
        {
            arm(app, WAND, !on);
            if !on && app.session.map().layer_by_name(&app.ui.rslab.training).is_none() {
                if app.session.map().layer_by_name("training").is_none() {
                    let _ = app.run("layer.template", json!({"template": "polygons", "name": "training"}));
                }
                app.ui.rslab.training = "training".into();
                app.ui.rslab.class_field = "name".into();
            }
            let _ = app.run("edit.enable", json!({"on": true}));
        }
        ui.label(tl!("Class"));
        ui.add(egui::TextEdit::singleline(&mut app.ui.rslab.wand_class).desired_width(90.0).hint_text(tl!("e.g. Rice")));
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Similarity tolerance"));
        ui.add(egui::Slider::new(&mut app.ui.rslab.wand_tolerance, 0.2..=5.0).step_by(0.1));
    });
    if on {
        ui.label(egui::RichText::new(tl!("Click the map to grow a training area (click the button again to stop).")).weak());
    }
}

fn wand_at(app: &mut RustGisApp, lon: f64, lat: f64) {
    let st = &app.ui.rslab;
    let (image, training, field, class, tol) =
        (st.image.clone(), st.training.clone(), st.class_field.clone(), st.wand_class.trim().to_string(), st.wand_tolerance);
    if image.is_empty() {
        return crate::rslab::say(app, false, tl!("Choose an image first.").to_string());
    }
    if class.is_empty() {
        return crate::rslab::say(app, false, tl!("Type the class first (e.g. Rice).").to_string());
    }
    let p = json!({"layer": image, "lon": lon, "lat": lat, "tolerance": tol, "addTo": training, "class": class, "classField": if field.is_empty() { "name".to_string() } else { field }});
    match app.run("raster.magicWand", p) {
        Ok(v) => {
            let text = crate::i18n::fmt(
                "{class}: {n} pixels ({ha} ha) added.",
                &[("class", &class), ("n", &v["pixels"].to_string()), ("ha", &format!("{:.2}", v["hectares"].as_f64().unwrap_or(0.0)))],
            );
            crate::rslab::say(app, true, text);
            app.map.invalidate();
        }
        Err(e) => crate::rslab::say(app, false, e),
    }
}

// ----------------------------------------------------------------------------- the image's data

fn raster(app: &RustGisApp, layer: &str) -> Option<std::sync::Arc<rustgis_engine::data::raster::Raster>> {
    app.session.map().layer_by_name(layer).and_then(|l| l.raster.as_ref()?.data.clone())
}

fn valid(r: &rustgis_engine::data::raster::Raster, v: f32) -> bool {
    r.is_valid(v) && v.is_finite() && v > -1e30
}

// --------------------------------------------------------------------------- spectral profile

fn profile_at(app: &mut RustGisApp, lon: f64, lat: f64) {
    let image = app.ui.rslab.image.clone();
    let Some(r) = raster(app, &image) else { return };
    let Some((c, row)) = r.pixel_at(rustgis_engine::geom::Coord::new(lon, lat)) else {
        return crate::rslab::say(app, false, tl!("Outside the image or no data there.").to_string());
    };
    let values: Vec<(String, f64)> = r
        .bands
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let name = if b.name.is_empty() { format!("b{}", i + 1) } else { b.name.clone() };
            r.value(i, c, row).filter(|v| valid(&r, *v)).map(|v| (name, f64::from(v)))
        })
        .collect();
    if values.is_empty() {
        return crate::rslab::say(app, false, tl!("Outside the image or no data there.").to_string());
    }
    let profiles = &mut app.ui.rslab.profiles;
    profiles.push((format!("{lat:.4}, {lon:.4}"), values));
    if profiles.len() > 6 {
        profiles.remove(0);
    }
}

pub fn profile_ui(app: &mut RustGisApp, ui: &mut Ui) {
    let on = armed(app, PROFILE);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::selectable(on, tl!("Spectral profile tool")))
            .on_hover_text(tl!("Click the map: each click adds the pixel's values in every band"))
            .clicked()
        {
            arm(app, PROFILE, !on);
        }
        if !app.ui.rslab.profiles.is_empty() && ui.small_button(tl!("Clear")).clicked() {
            app.ui.rslab.profiles.clear();
        }
    });
    let profiles = app.ui.rslab.profiles.clone();
    if profiles.is_empty() {
        return;
    }
    let bands: Vec<String> =
        profiles.iter().max_by_key(|p| p.1.len()).map(|p| p.1.iter().map(|b| b.0.clone()).collect()).unwrap_or_default();
    let (lo, hi) =
        profiles.iter().flat_map(|p| p.1.iter().map(|b| b.1)).fold((f64::INFINITY, f64::NEG_INFINITY), |a, v| (a.0.min(v), a.1.max(v)));
    let (lo, hi) = if hi > lo { (lo.min(0.0), hi) } else { (lo - 1.0, hi + 1.0) };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().clamp(240.0, 420.0), 170.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let plot = egui::Rect::from_min_max(rect.min + vec2(34.0, 6.0), rect.max - vec2(6.0, 30.0));
    let t = crate::theme::Tokens::get(ui.ctx());
    p.rect_stroke(plot, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let x = |i: usize| plot.left() + plot.width() * (i as f32 + 0.5) / bands.len().max(1) as f32;
    let y = |v: f64| plot.bottom() - plot.height() * ((v - lo) / (hi - lo)) as f32;
    let font = egui::FontId::proportional(10.0);
    for (i, b) in bands.iter().enumerate() {
        p.text(pos2(x(i), plot.bottom() + 3.0), egui::Align2::CENTER_TOP, b, font.clone(), t.text_dim);
    }
    for v in [lo, (lo + hi) / 2.0, hi] {
        p.text(pos2(plot.left() - 3.0, y(v)), egui::Align2::RIGHT_CENTER, format!("{v:.2}"), font.clone(), t.text_dim);
    }
    for (k, (label, vals)) in profiles.iter().enumerate() {
        let colour = class_color(k);
        let pts: Vec<egui::Pos2> = vals.iter().filter_map(|(b, v)| bands.iter().position(|n| n == b).map(|i| pos2(x(i), y(*v)))).collect();
        p.add(egui::Shape::line(pts.clone(), egui::Stroke::new(2.0, colour)));
        for q in pts {
            p.circle_filled(q, 2.5, colour);
        }
        p.text(pos2(plot.left() + 4.0, plot.top() + 2.0 + k as f32 * 12.0), egui::Align2::LEFT_TOP, label, font.clone(), colour);
    }
}

fn class_color(i: usize) -> Color32 {
    const C: [(u8, u8, u8); 6] =
        [(0x2E, 0x86, 0xC1), (0x27, 0xAE, 0x60), (0xE6, 0x7E, 0x22), (0xC0, 0x39, 0x2B), (0x8E, 0x44, 0xAD), (0x16, 0xA0, 0x85)];
    let (r, g, b) = C[i % C.len()];
    Color32::from_rgb(r, g, b)
}

// ------------------------------------------------------------------------ histogram & scatter

fn band_combo(ui: &mut Ui, id: &str, label: &str, value: &mut usize, names: &[String]) {
    if *value >= names.len() {
        *value = 0;
    }
    ui.label(tl!(label));
    egui::ComboBox::from_id_salt(id).selected_text(names.get(*value).cloned().unwrap_or_default()).width(90.0).show_ui(ui, |ui| {
        for (i, n) in names.iter().enumerate() {
            ui.selectable_value(value, i, n);
        }
    });
}

/// The histogram and the feature space plot of the image (2 Enhance).
pub fn plots_ui(app: &mut RustGisApp, ui: &mut Ui) {
    let image = app.ui.rslab.image.clone();
    let Some(r) = raster(app, &image) else { return };
    let names = crate::rslab::layer_band_names(app, &image);
    let t = crate::theme::Tokens::get(ui.ctx());
    let n = r.width as usize * r.height as usize;
    let step = (n / 150_000).max(1);
    ui.horizontal(|ui| band_combo(ui, "rslab_hist", "Histogram", &mut app.ui.rslab.hist_band, &names));
    if let Some(b) = r.bands.get(app.ui.rslab.hist_band) {
        let v: Vec<f32> = b.data.iter().step_by(step).copied().filter(|x| valid(&r, *x)).collect();
        if !v.is_empty() {
            let mut sorted = v.clone();
            sorted.sort_by(f32::total_cmp);
            let (lo, hi) = (sorted[sorted.len() / 200], sorted[sorted.len() - 1 - sorted.len() / 200]);
            let (lo, hi) = if hi > lo { (lo, hi) } else { (lo - 1.0, hi + 1.0) };
            let mut bins = [0usize; 64];
            for x in &v {
                bins[(((x - lo) / (hi - lo)) * 63.0).round().clamp(0.0, 63.0) as usize] += 1;
            }
            let most = *bins.iter().max().unwrap_or(&1) as f32;
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().clamp(240.0, 420.0), 110.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            let plot = egui::Rect::from_min_max(rect.min + vec2(4.0, 4.0), rect.max - vec2(4.0, 16.0));
            let w = plot.width() / 64.0;
            for (i, c) in bins.iter().enumerate() {
                let h = plot.height() * (*c as f32 / most.max(1.0));
                p.rect_filled(
                    egui::Rect::from_min_max(
                        pos2(plot.left() + i as f32 * w, plot.bottom() - h),
                        pos2(plot.left() + (i + 1) as f32 * w - 1.0, plot.bottom()),
                    ),
                    0.0,
                    t.accent,
                );
            }
            let font = egui::FontId::proportional(10.0);
            p.text(pos2(plot.left(), plot.bottom() + 2.0), egui::Align2::LEFT_TOP, format!("{lo:.3}"), font.clone(), t.text_dim);
            p.text(pos2(plot.right(), plot.bottom() + 2.0), egui::Align2::RIGHT_TOP, format!("{hi:.3}"), font, t.text_dim);
        }
    }
    if names.len() < 2 {
        return;
    }
    ui.horizontal(|ui| {
        band_combo(ui, "rslab_fsx", "Feature space X / Y", &mut app.ui.rslab.fs_x, &names);
        band_combo(ui, "rslab_fsy", "", &mut app.ui.rslab.fs_y, &names);
    });
    let (bx, by) = (r.bands.get(app.ui.rslab.fs_x), r.bands.get(app.ui.rslab.fs_y));
    let (Some(bx), Some(by)) = (bx, by) else { return };
    let pairs: Vec<(f32, f32)> = bx
        .data
        .iter()
        .zip(&by.data)
        .step_by(step.max(n / 60_000).max(1))
        .filter(|(a, b)| valid(&r, **a) && valid(&r, **b))
        .map(|(a, b)| (*a, *b))
        .collect();
    if pairs.is_empty() {
        return;
    }
    let range = |f: &dyn Fn(&(f32, f32)) -> f32| {
        let mut v: Vec<f32> = pairs.iter().map(f).collect();
        v.sort_by(f32::total_cmp);
        let (lo, hi) = (v[v.len() / 200], v[v.len() - 1 - v.len() / 200]);
        if hi > lo { (lo, hi) } else { (lo - 1.0, hi + 1.0) }
    };
    let ((x0, x1), (y0, y1)) = (range(&|p| p.0), range(&|p| p.1));
    const N: usize = 90;
    let mut grid = vec![0u32; N * N];
    for (a, b) in &pairs {
        let i = (((a - x0) / (x1 - x0)) * (N - 1) as f32).round();
        let j = (((b - y0) / (y1 - y0)) * (N - 1) as f32).round();
        if (0.0..N as f32).contains(&i) && (0.0..N as f32).contains(&j) {
            grid[(N - 1 - j as usize) * N + i as usize] += 1;
        }
    }
    let most = grid.iter().copied().max().unwrap_or(1).max(1) as f32;
    let side = ui.available_width().clamp(200.0, 300.0);
    let (rect, _) = ui.allocate_exact_size(vec2(side, side + 14.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let plot = egui::Rect::from_min_size(rect.min, vec2(side, side));
    p.rect_stroke(plot, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let cell = side / N as f32;
    for (k, c) in grid.iter().enumerate() {
        if *c == 0 {
            continue;
        }
        let f = ((*c as f32).ln_1p() / most.ln_1p()).clamp(0.0, 1.0);
        let colour =
            Color32::from_rgb((30.0 + 220.0 * f) as u8, (60.0 + 150.0 * (1.0 - (f - 0.5).abs() * 2.0)) as u8, (200.0 * (1.0 - f)) as u8);
        let (i, j) = (k % N, k / N);
        p.rect_filled(
            egui::Rect::from_min_size(plot.min + vec2(i as f32 * cell, j as f32 * cell), vec2(cell + 0.5, cell + 0.5)),
            0.0,
            colour,
        );
    }
    let font = egui::FontId::proportional(10.0);
    p.text(
        pos2(plot.center().x, plot.bottom() + 2.0),
        egui::Align2::CENTER_TOP,
        format!("{} →", names.get(app.ui.rslab.fs_x).cloned().unwrap_or_default()),
        font.clone(),
        t.text_dim,
    );
    p.text(
        plot.min + vec2(3.0, 3.0),
        egui::Align2::LEFT_TOP,
        format!("↑ {}", names.get(app.ui.rslab.fs_y).cloned().unwrap_or_default()),
        font,
        t.text_dim,
    );
}

// --------------------------------------------------------------------------- multi-season stack

/// Seasons of Cambodia's year: (name, start month-day, end month-day, starts the year before).
const SEASONS: [(&str, &str, &str, bool); 3] = [
    ("Early dry (Dec–Feb)", "12-01", "02-28", true),
    ("Late dry (Mar–May)", "03-01", "05-31", false),
    ("Wet (Jun–Oct)", "06-01", "10-31", false),
];

pub fn seasons_ui(app: &mut RustGisApp, ui: &mut Ui) {
    let ds = app.ui.rslab.dataset;
    if !matches!(ds, Dataset::Sentinel2 | Dataset::Landsat89 | Dataset::Landsat47) {
        return;
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Multi-season stack")).strong());
    ui.label(
        egui::RichText::new(tl!(
            "A median composite of each season of one year, and its NDVI, stacked: crops and forests differ by season."
        ))
        .weak(),
    );
    let st = &mut app.ui.rslab;
    ui.horizontal_wrapped(|ui| {
        ui.label(tl!("Year"));
        ui.add(egui::DragValue::new(&mut st.season_year).range(1985.0..=2030.0).speed(0.05).fixed_decimals(0));
        for (k, (name, ..)) in SEASONS.iter().enumerate() {
            ui.checkbox(&mut st.seasons[k], tl!(name));
        }
    });
    let busy = !st.season_search.is_empty() || crate::rslab::busy(app);
    if ui.add_enabled(!busy, egui::Button::new(tl!("Build multi-season stack"))).clicked() {
        seasons_start(app);
    }
}

fn seasons_start(app: &mut RustGisApp) {
    let bbox = match crate::rslab::area_bbox(app) {
        Ok(b) => b,
        Err(e) => return crate::rslab::say(app, false, e),
    };
    let st = &mut app.ui.rslab;
    let y = st.season_year.round() as i64;
    st.season_found.clear();
    st.season_search.clear();
    for (k, (_, a, b, before)) in SEASONS.iter().enumerate() {
        if !st.seasons[k] {
            continue;
        }
        let start = format!("{}-{a}", if *before { y - 1 } else { y });
        let end = format!("{y}-{b}");
        let url = crate::rslab::search_url(st.dataset, bbox, &start, &end, st.cloud.max(40.0));
        st.season_search.push((k, url.clone(), crate::rslab::fetch(&url)));
    }
}

fn seasons_poll(app: &mut RustGisApp, ctx: &egui::Context) {
    if app.ui.rslab.season_search.is_empty() {
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(300));
    let ds = app.ui.rslab.dataset;
    let mut i = 0;
    while i < app.ui.rslab.season_search.len() {
        let landed = app.ui.rslab.season_search[i].2.lock().unwrap_or_else(|e| e.into_inner()).take();
        match landed {
            None => i += 1,
            Some(r) => {
                let (k, ..) = app.ui.rslab.season_search.remove(i);
                match r.and_then(|b| crate::rslab::scenes(ds, &b)) {
                    Ok(s) => app.ui.rslab.season_found.push((k, s)),
                    Err(e) => crate::rslab::say(app, false, e),
                }
            }
        }
    }
    if app.ui.rslab.season_search.is_empty() {
        seasons_build(app);
    }
}

fn seasons_build(app: &mut RustGisApp) {
    let found = std::mem::take(&mut app.ui.rslab.season_found);
    let ds = app.ui.rslab.dataset;
    let y = app.ui.rslab.season_year.round() as i64;
    let per = app.ui.rslab.composite.round().clamp(3.0, 8.0) as usize;
    let cell = app.ui.rslab.cell;
    let mut steps = Vec::new();
    let mut layers: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut temporary = Vec::new();
    let mut found = found;
    found.sort_by_key(|f| f.0);
    for (k, scenes) in &found {
        let short = ["dry1", "dry2", "wet"][*k];
        if scenes.is_empty() {
            crate::rslab::say(app, false, crate::i18n::fmt("No clear images in the {s} season: left out.", &[("s", short)]));
            continue;
        }
        let chosen: Vec<Scene> = scenes.iter().take(per).cloned().collect();
        let (image, mut s) = match crate::rslab::composite_steps(app, ds, &chosen, cell, true) {
            Ok(x) => x,
            Err(e) => return crate::rslab::say(app, false, e),
        };
        steps.append(&mut s);
        let ndvi = format!("{image}_ndvi");
        steps.push(crate::rslab::makes(
            "raster.function",
            json!({"layer": image, "function": "bandMath", "expression": "(b4 - b3) / (b4 + b3)", "name": ndvi}),
            &ndvi,
        ));
        names.extend(crate::rslab::BANDS.iter().map(|b| format!("{short}_{b}")));
        names.push(format!("{short}_NDVI"));
        layers.push(image.clone());
        layers.push(ndvi.clone());
        temporary.push(image);
        temporary.push(ndvi);
    }
    let Some((first, rest)) = layers.split_first() else { return };
    let out = crate::rslab::fresh(app, &format!("{}_seasons_{y}", ds.short()));
    steps.push(crate::rslab::makes(
        "raster.process",
        json!({"layer": first, "operation": "stack", "layers": rest, "bandNames": names, "name": out}),
        &out,
    ));
    steps.push(crate::rslab::step("layer.remove", json!({"layer": temporary})));
    steps.push(crate::rslab::step(
        "raster.bands",
        json!({"layer": out, "red": 7, "green": 14.min(names.len()), "blue": 21.min(names.len())}),
    ));
    steps.push(crate::rslab::step("raster.stretch", json!({"layer": out, "type": "percentClip", "low": 2, "high": 2})));
    let done = crate::i18n::fmt("{name} is ready: the seasons' bands and NDVI, ready to classify.", &[("name", &out)]);
    app.ui.rslab.image = out;
    crate::rslab::start_chain(app, tl!("Multi-season stack").to_string(), steps, done);
}

// ------------------------------------------------------------------------------------ report

/// A table as CSV text: a header and rows.
fn csv(header: &[&str], rows: &[Vec<String>]) -> String {
    let q = |s: &str| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = header.join(",");
    for r in rows {
        out.push('\n');
        out.push_str(&r.iter().map(|c| q(c)).collect::<Vec<_>>().join(","));
    }
    out
}

/// A table the layout's table and chart can read: a layer (its rows put at `at`, the layer hidden).
fn add_table(app: &mut RustGisApp, name: &str, text: String, at: (f64, f64)) -> Result<(), String> {
    if app.session.map().layer_by_name(name).is_some() {
        let _ = app.run("layer.remove", json!({"layer": name}));
    }
    let mut lines = text.lines();
    let mut with_xy = format!("{},lon,lat", lines.next().unwrap_or_default());
    for l in lines {
        with_xy.push_str(&format!("\n{l},{},{}", at.0, at.1));
    }
    app.run("layer.addDataText", json!({"fileName": format!("{name}.csv"), "text": with_xy, "name": name, "x": "lon", "y": "lat"}))?;
    app.run("layer.visibility", json!({"layer": name, "visible": false})).map(|_| ())
}

/// The PDF report: an A4 page with the map, its legend, the class areas (table and chart), the
/// accuracy and the change found, then saved as PDF.
pub fn report(app: &mut RustGisApp) {
    if let Err(e) = report_inner(app) {
        crate::rslab::say(app, false, e);
    }
}

fn report_inner(app: &mut RustGisApp) -> Result<(), String> {
    build_report(app)?;
    crate::rslab::say(app, true, tl!("Report page made: choose where to save the PDF.").to_string());
    app.run("ui.exportLayout", json!({}))?;
    Ok(())
}

/// The report's page, made the active layout.
fn build_report(app: &mut RustGisApp) -> Result<(), String> {
    let km = crate::khdata::khmer();
    let st = &app.ui.rslab;
    let (title, class_map, image) = (st.title.clone(), st.class_map.clone(), st.image.clone());
    let accuracy = st.results.iter().find(|r| r.0 == "accuracy").map(|r| r.1.clone());
    let change = st.results.iter().find(|r| r.0 == "crossTab").map(|r| r.1.clone());
    let place = crate::rslab::area(app).map(|a| a.place).unwrap_or_default();
    let tr = |en: &str| tl!(en).to_string();
    // The class areas, as a table and a chart.
    let mut table: Option<String> = None;
    if !class_map.is_empty() && app.session.map().layer_by_name(&class_map).is_some() {
        let v = app.run("raster.classAreas", json!({"layer": class_map}))?;
        let rows: Vec<Value> = v["classes"].as_array().cloned().unwrap_or_default();
        let total: f64 = rows.iter().filter_map(|c| c["hectares"].as_f64()).sum();
        let body: Vec<Vec<String>> = rows
            .iter()
            .map(|c| {
                let ha = c["hectares"].as_f64().unwrap_or(0.0);
                vec![
                    c["class"].as_str().unwrap_or("").to_string(),
                    format!("{ha:.1}"),
                    format!("{:.1}", if total > 0.0 { ha / total * 100.0 } else { 0.0 }),
                ]
            })
            .collect();
        let name = tr("Class areas");
        let at = app.session.map().layer_by_name(&class_map).and_then(|l| l.raster.as_ref()?.data.clone()).map_or((0.0, 0.0), |r| {
            let e = r.extent();
            ((e.min_x + e.max_x) / 2.0, (e.min_y + e.max_y) / 2.0)
        });
        add_table(app, &name, csv(&[&tr("Class"), &tr("Area (ha)"), "%"], &body), at)?;
        table = Some(name);
    }
    let _ = app.run("layer.select", json!({"layer": if class_map.is_empty() { image.clone() } else { class_map.clone() }}));
    let lname = if km { "របាយការណ៍ LEE RS Lab" } else { "LEE RS Lab report" };
    app.run("layout.new", json!({"name": lname, "page": "a4", "orientation": "portrait", "template": "blank"}))?;
    let add = |app: &mut RustGisApp, p: Value| app.run("layout.addElement", p);
    app.run("layout.addFrame", json!({"x": 15, "y": 30, "w": 180, "h": 118}))?;
    add(app, json!({"kind": "rectangle"}))?;
    add(app, json!({"kind": "text", "text": title, "size": 16, "bold": true, "x": 15, "y": 10, "w": 180, "h": 10}))?;
    let date = crate::stacui::days_ago(0);
    let sub = if km { format!("{place} · ធ្វើនៅ {date}") } else { format!("{place} · made {date}") };
    add(app, json!({"kind": "text", "text": sub, "size": 9, "x": 15, "y": 21, "w": 180, "h": 6}))?;
    add(app, json!({"kind": "northArrow", "x": 182, "y": 33, "w": 10, "h": 14}))?;
    add(app, json!({"kind": "scaleBar", "x": 18, "y": 140, "w": 60, "h": 7}))?;
    add(app, json!({"kind": "legend", "x": 15, "y": 152, "w": 85, "h": 70, "exclude": table.iter().collect::<Vec<_>>()}))?;
    if let Some(t) = &table {
        add(app, json!({"kind": "text", "text": tr("Class areas"), "size": 10, "bold": true, "x": 105, "y": 152, "w": 90, "h": 6}))?;
        add(app, json!({"kind": "table", "layer": t, "rows": 14, "x": 105, "y": 159, "w": 90, "h": 50}))?;
        let _ = add(
            app,
            json!({"kind": "chart", "layer": t, "field": tr("Area (ha)"), "label": tr("Class"), "x": 105, "y": 211, "w": 90, "h": 45}),
        );
    }
    // What the lab found: accuracy and change, in words.
    let mut lines: Vec<String> = Vec::new();
    if let Some(a) = &accuracy {
        let f = |k: &str| a[k].as_f64().unwrap_or(0.0);
        lines.push(crate::i18n::fmt(
            "Overall accuracy {oa}% · kappa {k} · area-weighted {w}% · {n} samples",
            &[
                ("oa", &format!("{:.1}", f("overall") * 100.0)),
                ("k", &format!("{:.3}", f("kappa"))),
                ("w", &format!("{:.1}", f("weightedOverall") * 100.0)),
                ("n", &a["samples"].to_string()),
            ],
        ));
    }
    if let Some(c) = &change {
        lines.push(crate::i18n::fmt("Changed area: {ha} ha", &[("ha", &format!("{:.1}", c["changedHectares"].as_f64().unwrap_or(0.0)))]));
        let mut rows: Vec<(String, f64)> = c["matrix"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["fromClass"] != r["toClass"])
            .filter_map(|r| Some((format!("{} → {}", r["fromClass"].as_str()?, r["toClass"].as_str()?), r["hectares"].as_f64()?)))
            .collect();
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        lines.extend(rows.into_iter().take(4).map(|(t, ha)| format!("  {t}: {ha:.1} ha")));
    }
    if !lines.is_empty() {
        add(app, json!({"kind": "text", "text": tr("Results"), "size": 10, "bold": true, "x": 15, "y": 224, "w": 85, "h": 6}))?;
        add(app, json!({"kind": "text", "text": lines.join("\n"), "size": 8, "x": 15, "y": 231, "w": 85, "h": 30}))?;
    }
    let credit = if km {
        "ទិន្នន័យ៖ Copernicus Sentinel (ESA), Landsat (USGS/NASA), ESA WorldCover, Esri, JRC តាមរយៈ Microsoft Planetary Computer · ព្រំដែនរដ្ឋបាល ២០២៤ · ផលិតដោយ ក្រុមស្រាវជ្រាវ និងអភិវឌ្ឍន៍ព័ត៌មានភូមិសាស្ត្រ សភូមិន្ទភ្នំពេញ · KHGIS LEE RS Lab"
    } else {
        "Data: Copernicus Sentinel (ESA), Landsat (USGS/NASA), ESA WorldCover, Esri, JRC via Microsoft Planetary Computer · administrative boundaries 2024 · Produced by the Geoinformatics and Research Lab, RUPP · KHGIS LEE RS Lab"
    };
    add(app, json!({"kind": "text", "text": credit, "size": 7, "x": 15, "y": 272, "w": 180, "h": 12}))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clicks on the map: the profile reads every band there, the wand grows a training polygon.
    #[test]
    fn map_clicks_read_a_profile_and_grow_a_training_area() {
        use rustgis_engine::{Project, Session};
        let dir = std::env::temp_dir().join(format!("khgis-wand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = RustGisApp::new(Session::new(Project::default()));
        let mut names = Vec::new();
        for (file, low, high) in [("r.asc", 10, 20), ("n.asc", 12, 80)] {
            let rows: String =
                (0..10).map(|_| (0..10).map(|c| if c < 5 { low } else { high }.to_string()).collect::<Vec<_>>().join(" ") + "\n").collect();
            let path = dir.join(file);
            std::fs::write(&path, format!("ncols 10\nnrows 10\nxllcorner 104\nyllcorner 11\ncellsize 0.001\n{rows}")).unwrap();
            let id = app.run("layer.addData", json!({"path": path})).unwrap()["layer"].as_u64().unwrap();
            names.push(app.session.map().layer(id as u32).unwrap().name.clone());
        }
        app.run("raster.process", json!({"layer": names[0], "operation": "stack", "layers": [names[1]], "name": "img"})).unwrap();
        app.ui.rslab.image = "img".into();
        profile_at(&mut app, 104.0075, 11.005);
        assert_eq!(app.ui.rslab.profiles.len(), 1);
        assert_eq!(app.ui.rslab.profiles[0].1.iter().map(|b| b.1).collect::<Vec<_>>(), vec![20.0, 80.0]);
        app.run("layer.template", json!({"template": "polygons", "name": "training"})).unwrap();
        app.ui.rslab.training = "training".into();
        app.ui.rslab.class_field = "name".into();
        app.ui.rslab.wand_class = "Rice".into();
        wand_at(&mut app, 104.0075, 11.005);
        let l = app.session.map().layer_by_name("training").unwrap();
        assert_eq!(l.dataset.features.len(), 1, "{:?}", app.ui.rslab.log);
    }

    /// The report page has the map, legend, class-area table and chart, and the results.
    #[test]
    fn the_report_page_has_the_map_the_table_and_the_results() {
        use rustgis_engine::{Project, Session};
        let dir = std::env::temp_dir().join(format!("khgis-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = RustGisApp::new(Session::new(Project::default()));
        let rows: String = (0..10).map(|_| (0..10).map(|c| if c < 5 { "1" } else { "2" }).collect::<Vec<_>>().join(" ") + "\n").collect();
        let path = dir.join("classes.asc");
        std::fs::write(&path, format!("ncols 10\nnrows 10\nxllcorner 104\nyllcorner 11\ncellsize 0.001\n{rows}")).unwrap();
        let id = app.run("layer.addData", json!({"path": path})).unwrap()["layer"].as_u64().unwrap();
        let name = app.session.map().layer(id as u32).unwrap().name.clone();
        app.run("raster.classNames", json!({"layer": name, "names": {"1": "Water", "2": "Rice"}})).unwrap();
        app.ui.rslab.class_map = name;
        app.ui.rslab.title = "Test map".into();
        app.ui.rslab.results.push(("accuracy", json!({"overall": 0.9, "kappa": 0.8, "weightedOverall": 0.88, "samples": 120})));
        build_report(&mut app).unwrap();
        let l = app.session.project.layouts.last().unwrap();
        let text = format!("{:?}", l.elements);
        for want in ["Map", "Legend", "Table", "Chart", "Test map", "Overall accuracy 90.0%"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        let table = app.session.map().layer_by_name("Class areas").unwrap();
        assert_eq!(table.dataset.features.len(), 2);
    }

    #[test]
    fn tables_quote_their_cells() {
        assert_eq!(super::csv(&["a", "b"], &[vec!["x,y".into(), "1".into()]]), "a,b\n\"x,y\",1");
    }
}
