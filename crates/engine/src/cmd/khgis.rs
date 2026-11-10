//! KHGIS's own commands: the one-click map layout and the one-click workflows (recipes).

use serde_json::{Value, json};

use crate::{CmdResult, CommandSpec as C, Session};

pub fn commands() -> Vec<C> {
    vec![
        C::new("khgis.autoLayout", "Auto Map Layout", "Publish › Layout", auto_layout).roles(&[]).effects(crate::policy::Effects::NONE).params(
            r##"{"lang"?: "km"|"en" (the texts' language; default English), "title"?: text (default: the active layer and the map's area), "subtitle"?: text, "profile"?: as layout.publication (default "a4-portrait" for Cambodia-shaped areas, else "a4"), "style"?: as layout.publication (default "panel"), "accent"?: "#RRGGBB" (default KHGIS cyan #0891B2), "author"?: text} → a print-ready A4 layout of the active map: title, subtitle, legend, scale bar, north arrow, a regional inset with the area outlined, the data sources found in the map, and the date, in Khmer or English"##,
        ),
        C::new("khgis.loadArea", "Load Cambodia Area Data", "Home › Add › Cambodia Data", load_area).roles(&[]).effects(crate::policy::Effects::NONE.with(crate::policy::Effect::Network)).params(
            r##"{"province": English name (Kandal, Siem Reap…; khgis.areas lists them), "district"?: English name within it, "themes"?: ["roads"|"water"|"schools"|"health"|"places"|"worship"] (default all but worship; [] for the boundary only), "lang"?: "km"|"en" (layer names)} → adds the area's boundary (and a province's districts) from the bundled boundaries and zooms to it, then each theme from OpenStreetMap clipped to the area, answering the layers and their counts"##,
        ),
        C::new("khgis.areas", "List Cambodia Areas", "Home › Add › Cambodia Data", areas).roles(&[]).effects(crate::policy::Effects::NONE).view().params(
            r#"{"province"?: English name (its districts; none: the provinces)} → [{"name", "name_km"}]"#,
        ),
        C::new("khgis.recipe", "Run KHGIS Workflow", "Analyze › Workflows", recipe).roles(&[]).effects(crate::policy::Effects::NONE).params(
            r##"{"id": "floodExposure" (villages within a distance of rivers and water) | "schoolAccess" (villages farther than a distance from any school) | "healthAccess" (villages farther than a distance from any health facility) | "districtSchools" (schools counted per district, as a choropleth), "distance"?: km (defaults 1, 3 and 5), "lang"?: "km"|"en"} → runs the chain of tools on the layers Cambodia Data loaded (found by name) and answers what it made, with counts"##,
        ),
    ]
}

/// OpenStreetMap themes: id, English name, Khmer name, Overpass tags (as the Cambodia Data pane's).
pub const THEMES: &[(&str, &str, &str, &str)] = &[
    ("roads", "Main roads", "ផ្លូវធំៗ", "highway=trunk, highway=primary, highway=secondary, highway=tertiary"),
    ("water", "Rivers and water", "ទន្លេ និងប្រភពទឹក", "waterway=river, waterway=canal, waterway=stream, natural=water"),
    ("schools", "Schools", "សាលារៀន", "amenity=school, amenity=college, amenity=university, amenity=kindergarten"),
    ("health", "Health facilities", "មណ្ឌលសុខភាព", "amenity=hospital, amenity=clinic, amenity=doctors, healthcare=centre"),
    ("places", "Villages and towns", "ភូមិ និងទីប្រជុំជន", "place=village, place=town, place=city, place=hamlet"),
    ("worship", "Pagodas and places of worship", "វត្តអារាម", "amenity=place_of_worship"),
];

fn bundled(text: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(text).ok().and_then(|v| v["features"].as_array().cloned()).unwrap_or_default()
}

fn same_name(f: &Value, name: &str) -> bool {
    let p = &f["properties"];
    p["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name.trim())) || p["name_km"].as_str().is_some_and(|n| !n.is_empty() && n == name.trim())
}

fn areas(_: &mut Session, p: &Value) -> CmdResult {
    let list: Vec<Value> = match p.get("province").and_then(Value::as_str) {
        Some(prov) => bundled(crate::sample::CAMBODIA_DISTRICTS)
            .into_iter()
            .filter(|f| f["properties"]["province"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(prov.trim())))
            .map(|f| json!({"name": f["properties"]["name"], "name_km": ""}))
            .collect(),
        None => bundled(crate::sample::CAMBODIA_PROVINCES).into_iter().map(|f| json!({"name": f["properties"]["name"], "name_km": f["properties"]["name_km"]})).collect(),
    };
    Ok(Value::Array(list))
}

fn bbox_of(f: &Value) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    fn walk(v: &Value, b: &mut [f64; 4]) {
        match v.as_array() {
            Some(a) if a.len() >= 2 && a[0].is_number() => {
                let (x, y) = (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0));
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            Some(a) => a.iter().for_each(|c| walk(c, b)),
            None => {}
        }
    }
    walk(&f["geometry"]["coordinates"], &mut b);
    (b[0] < b[2]).then_some(b)
}

fn load_area(s: &mut Session, p: &Value) -> CmdResult {
    let khmer = p.get("lang").and_then(Value::as_str) == Some("km");
    let pname = p.get("province").and_then(Value::as_str).ok_or("province is required (khgis.areas lists them)")?;
    let prov = bundled(crate::sample::CAMBODIA_PROVINCES).into_iter().find(|f| same_name(f, pname)).ok_or_else(|| format!("no province '{pname}' (khgis.areas lists them)"))?;
    let pen = prov["properties"]["name"].as_str().unwrap_or(pname).to_string();
    let districts: Vec<Value> = bundled(crate::sample::CAMBODIA_DISTRICTS).into_iter().filter(|f| f["properties"]["province"].as_str() == Some(pen.as_str())).collect();
    let unit = match p.get("district").and_then(Value::as_str).filter(|d| !d.trim().is_empty()) {
        Some(d) => Some(districts.iter().find(|f| same_name(f, d)).cloned().ok_or_else(|| format!("no district '{d}' in {pen}"))?),
        None => None,
    };
    let feature = unit.clone().unwrap_or_else(|| prov.clone());
    let pr = &feature["properties"];
    let area = match (khmer, pr["name_km"].as_str().filter(|n| !n.is_empty())) {
        (true, Some(km)) => km.to_string(),
        _ => pr["name"].as_str().unwrap_or(pname).to_string(),
    };
    let b = bbox_of(&feature).ok_or("the area has no shape")?;
    let add = |s: &mut Session, name: &str, features: Vec<Value>| -> Result<(), String> {
        if s.map().layer_by_name(name).is_some() {
            let _ = s.run("layer.remove", &json!({"layer": name}));
        }
        let fc = json!({"type": "FeatureCollection", "features": features});
        s.run("layer.addDataText", &json!({"fileName": "area.geojson", "text": fc.to_string(), "name": name})).map(|_| ())
    };
    add(s, &area, vec![feature.clone()])?;
    let _ = s.run("layer.noFill", &json!({"layer": area}));
    let _ = s.run("layer.outline", &json!({"layer": area, "color": "#22D3EE", "width": 2.2}));
    let mut made = vec![json!({"name": area, "features": 1})];
    if unit.is_none() {
        let n = if khmer { format!("ស្រុក/ខណ្ឌ · {area}") } else { format!("Districts · {area}") };
        add(s, &n, districts)?;
        let _ = s.run("layer.noFill", &json!({"layer": n}));
        let _ = s.run("layer.outline", &json!({"layer": n, "color": "#CFFAFE88", "width": 0.8}));
        made.push(json!({"name": n, "features": count(s, &n)}));
    }
    let _ = s.run("layer.zoomTo", &json!({"layer": area}));
    let wanted: Vec<String> = match p.get("themes").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => THEMES.iter().map(|t| t.0.to_string()).filter(|t| t != "worship").collect(),
    };
    let mut failed = Vec::new();
    for (id, en, km, tags) in THEMES.iter().filter(|t| wanted.iter().any(|w| w == t.0)) {
        let name = format!("{} · {area}", if khmer { km } else { en });
        let r = match s.run("osm.download", &json!({"bbox": b, "tags": tags, "name": name})) {
            Ok(r) => r,
            Err(e) => {
                failed.push(json!({"theme": id, "error": e}));
                continue;
            }
        };
        let ids: Vec<u32> = r["layers"].as_array().into_iter().flatten().filter_map(|v| v.as_u64()).map(|v| v as u32).collect();
        for lid in ids {
            let Some(lname) = s.map().layers.iter().find(|l| l.id == lid).map(|l| l.name.clone()) else { continue };
            let clipped = format!("{lname} ✂");
            if s.run("gp.run", &json!({"tool": "clip", "params": {"input": lname, "clip": area, "output": clipped}})).is_ok() {
                let _ = s.run("layer.remove", &json!({"layer": lname}));
                let _ = s.run("layer.rename", &json!({"layer": clipped, "name": lname}));
            }
            made.push(json!({"name": lname, "features": count(s, &lname)}));
        }
    }
    Ok(json!({"area": area, "bbox": b, "layers": made, "failed": failed}))
}

/// The visible layer whose name holds every one of `all` and any one of `any` (lower case).
fn find_layer(s: &Session, any: &[&str], kind: Option<&str>) -> Option<String> {
    s.map()
        .layers
        .iter()
        .filter(|l| l.visible)
        .map(|l| l.name.clone())
        .find(|n| {
            let low = n.to_lowercase();
            any.iter().any(|k| low.contains(k)) && kind.is_none_or(|k| low.contains(k))
        })
}

fn layer_names(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(a) = v.get("layers").and_then(Value::as_array) {
        for l in a {
            if let Some(n) = l.get("name").and_then(Value::as_str).or(l.as_str()) {
                out.push(n.to_string());
            }
        }
    }
    if let Some(n) = v.get("layer").and_then(Value::as_str).or(v.get("output").and_then(Value::as_str)) {
        out.push(n.to_string());
    }
    out
}

fn count(s: &Session, name: &str) -> usize {
    s.map().layer_by_name(name).map_or(0, |l| l.dataset.features.len())
}

/// Buffers every layer found (lines and polygons too), merged into one area.
fn buffer_all(s: &mut Session, layers: &[String], km: f64, out: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        let name = format!("{out} {}", i + 1);
        s.run("gp.run", &json!({"tool": "buffer", "params": {"input": l, "distance": km, "units": "Kilometers", "dissolve": "All", "output": name}}))?;
        parts.push(name);
    }
    match parts.as_slice() {
        [] => Err("nothing to buffer".into()),
        [one] => {
            let _ = s.run("layer.rename", &json!({"layer": one, "name": out}));
            Ok(out.to_string())
        }
        many => {
            // Union the buffers' areas in turn, then dissolve them into one.
            let mut acc = many[0].clone();
            for (i, next) in many.iter().enumerate().skip(1) {
                let merged = format!("{out} union {i}");
                s.run("gp.run", &json!({"tool": "union", "params": {"input": acc, "overlay": next, "output": merged}}))?;
                acc = merged;
            }
            s.run("gp.run", &json!({"tool": "dissolve", "params": {"input": acc, "output": out}}))?;
            let temps: Vec<String> = s.map().layers.iter().map(|l| l.name.clone()).filter(|n| n.starts_with(&format!("{out} ")) && n != out).collect();
            for t in temps {
                let _ = s.run("layer.remove", &json!({"layer": t}));
            }
            Ok(out.to_string())
        }
    }
}

fn style_area(s: &mut Session, name: &str, rgba: &str) {
    let _ = s.run("layer.color", &json!({"layer": name, "color": rgba}));
    let _ = s.run("layer.outline", &json!({"layer": name, "color": &rgba[..7], "width": 1.0}));
}

fn recipe(s: &mut Session, p: &Value) -> CmdResult {
    let khmer = p.get("lang").and_then(Value::as_str) == Some("km");
    let id = p.get("id").and_then(Value::as_str).ok_or("id is required")?;
    let dist = |d: f64| p.get("distance").and_then(Value::as_f64).filter(|v| *v > 0.0 && *v <= 100.0).unwrap_or(d);
    let tr = |en: &str, km: &str| if khmer { km.to_string() } else { en.to_string() };
    let villages = find_layer(s, &["village", "ភូមិ"], Some("(points)"))
        .ok_or_else(|| tr("Load villages first (Cambodia Data).", "សូមផ្ទុកភូមិជាមុនសិន (ទិន្នន័យកម្ពុជា)។"));
    match id {
        "floodExposure" => {
            let villages = villages?;
            let km = dist(1.0);
            let water: Vec<String> = s
                .map()
                .layers
                .iter()
                .filter(|l| l.visible)
                .map(|l| l.name.clone())
                .filter(|n| {
                    let low = n.to_lowercase();
                    (low.contains("water") || low.contains("ទឹក")) && (low.contains("(lines)") || low.contains("(polygons)"))
                })
                .collect();
            if water.is_empty() {
                return Err(tr("Load rivers and water first (Cambodia Data).", "សូមផ្ទុកទន្លេ និងប្រភពទឹកជាមុនសិន។"));
            }
            let zone = format!("{} {km} km", tr("Near water", "ជិតទឹក"));
            let zone = buffer_all(s, &water, km, &zone)?;
            style_area(s, &zone, "#1E88E555");
            let out = tr(&format!("Villages within {km} km of water"), &format!("ភូមិក្នុងចម្ងាយ {km} គ.ម ពីទឹក"));
            s.run("gp.run", &json!({"tool": "clip", "params": {"input": villages, "clip": zone, "output": out}}))?;
            let _ = s.run("layer.color", &json!({"layer": out, "color": "#FF5C8A"}));
            let (n, all) = (count(s, &out), count(s, &villages));
            Ok(json!({"made": [zone, out], "summary": tr(&format!("{n} of {all} villages lie within {km} km of a river or water body."), &format!("ភូមិ {n} ក្នុងចំណោម {all} ស្ថិតក្នុងចម្ងាយ {km} គ.ម ពីទន្លេ ឬប្រភពទឹក។"))}))
        }
        "schoolAccess" | "healthAccess" => {
            let villages = villages?;
            let school = id == "schoolAccess";
            let km = dist(if school { 3.0 } else { 5.0 });
            let keys: &[&str] = if school { &["school", "សាលា"] } else { &["health", "សុខភាព"] };
            let pts = find_layer(s, keys, Some("(points)")).ok_or_else(|| {
                if school { tr("Load schools first (Cambodia Data).", "សូមផ្ទុកសាលារៀនជាមុនសិន។") } else { tr("Load health facilities first (Cambodia Data).", "សូមផ្ទុកមណ្ឌលសុខភាពជាមុនសិន។") }
            })?;
            let zone = if school { tr(&format!("Within {km} km of a school"), &format!("ក្នុង {km} គ.ម ពីសាលា")) } else { tr(&format!("Within {km} km of health care"), &format!("ក្នុង {km} គ.ម ពីមណ្ឌលសុខភាព")) };
            let zone = buffer_all(s, &[pts], km, &zone)?;
            style_area(s, &zone, if school { "#34F5A344" } else { "#C084FC44" });
            let out = if school { tr(&format!("Villages over {km} km from a school"), &format!("ភូមិឆ្ងាយជាង {km} គ.ម ពីសាលា")) } else { tr(&format!("Villages over {km} km from health care"), &format!("ភូមិឆ្ងាយជាង {km} គ.ម ពីមណ្ឌលសុខភាព")) };
            s.run("gp.run", &json!({"tool": "erase", "params": {"input": villages, "erase": zone, "output": out}}))?;
            let _ = s.run("layer.color", &json!({"layer": out, "color": "#FFC24B"}));
            let (n, all) = (count(s, &out), count(s, &villages));
            let summary = if school {
                tr(&format!("{n} of {all} villages are more than {km} km from a school."), &format!("ភូមិ {n} ក្នុងចំណោម {all} នៅឆ្ងាយជាង {km} គ.ម ពីសាលារៀន។"))
            } else {
                tr(&format!("{n} of {all} villages are more than {km} km from a health facility."), &format!("ភូមិ {n} ក្នុងចំណោម {all} នៅឆ្ងាយជាង {km} គ.ម ពីមណ្ឌលសុខភាព។"))
            };
            Ok(json!({"made": [zone, out], "summary": summary}))
        }
        "districtSchools" => {
            let districts = find_layer(s, &["districts", "ស្រុក"], None).ok_or_else(|| tr("Load a province first (Cambodia Data).", "សូមផ្ទុកខេត្តជាមុនសិន។"))?;
            let pts = find_layer(s, &["school", "សាលា"], Some("(points)")).ok_or_else(|| tr("Load schools first (Cambodia Data).", "សូមផ្ទុកសាលារៀនជាមុនសិន។"))?;
            let out = tr("Schools per district", "សាលារៀនតាមស្រុក");
            let r = s.run("gp.run", &json!({"tool": "countPoints", "params": {"polygons": districts, "points": pts, "output": out}}))?;
            let made = layer_names(&r).into_iter().next().unwrap_or(out);
            let _ = s.run("layer.symbology", &json!({"layer": made, "type": "graduated", "field": "point_count", "classes": 5, "ramp": "Viridis"}));
            Ok(json!({"made": [made], "summary": tr("Schools counted in each district.", "បានរាប់សាលារៀនក្នុងស្រុកនីមួយៗ។")}))
        }
        other => Err(format!("unknown workflow '{other}'")),
    }
}

/// The credits the map's layers and tiles call for, from what they are and where they came from.
fn sources(s: &Session, khmer: bool) -> Vec<String> {
    let m = s.map();
    let mut out: Vec<String> = Vec::new();
    let mut add = |t: &str| {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    };
    for t in m.tiles.iter().filter(|t| t.visible && !t.attribution.is_empty()) {
        add(&t.attribution);
    }
    let names: Vec<String> = m.layers.iter().filter(|l| l.visible).map(|l| l.name.to_lowercase()).collect();
    let any = |keys: &[&str]| names.iter().any(|n| keys.iter().any(|k| n.contains(k)));
    if any(&["province", "district", "ខេត្ត", "ស្រុក", "khan", "commune"]) {
        add(if khmer { "ព្រំដែនរដ្ឋបាល៖ geoBoundaries (© អ្នករួមចំណែក OpenStreetMap, ODbL)" } else { "Boundaries: geoBoundaries (© OpenStreetMap contributors, ODbL)" });
    }
    if any(&["road", "school", "health", "village", "water", "ផ្លូវ", "សាលា", "សុខភាព", "ភូមិ", "ទឹក", "វត្ត", "openstreetmap", "osm"]) {
        add("© OpenStreetMap contributors (ODbL)");
    }
    if any(&["elevation", "copernicus", "dem", "រយៈកម្ពស់"]) {
        add(if khmer { "រយៈកម្ពស់៖ Copernicus GLO-30 (© ESA)" } else { "Elevation: Copernicus GLO-30 (© ESA)" });
    }
    if any(&["sentinel"]) {
        add("Contains modified Copernicus Sentinel data");
    }
    out
}

/// The administrative unit the view is centred in, from the bundled Cambodian provinces.
fn province_at(lon: f64, lat: f64, khmer: bool) -> Option<String> {
    let v: Value = serde_json::from_str(crate::sample::CAMBODIA_PROVINCES).ok()?;
    for f in v["features"].as_array()? {
        let polys: Vec<&Value> = match f["geometry"]["type"].as_str()? {
            "Polygon" => vec![&f["geometry"]["coordinates"]],
            "MultiPolygon" => f["geometry"]["coordinates"].as_array()?.iter().collect(),
            _ => continue,
        };
        let inside = polys.iter().any(|p| p.get(0).and_then(Value::as_array).is_some_and(|ring| point_in_ring(lon, lat, ring)));
        if inside {
            let p = &f["properties"];
            let km = p["name_km"].as_str().unwrap_or("");
            let en = p["name"].as_str().unwrap_or("");
            return Some(if khmer && !km.is_empty() { format!("ខេត្ត{km}") } else { format!("{en} Province") });
        }
    }
    None
}

fn point_in_ring(x: f64, y: f64, ring: &[Value]) -> bool {
    let pts: Vec<(f64, f64)> = ring.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
    let mut inside = false;
    let n = pts.len();
    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[(i + n - 1) % n];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
    }
    inside
}

fn auto_layout(s: &mut Session, p: &Value) -> CmdResult {
    let khmer = p.get("lang").and_then(Value::as_str) == Some("km");
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    let c = s.map().center_lonlat();
    let area = province_at(c.x, c.y, khmer);
    // Wider than a province (the whole country in view): Cambodia.
    let scale = s.map().scale();
    let area = if scale > 4_000_000.0 || area.is_none() && (102.0..108.0).contains(&c.x) && (10.0..15.0).contains(&c.y) {
        Some(if khmer { "ព្រះរាជាណាចក្រកម្ពុជា".to_string() } else { "Kingdom of Cambodia".to_string() })
    } else {
        area
    };
    let layer = s.active_layer.and_then(|id| s.map().layers.iter().find(|l| l.id == id && l.visible)).map(|l| l.name.clone());
    let title = text("title").unwrap_or_else(|| match (&layer, &area) {
        // The layer already names the place (Districts · Kandal): it is the title.
        (Some(l), Some(a)) if l.contains(a.trim_start_matches("ខេត្ត").trim_end_matches(" Province")) => l.clone(),
        (Some(l), Some(a)) => format!("{l} · {a}"),
        (Some(l), None) => l.clone(),
        (None, Some(a)) => a.clone(),
        (None, None) => s.map().name.clone(),
    });
    let subtitle = text("subtitle").unwrap_or_else(|| {
        if khmer { "ផែនទីរៀបចំដោយ KHGIS".to_string() } else { "Map prepared with KHGIS".to_string() }
    });
    let mut srcs = sources(s, khmer);
    if srcs.is_empty() {
        srcs.push(if khmer { "ទិន្នន័យរបស់អ្នកប្រើ".to_string() } else { "User data".to_string() });
    }
    let headings = if khmer {
        json!(["ប្រភពទិន្នន័យ", "ប្រព័ន្ធយោង", "កំណត់សម្គាល់"])
    } else {
        json!(["Data sources", "Reference", "Notes"])
    };
    let author = text("author").unwrap_or_else(|| if khmer { "បង្កើតដោយ KHGIS · {date}".into() } else { "Made with KHGIS · {date}".into() });
    let params = json!({
        "profile": text("profile").unwrap_or_else(|| "a4".into()),
        "style": text("style").unwrap_or_else(|| "panel".into()),
        "accent": text("accent").unwrap_or_else(|| "#0891B2".into()),
        "title": title,
        "subtitle": subtitle,
        "sources": srcs,
        "headings": headings,
        "author": author,
        "locator": "inset",
        "name": if khmer { "ប្លង់ផែនទី" } else { "Map Layout" },
    });
    s.run("layout.publication", &params)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Villages, a river and a school by Phnom Penh, as Cambodia Data names them.
    fn session_with_data() -> Session {
        let mut s = Session::new(crate::sample::cambodia_project(false));
        let pts = |coords: &[(f64, f64)]| {
            let fs: Vec<Value> = coords.iter().map(|(x, y)| json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [x, y]}, "properties": {}})).collect();
            json!({"type": "FeatureCollection", "features": fs}).to_string()
        };
        s.run("layer.addDataText", &json!({"fileName": "v.geojson", "name": "Villages and towns · Kandal (points)", "text": pts(&[(104.90, 11.50), (104.91, 11.51), (105.30, 11.80)])})).unwrap();
        s.run("layer.addDataText", &json!({"fileName": "s.geojson", "name": "Schools · Kandal (points)", "text": pts(&[(104.905, 11.505)])})).unwrap();
        let river = json!({"type": "FeatureCollection", "features": [{"type": "Feature", "geometry": {"type": "LineString", "coordinates": [[104.89, 11.40], [104.92, 11.60]]}, "properties": {}}]});
        s.run("layer.addDataText", &json!({"fileName": "w.geojson", "name": "Rivers and water · Kandal (lines)", "text": river.to_string()})).unwrap();
        s
    }

    #[test]
    fn an_area_loads_from_the_bundled_boundaries() {
        let mut s = Session::new(crate::sample::cambodia_project(false));
        let r = s.run("khgis.loadArea", &json!({"province": "Kandal", "themes": []})).unwrap();
        assert_eq!(r["area"], "Kandal");
        assert!(r["layers"][1]["features"].as_u64().unwrap() >= 5, "Kandal's districts: {r}");
        let r = s.run("khgis.loadArea", &json!({"province": "កណ្ដាល", "themes": [], "lang": "km"})).unwrap();
        assert_eq!(r["area"], "កណ្ដាល");
        assert_eq!(s.run("khgis.areas", &json!({})).unwrap().as_array().unwrap().len(), 25);
        assert!(s.run("khgis.loadArea", &json!({"province": "Atlantis"})).is_err());
    }

    #[test]
    fn the_flood_and_school_workflows_count_villages() {
        let mut s = session_with_data();
        let r = s.run("khgis.recipe", &json!({"id": "floodExposure", "distance": 3})).unwrap();
        assert!(r["summary"].as_str().unwrap().starts_with("2 of 3"), "{r}");
        let r = s.run("khgis.recipe", &json!({"id": "schoolAccess", "distance": 3})).unwrap();
        assert!(r["summary"].as_str().unwrap().starts_with("1 of 3"), "{r}");
        let r = s.run("khgis.recipe", &json!({"id": "healthAccess"}));
        assert!(r.is_err(), "no health layer loaded");
    }

    #[test]
    fn the_province_under_phnom_penh_is_found() {
        assert_eq!(province_at(104.92, 11.56, false).as_deref(), Some("Phnom Penh Province"));
        assert_eq!(province_at(103.86, 13.36, true).as_deref(), Some("ខេត្តសៀមរាប"));
        assert_eq!(province_at(2.0, 48.0, false), None);
    }

    #[test]
    fn an_auto_layout_is_made_in_khmer() {
        let mut s = Session::new(crate::sample::cambodia_project(true));
        let n = s.project.layouts.len();
        s.run("khgis.autoLayout", &json!({"lang": "km"})).unwrap();
        assert_eq!(s.project.layouts.len(), n + 1);
        let text = serde_json::to_string(&s.project.layouts.last().unwrap()).unwrap();
        assert!(text.contains("ប្រភពទិន្នន័យ"), "Khmer headings");
        assert!(text.contains("geoBoundaries"), "sources found from the layers");
    }
}
