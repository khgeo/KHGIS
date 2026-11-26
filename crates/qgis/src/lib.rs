//! L3: QGIS Processing as geoprocessing tools — QGIS's own algorithms and those of every
//! installed QGIS plugin that provides Processing algorithms.
//!
//! KHGIS doesn't link QGIS. It runs the user's QGIS instead: natively a bridge script
//! (`bridge/khgis_qgis_bridge.py`) under the Python that comes with QGIS, which starts QGIS
//! without a window, loads Processing and the plugins, and answers one JSON line per request
//! (see `runner`). In the browser the same script runs as a small web service
//! (`khgis_qgis_bridge.py serve`, packaged in `packaging/qgis-bridge`), and the web app passes
//! its manifests to [`set_manifests`].
//!
//! [`register`] adds the algorithms to [`rustgis_analysis`], listed under "QGIS ›" toolboxes;
//! with no QGIS connected there are none. The list comes from the bridge's manifests, read once
//! when QGIS is connected and cached, so listing the tools never starts QGIS.
//!
//! Inputs are staged like the GeoLibre tools': raster layers by their file (or a GeoTIFF copy),
//! vector layers (their selection, if any) as GeoJSON in a metric coordinate system (the first
//! raster input's, else the UTM zone of the data), so distances are in metres. Vector outputs
//! come back from the bridge as GeoJSON in longitude/latitude; raster outputs as GeoTIFF.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
pub mod runner;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rustgis_analysis::{Inputs, Output, OutputLayer, ParamKind, ParamSpec, ToolResult, ToolSpec};
use rustgis_data::raster::{Crs, Transformer};
use rustgis_data::{Coord, Dataset, Extent, Geometry};
use serde_json::{Map, Value, json};

/// Tool ids get this prefix, so they never clash with the built-in tools.
pub const PREFIX: &str = "qgis:";

/// The key the QGIS tools are registered under ([`rustgis_analysis::register_external`]).
pub const SOURCE: &str = "qgis";

/// The bridge script, run with QGIS's Python.
pub const BRIDGE_SCRIPT: &str = include_str!("../bridge/khgis_qgis_bridge.py");

/// The bridge protocol this KHGIS speaks.
pub const BRIDGE_VERSION: u64 = 1;

/// The most algorithms taken from one manifest (QGIS with many plugins lists about 1,500).
const MAX_ALGORITHMS: usize = 5_000;

/// The QGIS providers whose algorithms are left out: they need a window or an open QGIS
/// project, or duplicate other providers' tools with steps KHGIS can't pass.
const SKIPPED_PROVIDERS: &[&str] = &["model", "script", "project"];

/// Adds the QGIS algorithms to the geoprocessing tools.
pub fn register() {
    rustgis_analysis::register_external(SOURCE, specs, run);
}

/// Manifests given directly (the web app's bridge service, or the cache a native connect wrote).
static MANIFESTS: Mutex<Option<Value>> = Mutex::new(None);

/// Uses `json` (the object `khgis_qgis_bridge.py manifests` prints under `ok`) for the tool
/// list, and reloads it. Returns how many algorithms it lists.
pub fn set_manifests(json: &str) -> Result<usize, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("unexpected QGIS manifests: {e}"))?;
    let v = v.get("ok").cloned().unwrap_or(v);
    let n = v.get("algorithms").and_then(Value::as_array).map_or(0, Vec::len);
    if v.get("algorithms").is_none() {
        return Err("the QGIS manifests list no algorithms".into());
    }
    if let Ok(mut m) = MANIFESTS.lock() {
        *m = Some(v);
    }
    rustgis_analysis::reload_external_of(SOURCE);
    let _ = rustgis_analysis::external_tools_of(SOURCE);
    Ok(n)
}

/// Forgets the manifests: the QGIS tools leave the toolbox.
pub fn clear_manifests() {
    if let Ok(mut m) = MANIFESTS.lock() {
        *m = None;
    }
    rustgis_analysis::reload_external_of(SOURCE);
}

/// What the connected QGIS reported: its version, how many algorithms, and its plugins.
pub fn status() -> Value {
    let m = manifests();
    match m {
        Some(m) => json!({
            "connected": true,
            "qgis": m.get("qgis").cloned().unwrap_or(Value::Null),
            "algorithms": m.get("algorithms").and_then(Value::as_array).map_or(0, Vec::len),
            "plugins": m.get("plugins").cloned().unwrap_or_else(|| json!([])),
        }),
        None => json!({"connected": false}),
    }
}

/// The manifests: given directly, else the native cache (none without one).
fn manifests() -> Option<Value> {
    if let Some(m) = MANIFESTS.lock().ok().and_then(|m| m.clone()) {
        return Some(m);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let cached = runner::cached_manifests()?;
        if let Ok(mut m) = MANIFESTS.lock() {
            *m = Some(cached.clone());
        }
        Some(cached)
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// A parameter's place in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// A layer or file to read: staged.
    Input,
    /// A vector layer the algorithm writes (a sink or vector destination).
    VectorOut,
    /// A raster the algorithm writes.
    RasterOut,
    /// Another file the algorithm writes (a report, an HTML page, a table).
    FileOut,
    /// A folder the algorithm writes into.
    FolderOut,
    /// A value passed as it is.
    Option,
}

/// What an input holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Data {
    Vector,
    Raster,
    Any,
    File,
}

#[derive(Clone, Debug)]
struct MetaParam {
    name: String,
    /// QGIS's parameter type (`source`, `number`, `enum`, …).
    qtype: String,
    role: Role,
    data: Data,
    many: bool,
    integer: bool,
    /// An enum's options, in QGIS's order (the value passed is the index).
    options: Vec<String>,
    /// A file output's extension.
    extension: String,
    description: String,
    /// The name the form gives it (see [`form_name`]); `None` when the form doesn't show it.
    form: Option<String>,
}

/// What a run needs to know about an algorithm beyond its [`ToolSpec`].
#[derive(Clone, Debug)]
struct Meta {
    params: Vec<MetaParam>,
}

static META: Mutex<Option<HashMap<String, Meta>>> = Mutex::new(None);

/// A `'static` copy of `s`, shared, so reloading the tools reuses the strings it made before.
fn leak(s: impl Into<String>) -> &'static str {
    static STRINGS: Mutex<Option<std::collections::HashSet<&'static str>>> = Mutex::new(None);
    let s = s.into();
    let Ok(mut set) = STRINGS.lock() else { return Box::leak(s.into_boxed_str()) };
    let set = set.get_or_insert_with(Default::default);
    if let Some(&have) = set.get(s.as_str()) {
        return have;
    }
    let new: &'static str = Box::leak(s.into_boxed_str());
    set.insert(new);
    new
}

/// Builds every algorithm's spec from the manifests (none without them).
fn specs() -> Vec<ToolSpec> {
    let Some(m) = manifests() else { return Vec::new() };
    let mut meta = HashMap::new();
    let mut out: Vec<ToolSpec> = m
        .get("algorithms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ALGORITHMS)
        .filter_map(|a| {
            let (spec, info) = spec_of(a)?;
            meta.insert(spec.id.trim_start_matches(PREFIX).to_string(), info);
            Some(spec)
        })
        .collect();
    if let Ok(mut g) = META.lock() {
        *g = Some(meta);
    }
    out.sort_by(|a, b| a.toolbox.cmp(b.toolbox).then(a.name.cmp(b.name)));
    out
}

/// Where an algorithm sits: `QGIS › GDAL › Raster analysis`. QGIS's own providers keep their
/// names; a plugin's provider is named by the plugin.
fn toolbox_of(a: &Value) -> String {
    let provider = a.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Other");
    match a.get("group").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        Some(group) => format!("QGIS › {provider} › {group}"),
        None => format!("QGIS › {provider}"),
    }
}

/// `Input layer` stays; `INPUT` becomes `Input`.
fn label_of(description: &str, name: &str) -> String {
    let d = description.trim();
    if !d.is_empty() {
        return d.to_string();
    }
    let words = name.replace('_', " ").to_lowercase();
    let mut c = words.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The parameter name as KHGIS forms need it: a lower-case first letter, then letters, digits
/// and underscores. QGIS names are usually upper case (`INPUT`): the original is kept in
/// [`MetaParam::name`].
fn form_name(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if let Some(first) = s.get(..1) {
        let lower = first.to_ascii_lowercase();
        s.replace_range(..1, &lower);
    }
    if !s.starts_with(|c: char| c.is_ascii_lowercase()) {
        s.insert(0, 'p');
    }
    s
}

fn spec_of(a: &Value) -> Option<(ToolSpec, Meta)> {
    let id = a.get("id")?.as_str()?.trim();
    if id.is_empty() || id.len() > 200 {
        return None;
    }
    let provider_id = a.get("provider_id").and_then(Value::as_str).unwrap_or("");
    if SKIPPED_PROVIDERS.contains(&provider_id) {
        return None;
    }
    let name = a.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(id);
    let summary: String = a.get("help").and_then(Value::as_str).unwrap_or("").chars().take(600).collect();
    let mut params = Vec::new();
    let mut meta = Vec::new();
    // Form names must be unique; two QGIS names can map to one (`Input` and `INPUT`).
    let mut used = std::collections::HashSet::new();
    for p in a.get("params").and_then(Value::as_array).into_iter().flatten() {
        let Some(pname) = p.get("name").and_then(Value::as_str) else { continue };
        let qtype = p.get("type").and_then(Value::as_str).unwrap_or("string").to_string();
        let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        let optional = p.get("optional").and_then(Value::as_bool).unwrap_or(false);
        let many = p.get("multiple").and_then(Value::as_bool).unwrap_or(false);
        let layer = p.get("layer").and_then(Value::as_str).unwrap_or("any");
        let data = match layer {
            "vector" => Data::Vector,
            "raster" => Data::Raster,
            "file" => Data::File,
            _ => Data::Any,
        };
        let role = match qtype.as_str() {
            "sink" | "vectorDestination" => Role::VectorOut,
            "rasterDestination" => Role::RasterOut,
            "fileDestination" => Role::FileOut,
            "folderDestination" => Role::FolderOut,
            "source" | "vector" | "raster" | "multilayer" | "layer" | "mapLayer" | "file" | "pointcloud" | "mesh" => Role::Input,
            _ => Role::Option,
        };
        let description = p.get("description").and_then(Value::as_str).unwrap_or("").to_string();
        let options: Vec<String> = p
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| o.as_str().map(str::to_string))
            .take(64)
            .collect();
        let mp = MetaParam {
            name: pname.to_string(),
            qtype: qtype.clone(),
            role,
            data: if qtype == "file" { Data::File } else { data },
            many,
            integer: p.get("integer").and_then(Value::as_bool).unwrap_or(false),
            options: options.clone(),
            extension: p
                .get("extension")
                .and_then(Value::as_str)
                .map(|e| e.trim_start_matches('.').chars().filter(char::is_ascii_alphanumeric).take(8).collect())
                .unwrap_or_default(),
            description: description.clone(),
            form: None,
        };
        // An optional output (a second report, a table of errors) is still written: KHGIS
        // passes a file for every output so QGIS never writes to a temporary place it forgets.
        let shows = !hidden && matches!(role, Role::Input | Role::Option);
        if !shows {
            meta.push(mp);
            continue;
        }
        let mut fname = form_name(pname);
        while !used.insert(fname.clone()) {
            fname.push('_');
        }
        meta.push(MetaParam { form: Some(fname.clone()), ..mp });
        let kind = match (role, qtype.as_str()) {
            (Role::Input, "multilayer") => match data {
                Data::Raster => ParamKind::RasterLayers,
                Data::File => ParamKind::Text,
                _ => ParamKind::VectorLayers,
            },
            (Role::Input, "file") => ParamKind::Path,
            (Role::Input, _) => match (data, p.get("geometry").and_then(Value::as_str).unwrap_or("")) {
                (Data::Raster, _) => ParamKind::RasterLayer,
                (Data::Vector, "point") => ParamKind::PointLayer,
                (Data::Vector, "polygon") => ParamKind::PolygonLayer,
                (Data::Vector, _) => ParamKind::VectorLayer,
                _ => ParamKind::Layer,
            },
            (_, "boolean") => ParamKind::Bool,
            (_, "number" | "distance" | "scale" | "duration" | "band" | "area" | "volume") => ParamKind::Number,
            (_, "enum") if !many && !options.is_empty() => {
                // Labels must be unique for the form: a repeated one gets its number.
                let mut seen = std::collections::HashSet::new();
                let labels: Vec<&'static str> = options
                    .iter()
                    .enumerate()
                    .map(|(i, o)| if seen.insert(o.clone()) { leak(o.clone()) } else { leak(format!("{o} ({})", i + 1)) })
                    .collect();
                ParamKind::Choice(Box::leak(labels.into_boxed_slice()))
            }
            (_, "field") => match p.get("parent").and_then(Value::as_str) {
                Some(parent) if many => ParamKind::Fields { input: leak(form_name(parent)) },
                _ => ParamKind::Field,
            },
            _ => ParamKind::Text,
        };
        let default = default_text(p, &kind, &options);
        let mut help = p.get("help").and_then(Value::as_str).unwrap_or("").trim().to_string();
        match qtype.as_str() {
            "crs" => help.push_str(" A coordinate system, such as EPSG:32648."),
            "extent" => help.push_str(" xmin,xmax,ymin,ymax [EPSG:4326], or a layer name."),
            "point" => help.push_str(" x,y [EPSG:4326]."),
            "enum" if many => {
                help.push_str(&format!(" Numbers of the options, separated by commas: {}.", numbered(&options)));
            }
            "matrix" | "fieldmapping" | "aggregates" => help.push_str(" As JSON."),
            _ => {}
        }
        if many && matches!(role, Role::Input) {
            help.push_str(" (layer names or file paths, separated by ;)");
        }
        params.push(ParamSpec {
            name: leak(fname),
            label: leak(label_of(&description, pname)),
            kind,
            required: !optional,
            default: leak(default),
            help: leak(help.trim().to_string()),
        });
    }
    let spec = ToolSpec {
        id: leak(format!("{PREFIX}{id}")),
        name: leak(name),
        toolbox: leak(toolbox_of(a)),
        summary: leak(summary),
        params: Box::leak(params.into_boxed_slice()),
        run: rustgis_analysis::external_placeholder,
    };
    Some((spec, Meta { params: meta }))
}

/// `0 = Mean, 1 = Median, …`.
fn numbered(options: &[String]) -> String {
    options.iter().enumerate().map(|(i, o)| format!("{i} = {o}")).collect::<Vec<_>>().join(", ")
}

/// A parameter's default as the JSON text [`ParamSpec::default`] holds (empty for none).
fn default_text(p: &Value, kind: &ParamKind, options: &[String]) -> String {
    let Some(d) = p.get("default").filter(|d| !d.is_null()) else { return String::new() };
    match kind {
        // Layers and files have no default worth showing (QGIS lists example paths).
        ParamKind::Layer
        | ParamKind::VectorLayer
        | ParamKind::PointLayer
        | ParamKind::PolygonLayer
        | ParamKind::RasterLayer
        | ParamKind::RasterLayers
        | ParamKind::VectorLayers
        | ParamKind::Path => String::new(),
        ParamKind::Bool => match d {
            Value::Bool(b) => b.to_string(),
            Value::String(s) => (s.eq_ignore_ascii_case("true") || s == "1").to_string(),
            Value::Number(n) => (n.as_f64().unwrap_or(0.0) != 0.0).to_string(),
            _ => String::new(),
        },
        ParamKind::Number => match d {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| json!(n).to_string()).unwrap_or_default(),
            _ => String::new(),
        },
        ParamKind::Choice(labels) => {
            let index = match d {
                Value::Number(n) => n.as_u64().map(|n| n as usize),
                Value::String(s) => s.trim().parse::<usize>().ok().or_else(|| options.iter().position(|o| o == s)),
                Value::Array(a) => a.first().and_then(Value::as_u64).map(|n| n as usize),
                _ => None,
            };
            index.and_then(|i| labels.get(i)).map(|l| Value::String((*l).to_string()).to_string()).unwrap_or_default()
        }
        _ => match d {
            Value::String(s) if s.is_empty() => String::new(),
            Value::String(s) => Value::String(s.clone()).to_string(),
            Value::Array(a) => {
                Value::String(a.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).collect::<Vec<_>>().join(","))
                    .to_string()
            }
            other => Value::String(other.to_string()).to_string(),
        },
    }
}

/// `g` with every point converted by `f`, or `None` when a point can't be.
fn convert(g: &Geometry, f: impl Fn(Coord) -> Option<Coord>) -> Option<Geometry> {
    let ok = std::cell::Cell::new(true);
    let out = g.map_coords(&|c| {
        f(c).filter(|m| m.is_finite()).unwrap_or_else(|| {
            ok.set(false);
            c
        })
    });
    ok.get().then_some(out)
}

/// A file-name-safe version of a layer name.
fn safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(60).collect();
    if s.is_empty() { "layer".into() } else { s }
}

/// The coordinate system vector inputs are written in: the first raster input's, else the UTM
/// zone around the vector data (metres), else longitude/latitude for data spanning the world.
fn working_crs(rasters: &[Crs], vectors: &[Extent]) -> Crs {
    if let Some(c) = rasters.first() {
        return *c;
    }
    let e = vectors.iter().fold(Extent::EMPTY, |a, b| a.union(b));
    if e.is_empty() || e.width() > 40.0 {
        return Crs::LonLat;
    }
    let c = e.center();
    let zone = (((c.x + 180.0) / 6.0).floor() as i64 + 1).clamp(1, 60) as u32;
    Crs::from_epsg(if c.y >= 0.0 { 32600 + zone } else { 32700 + zone }).unwrap_or(Crs::LonLat)
}

/// A vector layer (its selection, if any) as GeoJSON in `crs`, and how many features' geometry
/// couldn't be converted (they're written without geometry).
fn vector_bytes(ds: &Dataset, rows: &[usize], crs: Crs, tf: &Transformer) -> Result<(Vec<u8>, usize), String> {
    let mut out = ds.clone();
    if !rows.is_empty() {
        out.features = rows.iter().filter_map(|&r| ds.features.get(r).cloned()).collect();
    }
    let mut lost = 0;
    if crs != Crs::LonLat {
        for ft in &mut out.features {
            if let Some(g) = &ft.geometry {
                ft.geometry = convert(g, |c| tf.from_lonlat(c));
                lost += usize::from(ft.geometry.is_none());
            }
        }
    }
    let mut json = rustgis_data::io::to_geojson(&out);
    if crs != Crs::LonLat
        && let Some(o) = json.as_object_mut()
    {
        // The GeoJSON 2008 `crs` member, which GDAL (and so QGIS) honours when reading.
        o.insert("crs".into(), json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", crs.epsg())}}));
    }
    Ok((serde_json::to_vec(&json).map_err(|e| e.to_string())?, lost))
}

/// A file a parameter refers to.
#[derive(Clone, Debug)]
enum FileRef {
    /// A staged input or an output, by file name in the run's working folder.
    Work(String),
    /// A file the user named, used where it is (native only).
    Path(String),
}

/// A parameter's value in a run.
#[derive(Clone, Debug)]
enum Arg {
    /// One file, or several (a list).
    Files(Vec<FileRef>, bool),
    Value(Value),
}

/// A run, prepared: the input files to stage, the parameters, and what to read back. A runner
/// (the native bridge, or the web app's bridge service) stages the files in a working folder,
/// sends [`Job::request`], and hands the new files and the result to [`finish`].
#[derive(Clone, Debug)]
pub struct Job {
    /// QGIS's algorithm id (without [`PREFIX`]).
    pub alg: String,
    /// The algorithm's display name.
    pub name: String,
    /// Input files to place in the working folder, by file name.
    pub files: Vec<(String, Vec<u8>)>,
    params: Vec<(String, Arg)>,
    /// (QGIS parameter, file name, layer name) of each vector output.
    vector_outputs: Vec<(String, String, String)>,
    /// (file name, layer name) of every other output file.
    outputs: Vec<(String, String)>,
    /// Problems found while staging that don't stop the run.
    notes: Vec<String>,
}

impl Job {
    fn stage(&mut self, name: &str, ext: &str, bytes: Vec<u8>) -> FileRef {
        // Numbered, so two layers whose safe names agree never share a file.
        let file = format!("in{}-{}.{ext}", self.files.len(), safe(name));
        self.files.push((file.clone(), bytes));
        FileRef::Work(file)
    }

    /// The bridge's job: `{"alg", "params", "vector_outputs"}`, with working-folder files under
    /// `dir` (a scratch folder natively, `@WORK@` for the web service, which puts its own).
    pub fn request(&self, dir: &str) -> Value {
        let dir = dir.trim_end_matches(['/', '\\']);
        let sep = if dir.contains('\\') && !dir.contains('/') { '\\' } else { '/' };
        let path = |f: &FileRef| match f {
            FileRef::Work(n) => format!("{dir}{sep}{n}"),
            FileRef::Path(p) => p.clone(),
        };
        let mut params = Map::new();
        for (k, a) in &self.params {
            let v = match a {
                Arg::Value(v) => v.clone(),
                Arg::Files(files, true) => Value::Array(files.iter().map(|f| Value::String(path(f))).collect()),
                Arg::Files(files, false) => files.first().map(|f| Value::String(path(f))).unwrap_or(Value::Null),
            };
            params.insert(k.clone(), v);
        }
        let outs: Map<String, Value> =
            self.vector_outputs.iter().map(|(k, f, _)| (k.clone(), Value::String(path(&FileRef::Work(f.clone()))))).collect();
        json!({"alg": self.alg, "params": params, "vector_outputs": outs})
    }
}

/// Whether `p` names something on disk (never in the browser, which has no file system).
fn on_disk(p: &str) -> bool {
    !cfg!(target_arch = "wasm32") && Path::new(p).exists()
}

/// Stages one input (a layer by name, or a file path) for `job`.
fn stage(inputs: &dyn Inputs, value: &str, data: Data, crs: Crs, tf: &Transformer, job: &mut Job) -> Result<FileRef, String> {
    if data != Data::Vector
        && let Some(r) = inputs.raster(value)
    {
        // A scan has no place on the ground for QGIS to work in.
        r.raster.require_coordinates().map_err(|e| format!("{value}: {e}"))?;
        if let Some(p) = r.path.filter(|p| on_disk(p)) {
            return Ok(FileRef::Path(p.to_string()));
        }
        if let Some((file_name, bytes)) = r.embedded.filter(|(_, b)| !rustgis_data::raster::cog::Sparse::is_container(b)) {
            let ext = Path::new(file_name).extension().and_then(|e| e.to_str()).map(safe).unwrap_or_else(|| "tif".into());
            return Ok(job.stage(value, &ext, bytes.to_vec()));
        }
        let bytes = rustgis_data::raster::geotiff::write(r.raster, 0).map_err(|e| format!("{value}: {e}"))?;
        return Ok(job.stage(value, "tif", bytes));
    }
    if data != Data::Raster
        && let Some(ds) = inputs.dataset(value)
    {
        if ds.features.iter().all(|f| f.geometry.is_none()) {
            let text = rustgis_data::io::to_csv(ds).map_err(|e| format!("{value}: {e}"))?;
            return Ok(job.stage(value, "csv", text.into_bytes()));
        }
        let (bytes, lost) = vector_bytes(ds, inputs.selection(value), crs, tf)?;
        if lost > 0 {
            job.notes.push(format!("{value}: {lost} features lie outside the working coordinate system and were passed without geometry"));
        }
        return Ok(job.stage(value, "geojson", bytes));
    }
    if on_disk(value) {
        return Ok(FileRef::Path(value.to_string()));
    }
    Err(format!("there is no layer or file named '{value}'"))
}

/// A form value (text) as the value QGIS expects for a parameter of type `p.qtype`.
fn option_value(v: &Value, p: &MetaParam) -> Value {
    let text = v.as_str().map(str::trim);
    match p.qtype.as_str() {
        "boolean" => match (v, text) {
            (Value::Bool(b), _) => Value::Bool(*b),
            (_, Some(s)) => Value::Bool(s.eq_ignore_ascii_case("true") || s == "1"),
            _ => v.clone(),
        },
        "number" | "distance" | "scale" | "duration" | "band" | "area" | "volume" => {
            let n = v.as_f64().or_else(|| text.and_then(|s| s.parse::<f64>().ok())).filter(|n| n.is_finite());
            match n {
                Some(n) if p.integer || p.qtype == "band" || (n.fract() == 0.0 && n.abs() < 9.0e15 && p.qtype != "distance") => {
                    json!(n.round() as i64)
                }
                Some(n) => json!(n),
                None => v.clone(),
            }
        }
        "enum" if p.many => {
            let picks: Vec<Value> = text
                .unwrap_or("")
                .split([',', ';'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<u64>().ok().or_else(|| p.options.iter().position(|o| o == s).map(|i| i as u64)))
                .map(|i| json!(i))
                .collect();
            Value::Array(picks)
        }
        "enum" => {
            let s = text.unwrap_or("");
            // A label (possibly numbered when repeated), or an index.
            let index = p
                .options
                .iter()
                .position(|o| o == s)
                .or_else(|| p.options.iter().enumerate().position(|(i, o)| format!("{o} ({})", i + 1) == s))
                .or_else(|| s.parse::<usize>().ok());
            index.map_or_else(|| v.clone(), |i| json!(i))
        }
        "field" if p.many => match v {
            Value::Array(_) => v.clone(),
            _ => Value::Array(text.unwrap_or("").split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(|s| json!(s)).collect()),
        },
        "matrix" | "fieldmapping" | "aggregates" | "tininputlayers" => {
            text.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_else(|| v.clone())
        }
        _ => v.clone(),
    }
}

/// Prepares a run of `spec` (a QGIS algorithm) on the map's layers, with `params` already
/// resolved (defaults filled, required ones present).
pub fn prepare(spec: &ToolSpec, inputs: &dyn Inputs, params: &Value) -> Result<Job, String> {
    let id = spec.id.strip_prefix(PREFIX).ok_or("not a QGIS algorithm")?;
    let meta = META
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(id)).cloned())
        .ok_or_else(|| format!("{}: unknown algorithm", spec.name))?;
    let value_of = |qname: &str| {
        let p = meta.params.iter().find(|p| p.name == qname)?;
        params.get(p.form.as_deref()?).filter(|v| !v.is_null() && v.as_str() != Some(""))
    };
    let names = |v: &Value| -> Vec<String> {
        match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => {
                v.as_str().map(|s| s.split(';').map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect()).unwrap_or_default()
            }
        }
    };

    let mut raster_crs = Vec::new();
    let mut vector_extents = Vec::new();
    for p in meta.params.iter().filter(|p| p.role == Role::Input) {
        for n in value_of(&p.name).map(names).unwrap_or_default() {
            if let Some(r) = inputs.raster(&n) {
                raster_crs.push(r.raster.crs);
            } else if let Some(ds) = inputs.dataset(&n) {
                vector_extents.push(ds.extent());
            }
        }
    }
    let crs = working_crs(&raster_crs, &vector_extents);
    let tf = crs.transformer()?;
    let mut job = Job {
        alg: id.to_string(),
        name: spec.name.to_string(),
        files: Vec::new(),
        params: Vec::new(),
        vector_outputs: Vec::new(),
        outputs: Vec::new(),
        notes: Vec::new(),
    };
    let n_outputs = meta.params.iter().filter(|p| matches!(p.role, Role::VectorOut | Role::RasterOut | Role::FileOut)).count();
    let out_name = |p: &MetaParam| {
        if n_outputs > 1 { format!("{} — {}", spec.name, label_of(&p.description, &p.name)) } else { spec.name.to_string() }
    };
    for p in &meta.params {
        match p.role {
            Role::Input => {
                let Some(v) = value_of(&p.name) else { continue };
                let files = names(v).iter().map(|n| stage(inputs, n, p.data, crs, &tf, &mut job)).collect::<Result<Vec<_>, _>>()?;
                if !files.is_empty() {
                    job.params.push((p.name.clone(), Arg::Files(files, p.qtype == "multilayer")));
                }
            }
            Role::VectorOut => {
                let file = format!("out-{}.geojson", safe(&p.name));
                job.vector_outputs.push((p.name.clone(), file.clone(), out_name(p)));
                job.outputs.push((file, out_name(p)));
            }
            Role::RasterOut => {
                let file = format!("out-{}.tif", safe(&p.name));
                job.params.push((p.name.clone(), Arg::Files(vec![FileRef::Work(file.clone())], false)));
                job.outputs.push((file, out_name(p)));
            }
            Role::FileOut => {
                let ext = if p.extension.is_empty() { "txt".to_string() } else { p.extension.to_ascii_lowercase() };
                let file = format!("out-{}.{ext}", safe(&p.name));
                job.params.push((p.name.clone(), Arg::Files(vec![FileRef::Work(file.clone())], false)));
                job.outputs.push((file, out_name(p)));
            }
            // A folder output (tiles, a set of files) stays in QGIS's temporary folder.
            Role::FolderOut => {}
            Role::Option => {
                let Some(v) = value_of(&p.name) else { continue };
                job.params.push((p.name.clone(), Arg::Value(option_value(v, p))));
            }
        }
    }
    Ok(job)
}

/// Reads one output file back as a layer or a message.
fn load(file_name: &str, bytes: &[u8], layer_name: &str, messages: &mut Vec<String>) -> Option<OutputLayer> {
    let ext = Path::new(file_name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "tif" | "tiff" | "asc" => match rustgis_data::raster::read_bytes(file_name, bytes) {
            Ok(_) => Some(OutputLayer::Raster { name: layer_name.to_string(), file_name: file_name.to_string(), bytes: bytes.to_vec() }),
            Err(e) => {
                messages.push(format!("{layer_name}: {e}"));
                None
            }
        },
        "geojson" | "json" | "csv" => match rustgis_data::io::read_bytes(file_name, bytes) {
            Ok((mut ds, _)) => {
                ds.name = layer_name.to_string();
                Some(if ds.features.iter().all(|f| f.geometry.is_none()) { OutputLayer::Table(ds) } else { OutputLayer::Vector(ds) })
            }
            Err(_) if ext == "json" => {
                text_message(bytes, layer_name, messages);
                None
            }
            Err(e) => {
                messages.push(format!("{layer_name}: {e}"));
                None
            }
        },
        "txt" | "html" | "htm" => {
            text_message(bytes, layer_name, messages);
            None
        }
        _ => {
            messages.push(format!("{layer_name}: QGIS wrote a .{ext} file, which KHGIS can't open as a layer"));
            None
        }
    }
}

/// The start of a text or HTML report as a message.
fn text_message(bytes: &[u8], layer_name: &str, messages: &mut Vec<String>) {
    let text = String::from_utf8_lossy(bytes);
    let mut plain = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                plain.push(' ');
            }
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    let lines: Vec<&str> = plain.lines().map(str::trim).filter(|l| !l.is_empty()).take(30).collect();
    if !lines.is_empty() {
        messages.push(format!("{layer_name}:\n{}", lines.join("\n")));
    }
}

/// Turns a finished run into layers and messages. `files` are the new files in the working
/// folder (by file name), `result` the bridge's answer (`{"results", "notes"}`), `logs` the
/// lines QGIS logged while it ran.
pub fn finish(job: &Job, files: &[(String, Vec<u8>)], result: &Value, logs: &[String]) -> ToolResult {
    let mut messages = job.notes.clone();
    messages.extend(result.get("notes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    // QGIS ends with a line of its raw results (temporary paths): those come back as layers.
    messages.extend(logs.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with("Results: {")).take(40).map(str::to_string));
    let mut layers = Vec::new();
    for (file, name) in &job.outputs {
        if let Some((f, bytes)) = files.iter().find(|(f, _)| f == file)
            && let Some(l) = load(f, bytes, name, &mut messages)
        {
            layers.push(l);
        }
    }
    // Numbers and text the algorithm returns rather than writes (statistics, counts).
    if let Some(results) = result.get("results").and_then(Value::as_object) {
        for (k, v) in results {
            let text = match v {
                Value::String(s) if job.outputs.iter().any(|(f, _)| s.ends_with(f.as_str())) || s.ends_with(".gpkg") => continue,
                Value::String(s) if Path::new(s).is_absolute() || s.starts_with("@WORK@") => continue,
                Value::String(s) => s.clone(),
                Value::Null => continue,
                other => other.to_string(),
            };
            let text: String = text.chars().take(2000).collect();
            messages.push(format!("{k}: {text}"));
        }
    }
    if layers.is_empty() && messages.is_empty() {
        messages.push(format!("{} finished without output", job.name));
    }
    ToolResult { output: Output::Layers(layers), messages }
}

/// Runs an algorithm to completion through the native bridge. The browser's bridge service is
/// asynchronous: the web app uses [`prepare`] and [`finish`] around it instead.
fn run(spec: &ToolSpec, inputs: &dyn Inputs, params: &Value) -> Result<ToolResult, String> {
    let job = prepare(spec, inputs, params)?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        runner::run(&job).map_err(|e| if e == rustgis_analysis::control::CANCELLED { e } else { format!("{}: {e}", spec.name) })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = job;
        Err(format!("{}: QGIS algorithms run through the KHGIS QGIS bridge service; connect one in Plugins › QGIS", spec.name))
    }
}

#[cfg(test)]
mod tests;
