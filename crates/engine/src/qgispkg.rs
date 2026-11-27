//! Project › Save as GeoPackage for QGIS: the active map as one GeoPackage that QGIS opens and
//! keeps working with — every vector layer as a table, each with its style as the table's
//! default style (`layer_styles`, which QGIS applies when the table is added), and a QGIS project
//! stored inside (`qgis_projects`, which QGIS lists under the GeoPackage in its Browser and opens
//! with Project › Open From › GeoPackage) holding the layers in their order, the web tile layers
//! (XYZ) and the view.
//!
//! The file is written with KHGIS's own SQLite writer, so the browser makes it too. The project
//! refers to the layers by the GeoPackage's file name (`./name.gpkg|layername=…`), as QGIS's own
//! projects stored in a GeoPackage do: keep the file's name, or QGIS asks where the layers went.
//!
//! Raster layers travel as they look: a pyramid of Web Mercator PNG tiles (the GeoPackage tiles
//! QGIS and GDAL read), drawn with the layer's style as the map draws it, so a class map or a
//! false-colour composite shows the same in QGIS (its values stay in KHGIS: export a GeoTIFF for
//! analysis). The project also keeps KHGIS's print layouts as QGIS layouts (`qgislayouts`).
//!
//! What doesn't travel is said, never dropped silently: the features of a streamed layer not yet
//! read, a raster too large to tile, layout items QGIS has no match for, and styles QGIS draws
//! differently (stacked fills and strokes, effects) are each named in a notice.

use std::fmt::Write as _;

use rustgis_data::sqlite_write::{Header, Sql, Table, database};
use rustgis_data::{FieldType, Geometry, GeometryType, Value};
use rustgis_map::symbology::{Marker, Renderer, Symbol};
use rustgis_map::{Layer, Map, Rgba};

/// What a save made: the file, and what it left out or changed.
pub struct Package {
    pub bytes: Vec<u8>,
    pub notices: Vec<String>,
}

const WGS84_WKT: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]"#;

pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// An SQL identifier, quoted.
fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn rgba(c: Rgba) -> String {
    format!("{},{},{},{}", c.r(), c.g(), c.b(), c.a())
}

/// Whether a character stays in a table or file name: letters and digits of any script with
/// their combining marks (Khmer's vowel signs and coeng among them), and `_`.
fn keep(c: char) -> bool {
    c == '_' || !(c.is_whitespace() || c.is_control() || c.is_ascii_punctuation() || matches!(c, '\u{2018}'..='\u{201F}'))
}

/// A table name for `name`: letters, digits and underscores (Khmer letters kept), unique.
fn table_name(name: &str, taken: &mut Vec<String>) -> String {
    let mut base: String = name.chars().map(|c| if keep(c) { c } else { '_' }).take(60).collect();
    if base.is_empty()
        || base.starts_with(|c: char| c.is_ascii_digit())
        || base.to_ascii_lowercase().starts_with("gpkg")
        || base.to_ascii_lowercase().starts_with("sqlite")
    {
        base = format!("layer_{base}");
    }
    let mut out = base.clone();
    let mut n = 1;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&out))
        || ["layer_styles", "qgis_projects"].contains(&out.to_ascii_lowercase().as_str())
    {
        n += 1;
        out = format!("{base}_{n}");
    }
    taken.push(out.clone());
    out
}

/// The GeoPackage geometry blob: its header (WGS 84, an envelope) and the WKB.
fn geometry_blob(g: &Geometry) -> Vec<u8> {
    let e = g.extent();
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(b"GP");
    out.push(0);
    out.push(0b0000_0011);
    out.extend_from_slice(&4326i32.to_le_bytes());
    for v in [e.min_x, e.max_x, e.min_y, e.max_y] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&rustgis_data::wkb::encode(g));
    out
}

// ---------------------------------------------------------------------------------------------
// Styles, as QGIS's XML
// ---------------------------------------------------------------------------------------------

fn opt(out: &mut String, name: &str, value: &str) {
    let _ = writeln!(out, r#"<Option type="QString" name="{}" value="{}"/>"#, esc(name), esc(value));
}

fn marker_name(m: Marker) -> &'static str {
    match m {
        Marker::Circle | Marker::Char(_) => "circle",
        Marker::Square => "square",
        Marker::Triangle => "triangle",
        Marker::Diamond => "diamond",
        Marker::Star => "star",
    }
}

/// A symbol, as QGIS's `<symbol>`, for `g` features. Widths and sizes stay in points.
fn symbol_xml(s: &Symbol, g: GeometryType, name: &str, opacity: f32) -> String {
    let mut out = String::new();
    let (ty, class) = match g {
        GeometryType::Point => ("marker", "SimpleMarker"),
        GeometryType::Polyline => ("line", "SimpleLine"),
        _ => ("fill", "SimpleFill"),
    };
    let _ = writeln!(out, r#"<symbol type="{ty}" name="{}" alpha="{opacity}" clip_to_extent="1" force_rhr="0">"#, esc(name));
    let _ = writeln!(out, r#"<layer class="{class}" enabled="1" locked="0" pass="0"><Option type="Map">"#);
    match g {
        GeometryType::Point => {
            opt(&mut out, "name", marker_name(s.marker));
            opt(&mut out, "color", &rgba(s.fill));
            opt(&mut out, "outline_color", &rgba(s.stroke));
            opt(&mut out, "outline_width", &format!("{}", s.stroke_width.max(0.0)));
            opt(&mut out, "outline_width_unit", "Point");
            opt(&mut out, "size", &format!("{}", s.size.max(0.0)));
            opt(&mut out, "size_unit", "Point");
            opt(&mut out, "outline_style", if s.stroke.a() == 0 || s.stroke_width <= 0.0 { "no" } else { "solid" });
        }
        GeometryType::Polyline => {
            opt(&mut out, "line_color", &rgba(s.stroke));
            opt(&mut out, "line_width", &format!("{}", s.stroke_width.max(0.0)));
            opt(&mut out, "line_width_unit", "Point");
            opt(&mut out, "line_style", "solid");
            opt(&mut out, "capstyle", "round");
            opt(&mut out, "joinstyle", "round");
        }
        _ => {
            opt(&mut out, "color", &rgba(s.fill));
            opt(&mut out, "style", if s.fill.a() == 0 { "no" } else { "solid" });
            opt(&mut out, "outline_color", &rgba(s.stroke));
            opt(&mut out, "outline_width", &format!("{}", s.stroke_width.max(0.0)));
            opt(&mut out, "outline_width_unit", "Point");
            opt(&mut out, "outline_style", if s.stroke.a() == 0 || s.stroke_width <= 0.0 { "no" } else { "solid" });
        }
    }
    out.push_str("</Option></layer>\n</symbol>\n");
    out
}

/// The layer's renderer as `<renderer-v2>`, with notes on what QGIS draws differently.
fn renderer_xml(l: &Layer, g: GeometryType, opacity: f32, notes: &mut Vec<String>) -> String {
    let mut out = String::new();
    match &l.renderer {
        Renderer::Single(s) => {
            out.push_str(r#"<renderer-v2 type="singleSymbol" symbollevels="0" forceraster="0" enableorderby="0"><symbols>"#);
            out.push_str(&symbol_xml(s, g, "0", opacity));
            out.push_str("</symbols></renderer-v2>\n");
        }
        Renderer::Unique { field, classes, other, class_labels, other_label, expression, .. } => {
            let attr = expression.clone().unwrap_or_else(|| field.clone());
            let _ = writeln!(
                out,
                r#"<renderer-v2 type="categorizedSymbol" attr="{}" symbollevels="0" forceraster="0" enableorderby="0"><categories>"#,
                esc(&attr)
            );
            for (i, (value, _)) in classes.iter().enumerate() {
                let label = class_labels.get(value).cloned().unwrap_or_else(|| value.clone());
                let _ =
                    writeln!(out, r#"<category render="true" symbol="{i}" value="{}" label="{}" type="string"/>"#, esc(value), esc(&label));
            }
            let other_label = other_label.clone().unwrap_or_default();
            let _ = writeln!(
                out,
                r#"<category render="true" symbol="{}" value="" label="{}" type="string"/>"#,
                classes.len(),
                esc(&other_label)
            );
            out.push_str("</categories><symbols>");
            for (i, (_, s)) in classes.iter().enumerate() {
                out.push_str(&symbol_xml(s, g, &i.to_string(), opacity));
            }
            out.push_str(&symbol_xml(other, g, &classes.len().to_string(), opacity));
            out.push_str("</symbols></renderer-v2>\n");
            if expression.is_some() {
                notes.push(format!(
                    "{}: its categories are by an expression; QGIS reads it as one of its own and may need it adjusted",
                    l.name
                ));
            }
        }
        Renderer::Graduated { field, breaks, symbols, min, class_labels, .. } => {
            let _ = writeln!(
                out,
                r#"<renderer-v2 type="graduatedSymbol" attr="{}" graduatedMethod="GraduatedColor" symbollevels="0" forceraster="0" enableorderby="0"><ranges>"#,
                esc(field)
            );
            let mut lower = *min;
            for (i, upper) in breaks.iter().enumerate() {
                let label = class_labels.get(&i).cloned().unwrap_or_else(|| format!("{lower} – {upper}"));
                let _ = writeln!(out, r#"<range render="true" symbol="{i}" lower="{lower}" upper="{upper}" label="{}"/>"#, esc(&label));
                lower = *upper;
            }
            out.push_str("</ranges><symbols>");
            for (i, s) in symbols.iter().enumerate().take(breaks.len()) {
                out.push_str(&symbol_xml(s, g, &i.to_string(), opacity));
            }
            out.push_str("</symbols></renderer-v2>\n");
        }
        Renderer::Proportional { field, base, min_size, max_size, min, max } => {
            // A single symbol whose size follows the field, as QGIS's size assistant writes it.
            let span = (max - min).abs().max(f64::MIN_POSITIVE);
            let expr =
                format!("coalesce(scale_linear(\"{}\", {min}, {max}, {min_size}, {max_size}), {min_size})", field.replace('"', "\"\""),);
            let _ = span;
            out.push_str(r#"<renderer-v2 type="singleSymbol" symbollevels="0" forceraster="0" enableorderby="0"><symbols>"#);
            let sym = symbol_xml(base, g, "0", opacity);
            // The size from the field, as a data-defined property of the marker.
            let dd = format!(
                r#"<data_defined_properties><Option type="Map"><Option type="QString" name="name" value=""/><Option type="Map" name="properties"><Option type="Map" name="size"><Option type="bool" name="active" value="true"/><Option type="QString" name="expression" value="{}"/><Option type="int" name="type" value="3"/></Option></Option><Option type="QString" name="type" value="collection"/></Option></data_defined_properties>"#,
                esc(&expr)
            );
            out.push_str(&sym.replacen("</Option></layer>", &format!("</Option>{dd}</layer>"), 1));
            out.push_str("</symbols></renderer-v2>\n");
        }
    }
    if !l.fill_layers.is_empty() || !l.line_strokes.is_empty() {
        notes.push(format!("{}: its stacked fills or strokes are saved as one simple symbol", l.name));
    }
    out
}

/// The layer's labels as `<labeling>` (its first visible label class), or nothing.
fn labeling_xml(l: &Layer) -> (bool, String) {
    let lab = &l.labels;
    let class = std::iter::once(&lab.default).chain(lab.classes.iter()).find(|c| c.visible && !c.field.is_empty());
    let Some(c) = class.filter(|_| lab.enabled) else { return (false, String::new()) };
    let mut out = String::new();
    let font = c.font.clone().unwrap_or_else(|| "Noto Sans Khmer".into());
    let weight = if c.bold { 75 } else { 50 };
    let _ = write!(
        out,
        r#"<labeling type="simple"><settings calloutType="simple"><text-style fieldName="{}" isExpression="0" fontFamily="{}" fontSize="{}" fontSizeUnit="Point" fontWeight="{weight}" fontItalic="{}" textColor="{}" textOpacity="1">"#,
        esc(&c.field),
        esc(&font),
        c.size,
        u8::from(c.italic),
        rgba(c.color)
    );
    let halo = c.halo_color.unwrap_or(Rgba::rgb(255, 255, 255));
    let _ = write!(
        out,
        r#"<text-buffer bufferDraw="{}" bufferSize="{}" bufferSizeUnits="Point" bufferColor="{}" bufferOpacity="1"/>"#,
        u8::from(c.halo),
        c.halo_width.unwrap_or(1.0),
        rgba(halo)
    );
    out.push_str(r#"</text-style><placement placement="1" dist="0" priority="5"/><rendering obstacle="1" scaleVisibility="0"/></settings></labeling>"#);
    out.push('\n');
    (true, out)
}

/// A layer's style alone, as a `.qml` document (`layer_styles.styleQML`).
fn qml(renderer: &str, labeling: &str, labels_on: bool, opacity: f32) -> String {
    format!(
        "<!DOCTYPE qgis PUBLIC 'http://mrcc.com/qgis.dtd' 'SYSTEM'>\n<qgis version=\"3.34.0\" styleCategories=\"Symbology|Labeling|Rendering\" labelsEnabled=\"{}\">\n{renderer}{labeling}<layerOpacity>{opacity}</layerOpacity>\n</qgis>\n",
        u8::from(labels_on)
    )
}

// ---------------------------------------------------------------------------------------------
// The package
// ---------------------------------------------------------------------------------------------

struct Saved {
    /// A raster's tiles (table): drawn as a picture of red, green, blue and alpha bands.
    raster: bool,
    id: String,
    name: String,
    table: String,
    geometry: &'static str,
    renderer: String,
    labeling: String,
    labels_on: bool,
    opacity: f32,
    visible: bool,
    min_scale: Option<f64>,
    max_scale: Option<f64>,
}

/// The QGIS project (`.qgs` text) for the saved layers and the map's web tile layers.
/// A saved layer's source, as QGIS names it: a table of the GeoPackage, relative to the project.
fn source(s: &Saved, file: &str) -> String {
    if s.raster { format!("GPKG:./{file}:{}", s.table) } else { format!("./{file}|layername={}", s.table) }
}

fn project_xml(map: &Map, title: &str, file: &str, saved: &[Saved], view: (f64, f64), layouts: &str) -> String {
    let mut x = String::new();
    let _ = writeln!(x, r#"<!DOCTYPE qgis PUBLIC 'http://mrcc.com/qgis.dtd' 'SYSTEM'>"#);
    let _ = writeln!(x, r#"<qgis projectname="{}" version="3.34.0-KHGIS">"#, esc(title));
    let _ = writeln!(x, "<title>{}</title>", esc(title));
    // QGIS reads the project's coordinate system only with projections on.
    x.push_str(r#"<properties><SpatialRefSys><ProjectionsEnabled type="int">1</ProjectionsEnabled></SpatialRefSys></properties>"#);
    x.push('\n');
    let _ = writeln!(
        x,
        r#"<projectCrs><spatialrefsys><proj4>+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs</proj4><srsid>3857</srsid><srid>3857</srid><authid>EPSG:3857</authid></spatialrefsys></projectCrs>"#
    );
    // Web tile layers that are plain XYZ rasters, in drawing order (index 0 on top).
    let tiles: Vec<(String, String, String)> = map
        .tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t.kind, rustgis_map::tiles::TileKind::Raster) && t.url.contains("{z}"))
        .map(|(i, t)| {
            let sub = t.subdomains.first().cloned().unwrap_or_else(|| "a".into());
            let url = t.url.replace("{s}", &sub);
            let enc: String = url
                .chars()
                .map(|c| match c {
                    '{' => "%7B".to_string(),
                    '}' => "%7D".to_string(),
                    '&' => "%26".to_string(),
                    '=' => "%3D".to_string(),
                    c => c.to_string(),
                })
                .collect();
            let source = format!("type=xyz&url={enc}&zmax={}&zmin={}", t.max_zoom, t.min_zoom);
            (format!("tiles_{:08x}_0000_4000_8000_{i:012x}", i + 1), t.name.clone(), source)
        })
        .collect();
    x.push_str("<layer-tree-group>\n");
    for s in saved {
        let _ = writeln!(
            x,
            r#"<layer-tree-layer id="{}" name="{}" source="{}" providerKey="{}" checked="{}" expanded="{}"/>"#,
            esc(&s.id),
            esc(&s.name),
            esc(&source(s, file)),
            if s.raster { "gdal" } else { "ogr" },
            if s.visible { "Qt::Checked" } else { "Qt::Unchecked" },
            u8::from(!s.raster)
        );
    }
    for (id, name, source) in &tiles {
        let _ = writeln!(
            x,
            r#"<layer-tree-layer id="{}" name="{}" source="{}" providerKey="wms" checked="Qt::Checked" expanded="0"/>"#,
            esc(id),
            esc(name),
            esc(source)
        );
    }
    x.push_str("</layer-tree-group>\n<projectlayers>\n");
    for s in saved.iter().filter(|s| s.raster) {
        let enh = |c: &str| {
            format!(
                "<{c}ContrastEnhancement><minValue>0</minValue><maxValue>255</maxValue><algorithm>NoEnhancement</algorithm></{c}ContrastEnhancement>"
            )
        };
        let _ = writeln!(
            x,
            r#"<maplayer type="raster"><id>{}</id><datasource>{}</datasource><layername>{}</layername><provider>gdal</provider><srs><spatialrefsys><proj4>+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs</proj4><srsid>3857</srsid><srid>3857</srid><authid>EPSG:3857</authid></spatialrefsys></srs><pipe><rasterrenderer type="multibandcolor" opacity="{}" alphaBand="4" redBand="1" greenBand="2" blueBand="3" nodataColor=""><rasterTransparency/>{}{}{}</rasterrenderer><brightnesscontrast brightness="0" contrast="0" gamma="1"/><huesaturation saturation="0" grayscaleMode="0" colorizeOn="0"/><rasterresampler maxOversampling="2" zoomedInResampler="bilinear" zoomedOutResampler="bilinear"/></pipe><layerOpacity>{}</layerOpacity></maplayer>"#,
            esc(&s.id),
            esc(&source(s, file)),
            esc(&s.name),
            s.opacity,
            enh("red"),
            enh("green"),
            enh("blue"),
            s.opacity
        );
    }
    for s in saved.iter().filter(|s| !s.raster) {
        let scale = match (s.min_scale, s.max_scale) {
            (None, None) => r#"hasScaleBasedVisibilityFlag="0""#.to_string(),
            (a, b) => format!(r#"hasScaleBasedVisibilityFlag="1" minScale="{}" maxScale="{}""#, a.unwrap_or(0.0), b.unwrap_or(0.0)),
        };
        let _ = writeln!(
            x,
            r#"<maplayer type="vector" geometry="{}" labelsEnabled="{}" {scale}><id>{}</id><datasource>./{}|layername={}</datasource><layername>{}</layername><provider encoding="UTF-8">ogr</provider><srs><spatialrefsys><proj4>+proj=longlat +datum=WGS84 +no_defs</proj4><srsid>3452</srsid><srid>4326</srid><authid>EPSG:4326</authid></spatialrefsys></srs>"#,
            s.geometry,
            u8::from(s.labels_on),
            esc(&s.id),
            esc(file),
            esc(&s.table),
            esc(&s.name)
        );
        x.push_str(&s.renderer);
        x.push_str(&s.labeling);
        let _ = writeln!(x, "<layerOpacity>{}</layerOpacity></maplayer>", s.opacity);
    }
    for (id, name, source) in &tiles {
        let _ = writeln!(
            x,
            r#"<maplayer type="raster"><id>{}</id><datasource>{}</datasource><layername>{}</layername><provider>wms</provider><srs><spatialrefsys><proj4>+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs</proj4><srsid>3857</srsid><srid>3857</srid><authid>EPSG:3857</authid></spatialrefsys></srs></maplayer>"#,
            esc(id),
            esc(source),
            esc(name)
        );
    }
    x.push_str("</projectlayers>\n");
    x.push_str(layouts);
    // The view: the map's centre and resolution over the window's size, in Web Mercator.
    let cam = map.camera;
    let (w, h) = (view.0.max(100.0), view.1.max(100.0));
    let (hw, hh) = (cam.res * w / 2.0, cam.res * h / 2.0);
    let _ = writeln!(
        x,
        r#"<mapcanvas name="theMapCanvas" annotationsVisible="1"><units>meters</units><extent><xmin>{}</xmin><ymin>{}</ymin><xmax>{}</xmax><ymax>{}</ymax></extent><rotation>0</rotation><destinationsrs><spatialrefsys><proj4>+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs</proj4><srsid>3857</srsid><srid>3857</srid><authid>EPSG:3857</authid></spatialrefsys></destinationsrs></mapcanvas>"#,
        cam.center.x - hw,
        cam.center.y - hh,
        cam.center.x + hw,
        cam.center.y + hh
    );
    x.push_str("</qgis>\n");
    x
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The active map of `session` as a GeoPackage named `file` (the project refers to its layers by
/// that name), with `title` for the QGIS project.
pub fn build(
    map: &Map,
    layouts: &[rustgis_map::layout::Layout],
    title: &str,
    file: &str,
    view: (f64, f64),
    now: &str,
) -> Result<Package, String> {
    let mut notices = Vec::new();
    let mut tables: Vec<Table> = Vec::new();
    let mut contents_rows: Vec<(i64, Vec<Sql>)> = Vec::new();
    let mut geom_rows: Vec<(i64, Vec<Sql>)> = Vec::new();
    let mut style_rows: Vec<(i64, Vec<Sql>)> = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let mut matrix_rows: Vec<(i64, Vec<Sql>)> = Vec::new();
    let mut saved: Vec<Saved> = Vec::new();
    let mut set_rows: Vec<(i64, Vec<Sql>)> = Vec::new();
    let mut tile_bytes = 0usize;
    for l in &map.layers {
        if l.is_raster() {
            let opacity = (1.0 - l.transparency / 100.0).clamp(0.0, 1.0);
            match raster_tiles(l, MAX_TILE_BYTES.saturating_sub(tile_bytes), &mut notices) {
                Ok(Some((tiles, zooms, bounds))) => {
                    let table = table_name(&l.name, &mut taken);
                    tile_bytes += tiles.iter().map(|t| t.3.len()).sum::<usize>();
                    let rows = tiles
                        .into_iter()
                        .map(|(z, x, y, png)| {
                            (0i64, vec![Sql::Null, Sql::Int(i64::from(z)), Sql::Int(i64::from(x)), Sql::Int(i64::from(y)), Sql::Blob(png)])
                        })
                        .enumerate()
                        .map(|(i, (_, v))| (i as i64 + 1, v))
                        .collect();
                    tables.push(Table {
                        name: table.clone(),
                        sql: format!("CREATE TABLE {} (id INTEGER PRIMARY KEY, zoom_level INTEGER NOT NULL, tile_column INTEGER NOT NULL, tile_row INTEGER NOT NULL, tile_data BLOB NOT NULL, UNIQUE (zoom_level, tile_column, tile_row))", ident(&table)),
                        rows,
                        unique: vec![vec![1, 2, 3]],
                    });
                    contents_rows.push((
                        contents_rows.len() as i64 + 1,
                        vec![
                            Sql::Text(table.clone()),
                            Sql::Text("tiles".into()),
                            Sql::Text(table.clone()),
                            Sql::Text(String::new()),
                            Sql::Text(now.into()),
                            Sql::Real(bounds[0]),
                            Sql::Real(bounds[1]),
                            Sql::Real(bounds[2]),
                            Sql::Real(bounds[3]),
                            Sql::Int(3857),
                        ],
                    ));
                    let world = std::f64::consts::PI * rustgis_geom::MERCATOR_R;
                    set_rows.push((
                        set_rows.len() as i64 + 1,
                        vec![
                            Sql::Text(table.clone()),
                            Sql::Int(3857),
                            Sql::Real(-world),
                            Sql::Real(-world),
                            Sql::Real(world),
                            Sql::Real(world),
                        ],
                    ));
                    for z in zooms.0..=zooms.1 {
                        let n = 1i64 << z;
                        let px = 2.0 * world / 256.0 / n as f64;
                        matrix_rows.push((
                            matrix_rows.len() as i64 + 1,
                            vec![
                                Sql::Text(table.clone()),
                                Sql::Int(i64::from(z)),
                                Sql::Int(n),
                                Sql::Int(n),
                                Sql::Int(256),
                                Sql::Int(256),
                                Sql::Real(px),
                                Sql::Real(px),
                            ],
                        ));
                    }
                    saved.push(Saved {
                        raster: true,
                        id: format!("{table}_{:08x}_0000_4000_8000_{:012x}", l.id, saved.len() + 1),
                        name: l.name.clone(),
                        table,
                        geometry: "",
                        renderer: String::new(),
                        labeling: String::new(),
                        labels_on: false,
                        opacity,
                        visible: l.visible,
                        min_scale: l.min_scale,
                        max_scale: l.max_scale,
                    });
                }
                Ok(None) => {}
                Err(e) => notices.push(format!("{}: left out ({e})", l.name)),
            }
            continue;
        }
        if l.is_streamed() {
            notices.push(format!(
                "{}: only the {} features KHGIS has read so far were saved (Load All first for every feature)",
                l.name,
                l.dataset.features.len()
            ));
        }
        let ds = &l.dataset;
        let table = table_name(&l.name, &mut taken);
        let g = ds.geometry_type;
        let has_geom = g != GeometryType::None;
        let geometry_type = match g {
            GeometryType::Point if ds.features.iter().all(|f| !matches!(f.geometry, Some(Geometry::MultiPoint(_)))) => "POINT",
            GeometryType::Point => "MULTIPOINT",
            GeometryType::Polyline => "MULTILINESTRING",
            GeometryType::Polygon => "MULTIPOLYGON",
            GeometryType::None => "",
        };
        // Columns: fid, geom, then the fields (renamed when they'd clash).
        let mut cols: Vec<String> = vec!["fid".into()];
        let mut sql = format!("CREATE TABLE {} (\"fid\" INTEGER PRIMARY KEY NOT NULL", ident(&table));
        if has_geom {
            sql.push_str(&format!(", \"geom\" {geometry_type}"));
            cols.push("geom".into());
        }
        for f in &ds.fields {
            let mut name = f.name.clone();
            while cols.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                name.push('_');
            }
            let ty = match f.ty {
                FieldType::Integer => "INTEGER",
                FieldType::Double => "REAL",
                FieldType::Text => "TEXT",
                FieldType::Boolean => "BOOLEAN",
            };
            sql.push_str(&format!(", {} {ty}", ident(&name)));
            cols.push(name);
        }
        sql.push(')');
        let mut rows = Vec::with_capacity(ds.features.len());
        let mut extent = rustgis_data::Extent::EMPTY;
        for (i, f) in ds.features.iter().enumerate() {
            let mut vals = vec![Sql::Null];
            if has_geom {
                vals.push(match &f.geometry {
                    Some(Geometry::Point(c)) if geometry_type == "MULTIPOINT" => {
                        let mp = Geometry::MultiPoint(vec![*c]);
                        extent = extent.union(&mp.extent());
                        Sql::Blob(geometry_blob(&mp))
                    }
                    Some(geom) => {
                        extent = extent.union(&geom.extent());
                        Sql::Blob(geometry_blob(geom))
                    }
                    None => Sql::Null,
                });
            }
            for (k, _) in ds.fields.iter().enumerate() {
                vals.push(match f.attrs.get(k) {
                    Some(Value::Int(v)) => Sql::Int(*v),
                    Some(Value::Float(v)) if v.is_finite() => Sql::Real(*v),
                    Some(Value::Text(t)) => Sql::Text(t.clone()),
                    Some(Value::Bool(b)) => Sql::Int(i64::from(*b)),
                    _ => Sql::Null,
                });
            }
            rows.push((i as i64 + 1, vals));
        }
        tables.push(Table { name: table.clone(), sql, rows, unique: Vec::new() });
        let (data_type, bounds) = if has_geom && !extent.is_empty() {
            (
                "features",
                [Sql::Real(extent.min_x), Sql::Real(extent.min_y), Sql::Real(extent.max_x), Sql::Real(extent.max_y), Sql::Int(4326)],
            )
        } else if has_geom {
            ("features", [Sql::Null, Sql::Null, Sql::Null, Sql::Null, Sql::Int(4326)])
        } else {
            ("attributes", [Sql::Null, Sql::Null, Sql::Null, Sql::Null, Sql::Null])
        };
        let mut row = vec![
            Sql::Text(table.clone()),
            Sql::Text(data_type.into()),
            Sql::Text(table.clone()),
            Sql::Text(String::new()),
            Sql::Text(now.into()),
        ];
        row.extend(bounds);
        contents_rows.push((contents_rows.len() as i64 + 1, row));
        if has_geom {
            geom_rows.push((
                geom_rows.len() as i64 + 1,
                vec![
                    Sql::Text(table.clone()),
                    Sql::Text("geom".into()),
                    Sql::Text(geometry_type.into()),
                    Sql::Int(4326),
                    Sql::Int(0),
                    Sql::Int(0),
                ],
            ));
        }
        // The style.
        let opacity = (1.0 - l.transparency / 100.0).clamp(0.0, 1.0);
        let (renderer, labeling, labels_on) = if has_geom {
            let r = renderer_xml(l, g, 1.0, &mut notices);
            let (on, lab) = labeling_xml(l);
            (r, lab, on)
        } else {
            (String::new(), String::new(), false)
        };
        if has_geom {
            style_rows.push((
                style_rows.len() as i64 + 1,
                vec![
                    Sql::Null,
                    Sql::Text(String::new()),
                    Sql::Text(String::new()),
                    Sql::Text(table.clone()),
                    Sql::Text("geom".into()),
                    Sql::Text(l.name.chars().take(30).collect()),
                    Sql::Text(qml(&renderer, &labeling, labels_on, opacity)),
                    Sql::Text(String::new()),
                    Sql::Int(1),
                    Sql::Text("Style from KHGIS".into()),
                    Sql::Text(String::new()),
                    Sql::Null,
                    Sql::Text(now.into()),
                ],
            ));
        }
        saved.push(Saved {
            raster: false,
            // QGIS keeps a layer's id only in its own form, the name and a UUID.
            id: format!("{table}_{:08x}_0000_4000_8000_{:012x}", l.id, saved.len() + 1),
            name: l.name.clone(),
            table,
            geometry: match g {
                GeometryType::Point => "Point",
                GeometryType::Polyline => "Line",
                GeometryType::Polygon => "Polygon",
                GeometryType::None => "No geometry",
            },
            renderer,
            labeling,
            labels_on,
            opacity,
            visible: l.visible,
            min_scale: l.min_scale,
            max_scale: l.max_scale,
        });
    }
    if saved.is_empty() {
        return Err("the map has no layers to save".into());
    }
    // layer_styles, registered as an attributes table as QGIS does.
    contents_rows.push((
        contents_rows.len() as i64 + 1,
        vec![
            Sql::Text("layer_styles".into()),
            Sql::Text("attributes".into()),
            Sql::Text("layer_styles".into()),
            Sql::Text(String::new()),
            Sql::Text(now.into()),
            Sql::Null,
            Sql::Null,
            Sql::Null,
            Sql::Null,
            Sql::Null,
        ],
    ));
    // The project, zipped as a .qgz and stored as hexadecimal text, as QGIS stores its own.
    let project_layers: Vec<crate::qgislayouts::ProjectLayer> = saved
        .iter()
        .map(|s| crate::qgislayouts::ProjectLayer {
            id: s.id.clone(),
            name: s.name.clone(),
            source: source(s, file),
            provider: if s.raster { "gdal" } else { "ogr" },
        })
        .collect();
    let layouts_xml = crate::qgislayouts::layouts_xml(map, layouts, &project_layers, &mut notices);
    let qgs = project_xml(map, title, file, &saved, view, &layouts_xml);
    let qgz = rustgis_data::zip::write(&[(format!("{}.qgs", title_file(title)), qgs.into_bytes())])?;
    let project_row = vec![
        Sql::Text(title.into()),
        Sql::Text(format!(r#"{{"last_modified_time": "{now}", "last_modified_user": "KHGIS" }}"#)),
        Sql::Text(hex(&qgz)),
    ];
    let mut all = vec![
        Table {
            name: "gpkg_spatial_ref_sys".into(),
            sql: "CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER NOT NULL PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT)".into(),
            rows: vec![
                (-1, vec![Sql::Text("Undefined cartesian SRS".into()), Sql::Null, Sql::Text("NONE".into()), Sql::Int(-1), Sql::Text("undefined".into()), Sql::Text("undefined cartesian coordinate reference system".into())]),
                (0, vec![Sql::Text("Undefined geographic SRS".into()), Sql::Null, Sql::Text("NONE".into()), Sql::Int(0), Sql::Text("undefined".into()), Sql::Text("undefined geographic coordinate reference system".into())]),
                (4326, vec![Sql::Text("WGS 84 geodetic".into()), Sql::Null, Sql::Text("EPSG".into()), Sql::Int(4326), Sql::Text(WGS84_WKT.into()), Sql::Text("longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid".into())]),
                (3857, vec![Sql::Text("WGS 84 / Pseudo-Mercator".into()), Sql::Null, Sql::Text("EPSG".into()), Sql::Int(3857), Sql::Text(MERCATOR_WKT.into()), Sql::Text("Web Mercator, as web map tiles are".into())]),
            ],
            unique: Vec::new(),
        },
        Table {
            name: "gpkg_contents".into(),
            sql: "CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE, description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER, CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id))".into(),
            rows: contents_rows,
            unique: vec![vec![0], vec![2]],
        },
        Table {
            name: "gpkg_geometry_columns".into(),
            sql: "CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name), CONSTRAINT uk_gc_table_name UNIQUE (table_name), CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name), CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id))".into(),
            rows: geom_rows,
            unique: vec![vec![0, 1], vec![0]],
        },
    ];
    if !set_rows.is_empty() {
        all.push(Table {
            name: "gpkg_tile_matrix_set".into(),
            sql: "CREATE TABLE gpkg_tile_matrix_set (table_name TEXT NOT NULL PRIMARY KEY, srs_id INTEGER NOT NULL, min_x DOUBLE NOT NULL, min_y DOUBLE NOT NULL, max_x DOUBLE NOT NULL, max_y DOUBLE NOT NULL, CONSTRAINT fk_gtms_table_name FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name), CONSTRAINT fk_gtms_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id))".into(),
            rows: set_rows,
            unique: vec![vec![0]],
        });
        all.push(Table {
            name: "gpkg_tile_matrix".into(),
            sql: "CREATE TABLE gpkg_tile_matrix (table_name TEXT NOT NULL, zoom_level INTEGER NOT NULL, matrix_width INTEGER NOT NULL, matrix_height INTEGER NOT NULL, tile_width INTEGER NOT NULL, tile_height INTEGER NOT NULL, pixel_x_size DOUBLE NOT NULL, pixel_y_size DOUBLE NOT NULL, CONSTRAINT pk_ttm PRIMARY KEY (table_name, zoom_level), CONSTRAINT fk_tmm_table_name FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name))".into(),
            rows: matrix_rows,
            unique: vec![vec![0, 1]],
        });
    }
    all.extend(tables);
    all.push(Table {
        name: "layer_styles".into(),
        sql: "CREATE TABLE \"layer_styles\" (\"id\" INTEGER PRIMARY KEY NOT NULL, \"f_table_catalog\" TEXT(256), \"f_table_schema\" TEXT(256), \"f_table_name\" TEXT(256), \"f_geometry_column\" TEXT(256), \"styleName\" TEXT(30), \"styleQML\" TEXT, \"styleSLD\" TEXT, \"useAsDefault\" BOOLEAN, \"description\" TEXT, \"owner\" TEXT(30), \"ui\" TEXT(30), \"update_time\" DATETIME DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')))".into(),
        rows: style_rows,
        unique: Vec::new(),
    });
    all.push(Table {
        name: "qgis_projects".into(),
        sql: "CREATE TABLE qgis_projects(name TEXT PRIMARY KEY, metadata BLOB, content BLOB)".into(),
        rows: vec![(1, project_row)],
        unique: vec![vec![0]],
    });
    let bytes = database(&all, Header { application_id: 0x4750_4B47, user_version: 10300 })?;
    Ok(Package { bytes, notices })
}

/// Now, as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso() -> String {
    let secs = (crate::project_log::now_ms() / 1000) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The active map of `s` as a GeoPackage for QGIS named `file`, with the project's layouts.
pub fn save(s: &crate::Session, file: &str) -> Result<Package, String> {
    build(s.map(), &s.project.layouts, &s.project.name, file, s.view_size, &now_iso())
}

/// The most bytes of raster tiles one GeoPackage takes.
const MAX_TILE_BYTES: usize = 400 << 20;
const MAX_TILES: u64 = 20_000;

const MERCATOR_WKT: &str = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs"],AUTHORITY["EPSG","3857"]]"#;

/// A raster layer's tiles: `(zoom, column, row, png)`, the zooms, and its bounds in Web Mercator;
/// none for a raster with nothing to draw. Too many tiles at its own resolution, it is saved a
/// level or more coarser, and a notice says so.
#[allow(clippy::type_complexity)]
fn raster_tiles(
    l: &Layer,
    room: usize,
    notices: &mut Vec<String>,
) -> Result<Option<(Vec<(u8, u32, u32, Vec<u8>)>, (u8, u8), [f64; 4])>, String> {
    let Some(rl) = l.raster.as_ref() else { return Ok(None) };
    let Some(r) = rl.data.as_deref() else { return Err("its pixels aren't loaded".into()) };
    let e = r.extent();
    let m = rustgis_geom::extent_to_merc(&e);
    let (min, max) = rustgis_render::raster_export::zooms(rl)?;
    let opacity = 1.0;
    let mut top = max;
    loop {
        match rustgis_render::raster_export::tiles(rl, min.min(top), top, opacity, MAX_TILES, room) {
            Ok(t) => {
                if top < max {
                    notices.push(format!("{}: saved {} level(s) coarser than its own pixels, to fit the GeoPackage", l.name, max - top));
                }
                return Ok(Some((t, (min.min(top), top), [m.min_x, m.min_y, m.max_x, m.max_y])));
            }
            Err(e) if e.contains("exceeds") && top > min => top -= 1,
            Err(e) if e.contains("no visible pixels") => return Ok(None),
            Err(e) => return Err(e),
        }
    }
}

/// A file-name-safe version of the project's title.
pub fn title_file(title: &str) -> String {
    let s: String = title.chars().map(|c| if keep(c) || c == '-' { c } else { '_' }).take(80).collect();
    if s.is_empty() { "project".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_map_saves_as_a_geopackage_with_a_project() {
        let s = crate::Session::new(crate::sample::world_project());
        let p = build(s.map(), &[], "World Atlas", "atlas.gpkg", (1200.0, 800.0), "2026-11-26T00:00:00Z").unwrap();
        assert_eq!(&p.bytes[..16], b"SQLite format 3\0");
        assert_eq!(&p.bytes[68..72], b"GPKG");
        if let Some(out) = std::env::var_os("KHGIS_GPKG_OUT") {
            std::fs::write(out, &p.bytes).unwrap();
        }
    }

    /// A map with a raster and a layout: the raster as GeoPackage tiles, the layout in the project.
    #[test]
    fn rasters_and_layouts_travel_to_qgis() {
        use serde_json::json;
        let dir = std::env::temp_dir().join(format!("khgis-qgispkg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = crate::Session::new(crate::Project::default());
        let rows: String = (0..40).map(|r| (0..40).map(|c| ((r + c) % 7).to_string()).collect::<Vec<_>>().join(" ") + "\n").collect();
        let path = dir.join("classes.asc");
        std::fs::write(&path, format!("ncols 40\nnrows 40\nxllcorner 105.3\nyllcorner 12.2\ncellsize 0.001\n{rows}")).unwrap();
        s.run("layer.addData", &json!({"path": path})).unwrap();
        s.run("layer.template", &json!({"template": "polygons", "name": "AOI"})).unwrap();
        s.run("edit.enable", &json!({"on": true})).unwrap();
        s.run("edit.addFeature", &json!({"layer": "AOI", "geometry": {"type": "Polygon", "coordinates": [[[105.3, 12.2], [105.34, 12.2], [105.34, 12.24], [105.3, 12.2]]]}})).unwrap();
        s.run("layout.new", &json!({"name": "Report", "template": "blank"})).unwrap();
        s.run("layout.addFrame", &json!({"x": 10, "y": 10, "w": 180, "h": 150})).unwrap();
        s.run("layout.addElement", &json!({"kind": "text", "text": "Land cover", "x": 200, "y": 10, "w": 80, "h": 10})).unwrap();
        s.run("layout.addElement", &json!({"kind": "legend", "x": 200, "y": 30, "w": 80, "h": 60})).unwrap();
        s.run("layout.addElement", &json!({"kind": "scaleBar", "x": 20, "y": 150, "w": 60, "h": 8})).unwrap();
        s.run("layout.addElement", &json!({"kind": "northArrow", "x": 170, "y": 15, "w": 10, "h": 14})).unwrap();
        let p = save(&s, "test.gpkg").unwrap();
        assert!(!p.notices.iter().any(|n| n.contains("left out")), "{:?}", p.notices);
        // The tiles' table and its matrix are in the file; the project is zipped inside it.
        let text = String::from_utf8_lossy(&p.bytes);
        assert!(text.contains("gpkg_tile_matrix_set") && text.contains("tile_data"));
        if let Some(out) = std::env::var_os("KHGIS_GPKG_OUT") {
            std::fs::write(out, &p.bytes).unwrap();
        }
        let q = project_xml(
            s.map(),
            "t",
            "test.gpkg",
            &[],
            (800.0, 600.0),
            &crate::qgislayouts::layouts_xml(s.map(), &s.project.layouts, &[], &mut Vec::new()),
        );
        assert!(q.contains(r#"<Layout name="Report""#), "{q}");
    }

    #[test]
    fn names_and_text_are_made_safe() {
        let mut taken = vec![];
        assert_eq!(table_name("ព្រំដែន ជាតិ", &mut taken), "ព្រំដែន_ជាតិ");
        assert_eq!(table_name("ព្រំដែន ជាតិ", &mut taken), "ព្រំដែន_ជាតិ_2");
        assert_eq!(table_name("gpkg_x", &mut taken), "layer_gpkg_x");
        assert_eq!(table_name("", &mut taken), "layer_");
        assert_eq!(esc("a<b&\"c\""), "a&lt;b&amp;&quot;c&quot;");
        assert_eq!(now_iso().len(), 20);
    }
}
