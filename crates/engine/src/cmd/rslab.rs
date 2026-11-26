//! The remote sensing lab's raster commands (LEE RS Lab in KHGIS, see `rustgis_analysis::rslab`):
//! spatial filters, principal components, GLCM texture, the sieve, from → to change matrices,
//! class areas, accuracy assessment and sample points for it, and class names.

use rustgis_analysis::rasterops::{self, NODATA};
use rustgis_analysis::rslab;
use rustgis_data::raster::Raster;
use rustgis_data::{Geometry, GeometryType};
use serde_json::{Value, json};

use super::imagery::{bands_param, class_layer, name, raster, raster_name, raster_named};
use super::{ps, uint};
use crate::{CmdResult, CommandSpec as C, Session};

pub fn commands() -> Vec<C> {
    vec![
        C::new("raster.filter", "Spatial Filter", "Analyze › Imagery Tools", |s, p| super::workflows::made_or_ran(s, p, "raster.filter"))
            .inputs(one_input)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .params(r#"{"layer": raster, "filter": "mean"|"median"|"gaussian"|"sharpen"|"sobel"|"laplacian"|"emboss"|"majority"|"min"|"max", "size"?: 3..15 (odd; default 3), "bands"?: [1-based] (default all), "name"?: output, "ran"?: (internal)}"#),
        C::new("raster.pca", "Principal Components", "Analyze › Imagery Tools", |s, p| super::workflows::made_or_ran(s, p, "raster.pca"))
            .inputs(one_input)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .params(r#"{"layer": raster, "bands"?: [1-based] (default all), "components"?: n (default 3), "name"?: output, "ran"?: (internal)} → the components as bands, with each one's eigenvalue, share of the variance and loadings"#),
        C::new("raster.texture", "GLCM Texture", "Analyze › Imagery Tools", |s, p| super::workflows::made_or_ran(s, p, "raster.texture"))
            .inputs(one_input)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .params(r#"{"layer": raster, "band"?: 1-based (default 1), "window"?: 3..11 (odd; default 5), "measures"?: ["contrast"|"dissimilarity"|"homogeneity"|"entropy"|"asm"|"mean"|"variance"] (default contrast, homogeneity, entropy), "name"?: output, "ran"?: (internal)}"#),
        C::new("raster.sieve", "Sieve (Minimum Mapping Unit)", "Analyze › Image Classification", |s, p| {
            super::workflows::made_or_ran(s, p, "raster.sieve")
        })
        .inputs(one_input)
        .roles(&[])
        .effects(crate::policy::Effects::NONE)
        .params(r#"{"layer": class raster, "minPixels"?: n (regions smaller take their neighbours' class; default 8), "majority"?: 0|3|5|7 (a majority filter first; default 0), "name"?: output, "ran"?: (internal)}"#),
        C::new("raster.crossTab", "From–To Change", "Analyze › Change Detection", |s, p| super::workflows::made_or_ran(s, p, "raster.crossTab"))
            .inputs(cross_inputs)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .params(r#"{"from": class raster (date 1), "to": class raster (date 2, put on from's grid), "name"?: output, "ran"?: (internal)} → the change map (from × 1000 + to where they differ, 0 where they don't) and the from → to matrix in hectares"#),
        C::new("raster.classAreas", "Class Areas", "Analyze › Image Classification", class_areas)
            .inputs(one_input)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .view()
            .params(r#"{"layer": class raster, "band"?: 1-based} → each class's pixels and hectares"#),
        C::new("raster.accuracy", "Accuracy Assessment", "Analyze › Image Classification", accuracy)
            .inputs(accuracy_inputs)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .view()
            .params(r#"{"layer": class raster, "reference": point or polygon layer, "classField": its field of the true class (a class name or value)} → error matrix, overall accuracy, kappa, user's and producer's accuracy, and area-adjusted estimates with 95% confidence intervals (Olofsson et al. 2014)"#),
        C::new("raster.samplePoints", "Create Accuracy Points", "Analyze › Image Classification", sample_points)
            .inputs(one_input)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .params(r#"{"layer": class raster, "perClass"?: n (default 30), "name"?: output layer} → a stratified random point layer: map_class (the map's), ref_class (empty, to fill in)"#),
        C::new("raster.classNames", "Class Names", "Analyze › Image Classification", class_names)
            .inputs(one_input)
            .roles(&[])
            .effects(crate::policy::Effects::NONE)
            .params(r#"{"layer": class raster, "names": {"value": "name", …}}"#),
    ]
}

fn one_input(input: &super::InputContext<'_>, p: &Value) -> Result<super::Inputs, String> {
    super::Inputs::named(input.map(), [raster_name(p)?])
}

fn cross_inputs(input: &super::InputContext<'_>, p: &Value) -> Result<super::Inputs, String> {
    super::Inputs::named(input.map(), [ps(p, "from").ok_or("from is required")?, ps(p, "to").ok_or("to is required")?])
}

fn accuracy_inputs(input: &super::InputContext<'_>, p: &Value) -> Result<super::Inputs, String> {
    super::Inputs::named(input.map(), [raster_name(p)?, ps(p, "reference").ok_or("reference is required")?])
}

/// All bands, or those asked for.
fn all_bands(p: &Value, r: &Raster) -> Result<Vec<usize>, String> {
    if p.get("bands").is_some_and(|v| !v.is_null()) { bands_param(p, r) } else { Ok((0..r.band_count()).collect()) }
}

fn one_band(p: &Value, r: &Raster) -> Result<usize, String> {
    match p.get("band") {
        None | Some(Value::Null) => Ok(0),
        Some(_) => uint(p, "band")
            .map(|b| b as usize)
            .filter(|b| *b >= 1 && *b <= r.band_count())
            .map(|b| b - 1)
            .ok_or_else(|| format!("{} has bands 1 to {}", r.name, r.band_count())),
    }
}

/// The name of a class value of band `b`: its class name when the raster has one, else the value.
pub(crate) fn class_label(r: &Raster, b: usize, v: f32) -> String {
    r.bands.get(b).and_then(|band| band.class_name(f64::from(v))).map(str::to_string).unwrap_or_else(|| format!("{v}"))
}

pub(crate) fn filter_work(s: &Session, p: &Value) -> Result<super::workflows::Work, String> {
    let r = raster(s, p)?;
    r.require_coordinates()?;
    let kind = rslab::Filter::parse(ps(p, "filter").ok_or("filter is required")?)?;
    let size = match p.get("size") {
        None | Some(Value::Null) => 3,
        Some(_) => uint(p, "size").filter(|n| (3..=15).contains(n) && n % 2 == 1).ok_or("size is an odd number, 3 to 15")? as usize,
    };
    let bands = all_bands(p, &r)?;
    let out = name(p, format!("{}_{}", r.name, ps(p, "filter").unwrap_or("filter")));
    Ok(Box::new(move || {
        let mut made = Vec::with_capacity(bands.len());
        for b in &bands {
            made.push(rslab::filter(&r, *b, kind, size)?);
        }
        let mut res = rasterops::derive(&r, &out, made)?;
        for (band, b) in res.bands.iter_mut().zip(&bands) {
            if let Some(src) = r.bands.get(*b) {
                band.name.clone_from(&src.name);
                band.classes.clone_from(&src.classes);
            }
        }
        if kind == rslab::Filter::Majority {
            let layer = class_layer(res)?;
            return Ok(super::workflows::made_layer(out, layer, None, None));
        }
        super::workflows::made(res, None)
    }))
}

pub(crate) fn pca_work(s: &Session, p: &Value) -> Result<super::workflows::Work, String> {
    let r = raster(s, p)?;
    r.require_coordinates()?;
    let bands = all_bands(p, &r)?;
    let k = match p.get("components") {
        None | Some(Value::Null) => 3.min(bands.len()),
        Some(_) => uint(p, "components")
            .filter(|n| *n >= 1 && (*n as usize) <= bands.len())
            .ok_or("components is 1 to the number of bands")? as usize,
    };
    let out = name(p, format!("{}_pca", r.name));
    Ok(Box::new(move || {
        let pc = rslab::pca(&r, &bands, k)?;
        let mut res = rasterops::derive(&r, &out, pc.components)?;
        for (i, band) in res.bands.iter_mut().enumerate() {
            band.name = format!("PC{}", i + 1);
        }
        let extra = json!({
            "eigenvalues": pc.eigenvalues,
            "explained": pc.explained,
            "loadings": pc.loadings,
            "inputBands": bands.iter().map(|b| r.bands.get(*b).map(|x| x.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Band {}", b + 1))).collect::<Vec<_>>(),
        });
        super::workflows::made(res, Some(extra))
    }))
}

pub(crate) fn texture_work(s: &Session, p: &Value) -> Result<super::workflows::Work, String> {
    let r = raster(s, p)?;
    r.require_coordinates()?;
    let b = one_band(p, &r)?;
    let window = match p.get("window") {
        None | Some(Value::Null) => 5,
        Some(_) => uint(p, "window").filter(|n| (3..=11).contains(n) && n % 2 == 1).ok_or("window is an odd number, 3 to 11")? as usize,
    };
    let measures: Vec<rslab::Texture> = match p.get("measures").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a.iter().map(|v| rslab::Texture::parse(v.as_str().unwrap_or(""))).collect::<Result<_, _>>()?,
        _ => vec![rslab::Texture::Contrast, rslab::Texture::Homogeneity, rslab::Texture::Entropy],
    };
    let source = r.bands.get(b).map(|x| x.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("b{}", b + 1));
    let out = name(p, format!("{}_texture", r.name));
    Ok(Box::new(move || {
        let made = rslab::texture(&r, b, window, &measures)?;
        let mut res = rasterops::derive(&r, &out, made)?;
        for (band, m) in res.bands.iter_mut().zip(&measures) {
            band.name = format!("{source}_{}", m.name());
        }
        super::workflows::made(res, None)
    }))
}

pub(crate) fn sieve_work(s: &Session, p: &Value) -> Result<super::workflows::Work, String> {
    let r = raster(s, p)?;
    r.require_coordinates()?;
    let min = match p.get("minPixels") {
        None | Some(Value::Null) => 8,
        Some(_) => uint(p, "minPixels").filter(|n| (1..=1_000_000).contains(n)).ok_or("minPixels is 1 or more")? as usize,
    };
    let majority = match p.get("majority") {
        None | Some(Value::Null) => 0,
        Some(_) => uint(p, "majority").filter(|n| [0, 3, 5, 7].contains(n)).ok_or("majority is 0, 3, 5 or 7")? as usize,
    };
    let out = name(p, format!("{}_clean", r.name));
    Ok(Box::new(move || {
        let mut work = r.clone();
        if majority > 0 {
            let smoothed = rslab::filter(&r, 0, rslab::Filter::Majority, majority)?;
            let mut m = (*r).clone();
            if let Some(b) = m.bands.get_mut(0) {
                b.data = smoothed;
            }
            work = std::sync::Arc::new(m);
        }
        let (cleaned, changed) = rslab::sieve(&work, 0, min)?;
        let mut res = rasterops::derive(&r, &out, vec![cleaned])?;
        if let (Some(band), Some(src)) = (res.bands.get_mut(0), r.bands.first()) {
            band.classes.clone_from(&src.classes);
        }
        let layer = class_layer(res)?;
        Ok(super::workflows::made_layer(out, layer, Some(json!({"changed": changed, "minPixels": min, "majority": majority})), None))
    }))
}

pub(crate) fn cross_work(s: &Session, p: &Value) -> Result<super::workflows::Work, String> {
    let from = raster_named(s, ps(p, "from").ok_or("from is required")?)?;
    let to = raster_named(s, ps(p, "to").ok_or("to is required")?)?;
    from.require_coordinates()?;
    to.require_coordinates()?;
    let out = name(p, format!("{}_to_{}", from.name, to.name));
    Ok(Box::new(move || {
        let a = from.bands.first().map(|b| b.data.clone()).ok_or("from has no band")?;
        let a: Vec<f32> = a.into_iter().map(|v| if from.is_valid(v) { v } else { NODATA }).collect();
        // Date 2 on date 1's grid (each cell the class under its centre).
        let mut b = rasterops::resample_to(&to, 0, &from)?;
        // Maps numbered differently (a land cover map and a classification trained on it) are
        // compared by class name: date 2's values become date 1's where their names match.
        let from_values: Vec<f32> = rslab::counts(&from, 0)?.into_iter().map(|c| c.0).collect();
        let to_values: Vec<f32> = rslab::counts(&to, 0)?.into_iter().map(|c| c.0).collect();
        let named = |r: &Raster, v: f32| r.bands.first().and_then(|band| band.class_name(f64::from(v))).map(str::to_lowercase);
        let renumber: Vec<(f32, f32)> = to_values
            .iter()
            .filter_map(|t| {
                let name = named(&to, *t)?;
                from_values.iter().find(|f| named(&from, **f).as_deref() == Some(name.as_str())).map(|f| (*t, *f))
            })
            .collect();
        if !renumber.is_empty() {
            for v in &mut b {
                if let Some((_, f)) = renumber.iter().find(|(t, _)| *t == *v) {
                    *v = *f;
                }
            }
        }
        if rustgis_analysis::control::stop_requested() {
            return Err(rustgis_analysis::control::CANCELLED.to_string());
        }
        let (matrix, map) = rslab::cross_tab(&a, &b)?;
        let ha = rslab::pixel_hectares(&from);
        let rows: Vec<Value> = matrix
            .iter()
            .map(|((x, y), n)| {
                json!({"from": x, "to": y, "fromClass": class_label(&from, 0, *x), "toClass": class_label(if renumber.is_empty() { &to } else { &from }, 0, *y), "pixels": n, "hectares": *n as f64 * ha})
            })
            .collect();
        let changed: f64 = matrix.iter().filter(|((x, y), _)| x != y).map(|(_, n)| *n as f64 * ha).sum();
        let mut res = rasterops::derive(&from, &out, vec![map])?;
        if let Some(band) = res.bands.get_mut(0) {
            // Names for the change codes, so the legend reads "from → to".
            let mut names = Vec::new();
            for ((x, y), _) in matrix.iter().filter(|((x, y), _)| x != y) {
                let code = (*x * 1000.0 + *y) as usize;
                if code < 200_000 {
                    if names.len() <= code {
                        names.resize(code + 1, String::new());
                    }
                    names[code] =
                        format!("{} → {}", class_label(&from, 0, *x), class_label(if renumber.is_empty() { &to } else { &from }, 0, *y));
                }
            }
            if !names.is_empty() {
                names[0] = "No change".into();
            }
            band.classes = names;
        }
        let layer = class_layer(res)?;
        Ok(super::workflows::made_layer(out, layer, Some(json!({"matrix": rows, "changedHectares": changed, "pixelHectares": ha})), None))
    }))
}

fn class_areas(s: &mut Session, p: &Value) -> CmdResult {
    let r = raster(s, p)?;
    let b = one_band(p, &r)?;
    let ha = rslab::pixel_hectares(&r);
    let rows: Vec<Value> = rslab::counts(&r, b)?
        .into_iter()
        .map(|(v, n)| json!({"value": v, "class": class_label(&r, b, v), "pixels": n, "hectares": n as f64 * ha}))
        .collect();
    Ok(json!({"classes": rows, "pixelHectares": ha}))
}

/// The reference's class (its field's text) at each of its points, or the pixels under its
/// polygons, paired with the map's.
fn accuracy(s: &mut Session, p: &Value) -> CmdResult {
    let r = raster(s, p)?;
    r.require_coordinates()?;
    let reference = ps(p, "reference").ok_or("reference is required")?;
    let l =
        s.map().layer_by_name(reference).filter(|l| !l.is_raster()).ok_or_else(|| format!("no point or polygon layer '{reference}'"))?;
    let field = ps(p, "classField").ok_or("classField is required")?;
    let fi = l.dataset.field_index(field).ok_or_else(|| format!("{reference} has no field '{field}'"))?;
    let counts = rslab::counts(&r, 0)?;
    let ha = rslab::pixel_hectares(&r);
    // The map's classes, by name; reference classes the map doesn't have are added after them.
    let mut classes: Vec<String> = counts.iter().filter(|(v, _)| *v != 0.0).map(|(v, _)| class_label(&r, 0, *v)).collect();
    let mut mapped: Vec<f64> = counts.iter().filter(|(v, _)| *v != 0.0).map(|(_, n)| *n as f64 * ha).collect();
    let index_of = |classes: &mut Vec<String>, mapped: &mut Vec<f64>, name: &str| -> usize {
        let key = name.trim();
        match classes
            .iter()
            .position(|c| c.eq_ignore_ascii_case(key) || c.parse::<f64>().ok().zip(key.parse::<f64>().ok()).is_some_and(|(a, b)| a == b))
        {
            Some(i) => i,
            None => {
                classes.push(key.to_string());
                mapped.push(0.0);
                classes.len() - 1
            }
        }
    };
    let mut pairs = Vec::new();
    let (mut skipped, mut unlabelled) = (0usize, 0usize);
    for (row, ft) in l.dataset.features.iter().enumerate() {
        let truth = l.dataset.value(row, fi).to_string();
        if truth.trim().is_empty() || truth == "null" {
            unlabelled += 1;
            continue;
        }
        let t = index_of(&mut classes, &mut mapped, &truth);
        let mut cells: Vec<(i64, i64)> = Vec::new();
        match &ft.geometry {
            Some(Geometry::Point(c)) => cells.extend(r.pixel_at(*c)),
            Some(Geometry::MultiPoint(cs)) => cells.extend(cs.iter().filter_map(|c| r.pixel_at(*c))),
            Some(Geometry::Polygon(polys)) => {
                for rings in polys {
                    if let Some((cols, rows, mask)) = rasterops::polygon_mask(&r, std::slice::from_ref(rings))? {
                        let w = (cols.end - cols.start) as usize;
                        for (ri, rr) in rows.clone().enumerate() {
                            for ci in 0..w {
                                if mask.get(ri * w + ci).copied().unwrap_or(false) {
                                    cells.push((i64::from(cols.start) + ci as i64, i64::from(rr)));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if cells.is_empty() {
            skipped += 1;
        }
        for (c, rr) in cells {
            match r.value(0, c, rr).filter(|v| *v != NODATA && *v != 0.0) {
                Some(v) => {
                    let m = index_of(&mut classes, &mut mapped, &class_label(&r, 0, v));
                    pairs.push((m, t));
                }
                None => skipped += 1,
            }
        }
    }
    if pairs.is_empty() {
        return Err(format!("no reference sample of {reference} falls on a classified pixel of {} (fill in {field} first)", r.name));
    }
    if skipped > 0 {
        s.notify(format!("Accuracy: {skipped} reference sample(s) outside the map or on no class, left out"), false);
    }
    if unlabelled > 0 {
        s.notify(format!("Accuracy: {unlabelled} reference feature(s) with no {field}, left out"), false);
    }
    let a = rslab::accuracy(&classes, &pairs, &mapped);
    Ok(json!({
        "classes": a.classes, "matrix": a.matrix, "samples": pairs.len(),
        "overall": a.overall, "kappa": a.kappa, "users": a.users, "producers": a.producers,
        "weightedOverall": a.weighted_overall, "weightedProducers": a.weighted_producers,
        "mappedHectares": a.mapped_ha, "estimatedHectares": a.estimated_ha, "ci95Hectares": a.ci95_ha,
        "skipped": skipped, "unlabelled": unlabelled,
    }))
}

fn sample_points(s: &mut Session, p: &Value) -> CmdResult {
    let r = raster(s, p)?;
    r.require_coordinates()?;
    let per = match p.get("perClass") {
        None | Some(Value::Null) => 30,
        Some(_) => uint(p, "perClass").filter(|n| (1..=1000).contains(n)).ok_or("perClass is 1 to 1000")? as usize,
    };
    let pts = rslab::sample_points(&r, 0, per, 0x5A3D)?;
    let features: Vec<Value> = pts
        .iter()
        .filter(|(_, v)| *v != 0.0)
        .enumerate()
        .map(|(i, (c, v))| {
            json!({"type": "Feature", "properties": {"id": i + 1, "map_class": class_label(&r, 0, *v), "map_value": v, "ref_class": ""},
                "geometry": {"type": "Point", "coordinates": [c.x, c.y]}})
        })
        .collect();
    if features.is_empty() {
        return Err(format!("{} has no classified pixel", r.name));
    }
    let n = features.len();
    let layer = name(p, format!("{}_accuracy_points", r.name));
    let text = json!({"type": "FeatureCollection", "features": features}).to_string();
    let mut out = s.run("layer.addDataText", &json!({"fileName": format!("{layer}.geojson"), "text": text, "name": layer}))?;
    out["points"] = json!(n);
    Ok(out)
}

fn class_names(s: &mut Session, p: &Value) -> CmdResult {
    let r = raster(s, p)?;
    let names = p.get("names").and_then(Value::as_object).ok_or("names is {\"value\": \"name\", …}")?;
    let mut list: Vec<String> = r.bands.first().map(|b| b.classes.clone()).unwrap_or_default();
    for (k, v) in names {
        let value = k
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| *x >= 0.0 && x.fract() == 0.0 && *x < 100_000.0)
            .ok_or_else(|| format!("'{k}' isn't a class value (a whole number)"))? as usize;
        let text = v.as_str().ok_or("class names are text")?.chars().take(80).collect::<String>();
        if list.len() <= value {
            list.resize(value + 1, String::new());
        }
        list[value] = text;
    }
    let mut new = (*r).clone();
    if let Some(b) = new.bands.get_mut(0) {
        b.classes = list;
    }
    let layer = raster_name(p)?.to_string();
    super::imagery::replace(s, &layer, new)
}

/// Training pixels from a class raster (`trainingRaster`), used by `raster.classifySupervised`.
pub(crate) fn raster_samples(r: &Raster, bands: &[usize], labels: &Raster, per_class: usize) -> Result<rslab::Samples, String> {
    let on_grid = rasterops::resample_to(labels, 0, r)?;
    rslab::samples_from_labels(r, bands, &on_grid, per_class, 10, |v| class_label(labels, 0, v))
}

/// The training polygons (lon/lat rings) and their classes, for `raster.classifySupervised`.
pub(crate) fn training_polygons(s: &Session, p: &Value) -> Result<Vec<(String, Vec<Vec<rustgis_geom::Coord>>)>, String> {
    let tl = super::imagery::training_name(p)?;
    let l = s
        .map()
        .layer_by_name(tl)
        .filter(|l| l.geometry_type() == GeometryType::Polygon)
        .ok_or_else(|| format!("no polygon layer '{tl}'"))?;
    let fi = l.dataset.field_index(ps(p, "classField").ok_or("classField is required")?).ok_or("no such class field")?;
    let mut polygons = Vec::new();
    for (row, ft) in l.dataset.features.iter().enumerate() {
        let Some(Geometry::Polygon(polys)) = &ft.geometry else { continue };
        let key = l.dataset.value(row, fi).to_string();
        polygons.extend(polys.iter().map(|rings| (key.clone(), rings.clone())));
    }
    Ok(polygons)
}
