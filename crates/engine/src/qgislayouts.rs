//! KHGIS's print layouts as QGIS print layouts, for the project a GeoPackage for QGIS keeps
//! (`qgispkg`): page, map frames (extent, grid, extent indicator), text, legend, scale bar, north
//! arrow, pictures, rectangles, ellipses and lines, each where it was on the page, so the layout
//! opens in QGIS's Layout Manager ready to print.
//!
//! What QGIS has no matching item for here (tables, charts, colourbars) is named in a notice, and a
//! frame showing another map than the one saved is left out the same way.

use std::fmt::Write as _;

use rustgis_geom::{Coord, lonlat_to_merc};
use rustgis_map::layout::{ElementKind, LineDir, LineHead, TextAlign};
use rustgis_map::{Map, Rgba, layout::Layout};

use crate::qgispkg::esc;

/// A layer of the saved project, as a legend lists it: QGIS's id, its name, source and provider.
pub struct ProjectLayer {
    pub id: String,
    pub name: String,
    pub source: String,
    pub provider: &'static str,
}

const MERCATOR: &str = r#"<crs><spatialrefsys><proj4>+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs</proj4><srsid>3857</srsid><srid>3857</srid><authid>EPSG:3857</authid></spatialrefsys></crs>"#;
const WGS84: &str = r#"<crs><spatialrefsys><proj4>+proj=longlat +datum=WGS84 +no_defs</proj4><srsid>3452</srsid><srid>4326</srid><authid>EPSG:4326</authid></spatialrefsys></crs>"#;

fn colour(c: Rgba) -> String {
    format!("{},{},{},{}", c.r(), c.g(), c.b(), c.a())
}

fn colour_el(tag: &str, c: Rgba) -> String {
    format!(r#"<{tag} red="{}" green="{}" blue="{}" alpha="{}"/>"#, c.r(), c.g(), c.b(), c.a())
}

/// A fill symbol: its colour and outline (width in millimetres).
fn fill_symbol(fill: Rgba, outline: Rgba, width_mm: f64) -> String {
    format!(
        r#"<symbol type="fill" name="" alpha="1" clip_to_extent="1" force_rhr="0"><layer class="SimpleFill" enabled="1" locked="0" pass="0"><Option type="Map"><Option type="QString" name="color" value="{}"/><Option type="QString" name="style" value="{}"/><Option type="QString" name="outline_color" value="{}"/><Option type="QString" name="outline_style" value="{}"/><Option type="QString" name="outline_width" value="{}"/><Option type="QString" name="outline_width_unit" value="MM"/><Option type="QString" name="joinstyle" value="miter"/></Option></layer></symbol>"#,
        colour(fill),
        if fill.a() == 0 { "no" } else { "solid" },
        colour(outline),
        if outline.a() == 0 || width_mm <= 0.0 { "no" } else { "solid" },
        width_mm.max(0.0)
    )
}

fn line_symbol(c: Rgba, width_mm: f64) -> String {
    format!(
        r#"<symbol type="line" name="" alpha="1" clip_to_extent="1" force_rhr="0"><layer class="SimpleLine" enabled="1" locked="0" pass="0"><Option type="Map"><Option type="QString" name="line_color" value="{}"/><Option type="QString" name="line_style" value="solid"/><Option type="QString" name="line_width" value="{}"/><Option type="QString" name="line_width_unit" value="MM"/><Option type="QString" name="capstyle" value="square"/><Option type="QString" name="joinstyle" value="bevel"/></Option></layer></symbol>"#,
        colour(c),
        width_mm.max(0.0)
    )
}

/// A text format: the family, size in points, colour, bold and italic.
fn text_style(family: &str, size: f64, c: Rgba, bold: bool, italic: bool) -> String {
    format!(
        r#"<text-style fontFamily="{}" fontSize="{}" fontSizeUnit="Point" fontWeight="{}" fontItalic="{}" textColor="{}" textOpacity="1" namedStyle="" fontLetterSpacing="0" fontWordSpacing="0" fontUnderline="0" fontStrikeout="0" blendMode="0" multilineHeight="1" multilineHeightUnit="Percentage" previewBkgrdColor="255,255,255,255" allowHtml="0"><text-buffer bufferDraw="0" bufferSize="1" bufferSizeUnits="MM" bufferColor="255,255,255,255" bufferOpacity="1" bufferJoinStyle="128" bufferNoFill="1" bufferBlendMode="0"/><background shapeDraw="0"/><shadow shadowDraw="0"/></text-style>"#,
        esc(family),
        size,
        if bold { 75 } else { 50 },
        u8::from(italic),
        colour(c)
    )
}

/// The attributes and children every layout item has, opening its element.
fn item_open(out: &mut String, ty: u32, uuid: &str, name: &str, (x, y, w, h): (f64, f64, f64, f64), visible: bool, z: usize, extra: &str) {
    let _ = write!(
        out,
        r#"<LayoutItem type="{ty}" uuid="{uuid}" templateUuid="{uuid}" id="{}" position="{x},{y},mm" positionOnPage="{x},{y},mm" size="{w},{h},mm" itemRotation="0" referencePoint="0" zValue="{z}" visibility="{}" opacity="1" frame="false" background="{}" positionLock="false" outlineWidthM="0.3,mm" frameJoinStyle="miter" blendMode="0" excludeFromExports="0" groupUuid="" {extra}>"#,
        esc(name),
        u8::from(visible),
        ty == 65638
    );
    out.push_str(r#"<FrameColor red="0" green="0" blue="0" alpha="255"/><BackgroundColor red="255" green="255" blue="255" alpha="255"/><LayoutObject><dataDefinedProperties><Option type="Map"><Option type="QString" name="name" value=""/><Option name="properties"/><Option type="QString" name="type" value="collection"/></Option></dataDefinedProperties><customproperties><Option/></customproperties></LayoutObject>"#);
}

fn uuid(layout: usize, element: u32) -> String {
    format!("{{{:08x}-4b47-4c41-8000-{:012x}}}", 0x6b68_0000 + layout as u32, element)
}

/// The extent a frame shows, in Web Mercator: its centre at its paper scale (as KHGIS draws it,
/// the scale true at the centre).
fn frame_extent(center: Coord, scale: f64, w: f64, h: f64) -> [f64; 4] {
    let c = lonlat_to_merc(center);
    let per = center.y.to_radians().cos().max(1e-6);
    // Map units per millimetre of paper.
    let res = scale / 1000.0 / per;
    let (hw, hh) = (res * w / 2.0, res * h / 2.0);
    [c.x - hw, c.y - hh, c.x + hw, c.y + hh]
}

/// The QGIS layouts (`<Layouts>`) for `layouts` whose frames show `map`, and notices for what
/// didn't travel.
pub fn layouts_xml(map: &Map, layouts: &[Layout], layers: &[ProjectLayer], notes: &mut Vec<String>) -> String {
    let mut out = String::from("<Layouts>\n");
    for (li, l) in layouts.iter().enumerate() {
        let mut left_out: Vec<&str> = Vec::new();
        let _ = write!(out, r#"<Layout name="{}" units="mm" worldFileMap="" printResolution="300">"#, esc(&l.name));
        out.push_str(r#"<Snapper tolerance="5" snapToItems="1" snapToGuides="1" snapToGrid="0"/><Grid resUnits="mm" resolution="10" offsetX="0" offsetY="0" offsetUnits="mm"/>"#);
        out.push_str("<PageCollection>");
        item_open(&mut out, 65638, &uuid(li, 0), "", (0.0, 0.0, l.width, l.height), true, 0, "");
        out.push_str(&fill_symbol(Rgba::new(255, 255, 255, 255), Rgba::new(0, 0, 0, 0), 0.0));
        out.push_str("</LayoutItem><GuideCollection visible=\"1\"/></PageCollection>\n");
        for (z, e) in l.elements.iter().enumerate() {
            let id = uuid(li, e.id);
            let place = (e.x, e.y, e.w, e.h);
            let z = z + 1;
            let frame_uuid = |f: u32| uuid(li, f);
            match &e.kind {
                ElementKind::MapFrame { map: shown, center, scale, grid, extent_of, extent_color, extent_width, bearing, .. } => {
                    if *shown != map.name {
                        left_out.push("a frame of another map");
                        continue;
                    }
                    let [x0, y0, x1, y1] = frame_extent(*center, *scale, e.w, e.h);
                    item_open(
                        &mut out,
                        65639,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(
                            r#"keepLayerSet="false" drawCanvasItems="true" mapRotation="{}" followPreset="false" followPresetName="" isTemporal="0" labelMargin="0,mm" mapFlags="1""#,
                            -bearing
                        ),
                    );
                    let _ = write!(out, r#"<Extent xmin="{x0}" ymin="{y0}" xmax="{x1}" ymax="{y1}"/>{MERCATOR}<LayerSet/>"#);
                    if let Some(g) = grid {
                        let step =
                            if g.step > 0.0 { g.step } else { nice_step((x1 - x0) / 111_320.0 * (center.y.to_radians().cos()) / 4.0) };
                        let style = match g.style {
                            rustgis_map::layout::GridStyle::Lines => 0,
                            _ => 1,
                        };
                        let _ = write!(
                            out,
                            r#"<ComposerMapGrid show="1" name="Grid 1" uuid="{{{}-grid}}" position="3" intervalX="{step}" intervalY="{step}" offsetX="0" offsetY="0" crossLength="3" gridStyle="{style}" gridFrameStyle="0" gridFrameWidth="2" gridFramePenThickness="0.3" gridFramePenColor="0,0,0,255" frameFillColor1="255,255,255,255" frameFillColor2="0,0,0,255" leftFrameDivisions="0" rightFrameDivisions="0" topFrameDivisions="0" bottomFrameDivisions="0" leftFramePosition="0" rightFramePosition="0" topFramePosition="0" bottomFramePosition="0" unit="0" blendMode="0" showAnnotation="{}" annotationFormat="{}" annotationPrecision="2" leftAnnotationDisplay="0" rightAnnotationDisplay="0" topAnnotationDisplay="0" bottomAnnotationDisplay="0" leftAnnotationPosition="0" rightAnnotationPosition="0" topAnnotationPosition="0" bottomAnnotationPosition="0" leftAnnotationDirection="0" rightAnnotationDirection="0" topAnnotationDirection="0" bottomAnnotationDirection="0" frameAnnotationDistance="1.5" annotationFontColor="{}">{WGS84}{}{}</ComposerMapGrid>"#,
                            id.trim_matches(['{', '}']),
                            u8::from(g.labels),
                            if g.hemisphere { 3 } else { 0 },
                            colour(g.color),
                            line_symbol(g.color, g.width * 0.3528),
                            text_style("Sans", 6.0, g.color, false, false)
                        );
                    }
                    if let Some(of) = extent_of {
                        let _ = write!(
                            out,
                            r#"<ComposerMapOverview show="1" name="Overview 1" uuid="{{{}-overview}}" frameMap="{}" blendMode="0" inverted="0" centered="0">{}</ComposerMapOverview>"#,
                            id.trim_matches(['{', '}']),
                            frame_uuid(*of),
                            fill_symbol(Rgba::new(0, 0, 0, 0), *extent_color, extent_width * 0.3528)
                        );
                    }
                    out.push_str("<AtlasMap atlasDriven=\"0\" scalingMode=\"2\" margin=\"0.1\"/><labelBlockingItems/></LayoutItem>\n");
                }
                ElementKind::Text { text, size, color, bold, italic, align, font_name, .. } => {
                    let halign = match align {
                        TextAlign::Left => 1,
                        TextAlign::Center => 4,
                        TextAlign::Right => 2,
                    };
                    let family = font_name.clone().unwrap_or_else(|| "Noto Sans".into());
                    item_open(
                        &mut out,
                        65641,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(r#"labelText="{}" htmlState="0" marginX="0" marginY="0" halign="{halign}" valign="32""#, esc(text)),
                    );
                    out.push_str(&text_style(&family, *size, *color, *bold, *italic));
                    let _ = write!(
                        out,
                        r#"<LabelFont description="{},{},-1,5,{},{},0,0,0,0" style=""/>{}</LayoutItem>"#,
                        esc(&family),
                        size,
                        if *bold { 75 } else { 50 },
                        u8::from(*italic),
                        colour_el("FontColor", *color)
                    );
                    out.push('\n');
                }
                ElementKind::Legend { frame, title, exclude, columns, all_classes, .. } => {
                    item_open(
                        &mut out,
                        65642,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(
                            r#"title="{}" map_uuid="{}" legendFilterByMap="{}" columnCount="{}" splitLayer="0" equalColumnWidth="0" symbolWidth="7" symbolHeight="4" maxSymbolSize="0" minSymbolSize="0" wmsLegendWidth="50" wmsLegendHeight="25" lineSpacing="1" boxSpace="2" columnSpace="2" titleAlignment="1" resizeToContents="0" rasterBorder="1" rasterBorderColor="0,0,0,255" rasterBorderWidth="0""#,
                            esc(title),
                            frame_uuid(*frame),
                            u8::from(!all_classes),
                            (*columns).max(1)
                        ),
                    );
                    out.push_str(r#"<styles><style name="title" marginBottom="3.5" alignment="1">"#);
                    out.push_str(&text_style("Noto Sans", 11.0, Rgba::new(0, 0, 0, 255), true, false));
                    out.push_str(r#"</style><style name="group" marginTop="3" alignment="1">"#);
                    out.push_str(&text_style("Noto Sans", 9.0, Rgba::new(0, 0, 0, 255), true, false));
                    out.push_str(r#"</style><style name="subgroup" marginTop="3" alignment="1">"#);
                    out.push_str(&text_style("Noto Sans", 8.0, Rgba::new(0, 0, 0, 255), false, false));
                    out.push_str(r#"</style><style name="symbol" marginTop="2.5" alignment="1"/><style name="symbolLabel" marginTop="2" marginLeft="2" alignment="1">"#);
                    out.push_str(&text_style("Noto Sans", 8.0, Rgba::new(0, 0, 0, 255), false, false));
                    out.push_str("</style></styles>");
                    // Layers left out: a legend of its own (others follow the project's layers).
                    if !exclude.is_empty() {
                        out.push_str(r#"<layer-tree-group><customproperties><Option/></customproperties>"#);
                        for pl in layers.iter().filter(|pl| !exclude.contains(&pl.name)) {
                            let _ = write!(
                                out,
                                r#"<layer-tree-layer id="{}" name="{}" source="{}" providerKey="{}" checked="Qt::Checked" expanded="1"><customproperties><Option/></customproperties></layer-tree-layer>"#,
                                esc(&pl.id),
                                esc(&pl.name),
                                esc(&pl.source),
                                pl.provider
                            );
                        }
                        out.push_str("</layer-tree-group>");
                    }
                    out.push_str("</LayoutItem>\n");
                }
                ElementKind::ScaleBar { frame, units, divisions, color, fill, .. } => {
                    use rustgis_map::layout::ScaleUnits as U;
                    let (unit, label, per) = match units {
                        U::Meters => ("m", "m", 1.0),
                        U::Miles => ("mi", "mi", 1609.344),
                        U::Feet => ("ft", "ft", 0.3048),
                        _ => ("km", "km", 1000.0),
                    };
                    item_open(
                        &mut out,
                        65646,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(
                            r#"mapUuid="{}" style="Single Box" unitType="{unit}" unitLabel="{label}" numMapUnitsPerScaleBarUnit="{per}" numSegments="{}" numSegmentsLeft="0" numSubdivisions="1" subdivisionsHeight="1.5" numUnitsPerSegment="1" segmentSizeMode="1" minBarWidth="{}" maxBarWidth="{}" height="2.5" labelBarSpace="2" boxContentSpace="1" alignment="0" labelVerticalPlacement="0" labelHorizontalPlacement="0" lineJoinStyle="miter" lineCapStyle="square""#,
                            frame_uuid(*frame),
                            (*divisions).clamp(1, 10),
                            (e.w * 0.5).max(10.0),
                            (e.w - 4.0).max(12.0)
                        ),
                    );
                    out.push_str(&text_style("Noto Sans", 7.0, *color, false, false));
                    let _ = write!(
                        out,
                        r#"<lineSymbol>{}</lineSymbol><divisionLineSymbol>{}</divisionLineSymbol><subdivisionLineSymbol>{}</subdivisionLineSymbol><fillSymbol1>{}</fillSymbol1><fillSymbol2>{}</fillSymbol2></LayoutItem>"#,
                        line_symbol(*color, 0.3),
                        line_symbol(*color, 0.3),
                        line_symbol(*color, 0.3),
                        fill_symbol(*color, Rgba::new(0, 0, 0, 0), 0.0),
                        fill_symbol(*fill, Rgba::new(0, 0, 0, 0), 0.0)
                    );
                    out.push('\n');
                }
                ElementKind::NorthArrow { frame, fill, outline, .. } => {
                    item_open(
                        &mut out,
                        65640,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(
                            r#"file=":/images/north_arrows/layout_default_north_arrow.svg" pictureWidth="{}" pictureHeight="{}" resizeMode="0" anchorPoint="0" svgFillColor="{}" svgBorderColor="{}" svgBorderWidth="0.2" mode="0" northMode="0" northOffset="0" pictureRotation="0" mapUuid="{}""#,
                            e.w,
                            e.h,
                            colour(*fill),
                            colour(*outline),
                            frame_uuid(*frame)
                        ),
                    );
                    out.push_str("</LayoutItem>\n");
                }
                ElementKind::Picture { image } => {
                    // Raster pictures (format 1), from the data itself.
                    let mode = 1;
                    item_open(
                        &mut out,
                        65640,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(
                            r#"file="base64:{}" pictureWidth="{}" pictureHeight="{}" resizeMode="1" anchorPoint="0" mode="{mode}" northMode="0" northOffset="0" pictureRotation="0" mapUuid="""#,
                            esc(image),
                            e.w,
                            e.h
                        ),
                    );
                    out.push_str("</LayoutItem>\n");
                }
                ElementKind::Rectangle { fill, outline, width, radius } => {
                    item_open(
                        &mut out,
                        65643,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(r#"shapeType="1" cornerRadiusMeasure="{radius},mm""#),
                    );
                    out.push_str(&fill_symbol(*fill, *outline, width * 0.3528));
                    out.push_str("</LayoutItem>\n");
                }
                ElementKind::Ellipse { fill, outline, width } => {
                    item_open(&mut out, 65643, &id, &e.name, place, e.visible, z, r#"shapeType="0" cornerRadiusMeasure="0,mm""#);
                    out.push_str(&fill_symbol(*fill, *outline, width * 0.3528));
                    out.push_str("</LayoutItem>\n");
                }
                ElementKind::Line { color, width, dir, head } => {
                    let (a, b) = match dir {
                        LineDir::Horizontal => ((0.0, e.h / 2.0), (e.w, e.h / 2.0)),
                        LineDir::Vertical => ((e.w / 2.0, 0.0), (e.w / 2.0, e.h)),
                        LineDir::Falling => ((0.0, 0.0), (e.w, e.h)),
                        _ => ((0.0, e.h), (e.w, 0.0)),
                    };
                    let (start, end) = match head {
                        LineHead::None => (0, 0),
                        LineHead::End => (0, 1),
                        LineHead::Start => (1, 0),
                        _ => (1, 1),
                    };
                    item_open(
                        &mut out,
                        65645,
                        &id,
                        &e.name,
                        place,
                        e.visible,
                        z,
                        &format!(
                            r#"startMarkerMode="{start}" endMarkerMode="{end}" arrowHeadWidth="{}" arrowHeadFillColor="{}" arrowHeadOutlineColor="{}" outlineWidth="0""#,
                            (width * 0.3528 * 4.0).max(2.0),
                            colour(*color),
                            colour(*color)
                        ),
                    );
                    let _ = write!(
                        out,
                        r#"{}<nodes><node x="{}" y="{}"/><node x="{}" y="{}"/></nodes></LayoutItem>"#,
                        line_symbol(*color, width * 0.3528),
                        a.0,
                        a.1,
                        b.0,
                        b.1
                    );
                    out.push('\n');
                }
                ElementKind::Table { .. } => left_out.push("tables"),
                ElementKind::Chart { .. } => left_out.push("charts"),
                ElementKind::Colorbar { .. } => left_out.push("colourbars"),
            }
        }
        out.push_str("</Layout>\n");
        left_out.sort_unstable();
        left_out.dedup();
        if !left_out.is_empty() {
            notes.push(format!(
                "Layout {}: QGIS has it without its {} (export the layout as PDF from KHGIS for those)",
                l.name,
                left_out.join(", ")
            ));
        }
    }
    out.push_str("</Layouts>\n");
    out
}

/// A round grid step (1, 2 or 5 × a power of ten) near `v` degrees.
fn nice_step(v: f64) -> f64 {
    if !v.is_finite() || v <= 0.0 {
        return 1.0;
    }
    let p = 10f64.powf(v.log10().floor());
    let m = v / p;
    p * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_becomes_a_qgis_layout() {
        let map = Map::new("Map");
        let mut l = Layout::new("Report", 297.0, 210.0);
        let f = l.add(
            "Map 1",
            (10.0, 10.0, 180.0, 150.0),
            ElementKind::MapFrame {
                map: "Map".into(),
                center: Coord::new(105.0, 12.0),
                scale: 150_000.0,
                shape: Default::default(),
                grid: None,
                extent_of: None,
                extent_color: Rgba::new(255, 0, 0, 255),
                extent_width: 2.0,
                bearing: 0.0,
                pitch: 0.0,
            },
        );
        l.add("Title", (200.0, 10.0, 80.0, 10.0), ElementKind::text("Land cover <2025>", 16.0, Rgba::new(0, 0, 0, 255), true));
        l.add("Scale", (20.0, 150.0, 60.0, 8.0), ElementKind::scale_bar(f));
        l.add("North", (170.0, 15.0, 10.0, 14.0), ElementKind::north_arrow(f));
        l.add("Table", (200.0, 100.0, 80.0, 40.0), ElementKind::Table { frame: f, layer: "x".into(), rows: 5 });
        let mut notes = Vec::new();
        let x = layouts_xml(&map, &[l], &[], &mut notes);
        assert!(x.contains(r#"<Layout name="Report""#));
        assert!(x.contains(r#"type="65638""#) && x.contains(r#"size="297,210,mm""#));
        assert!(x.contains(r#"type="65639""#) && x.contains("<Extent xmin="));
        assert!(x.contains(r#"labelText="Land cover &lt;2025&gt;""#));
        assert!(x.contains(r#"type="65646""#) && x.contains(r#"type="65640""#));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("tables"));
        // The frame's extent: 180 mm at 1:150,000 is 27 km on the ground at its centre.
        let [x0, _, x1, _] = frame_extent(Coord::new(105.0, 12.0), 150_000.0, 180.0, 150.0);
        assert!(((x1 - x0) * 12f64.to_radians().cos() - 27_000.0).abs() < 1.0);
        assert_eq!(nice_step(0.07), 0.05);
    }
}
