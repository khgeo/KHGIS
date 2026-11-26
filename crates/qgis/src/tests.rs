use super::*;
use rustgis_data::{Feature, Field, FieldType};

/// Five algorithms as QGIS 3.34's bridge lists them.
const FIXTURE: &str = include_str!("fixture.json");

struct Fixture {
    points: Dataset,
}

impl Fixture {
    /// A point in Phnom Penh.
    fn new() -> Fixture {
        let mut points = Dataset::new("pts", rustgis_data::GeometryType::Point, vec![Field::new("id", FieldType::Integer)]);
        points.features.push(Feature {
            id: 1,
            geometry: Some(Geometry::Point(Coord::new(104.92, 11.56))),
            attrs: vec![rustgis_data::Value::Int(1)],
        });
        Fixture { points }
    }
}

impl Inputs for Fixture {
    fn dataset(&self, layer: &str) -> Option<&Dataset> {
        (layer == "pts").then_some(&self.points)
    }
    fn selection(&self, _: &str) -> &[usize] {
        &[]
    }
}

/// The tests share the one manifest list: they take turns.
static TURN: Mutex<()> = Mutex::new(());

fn loaded() -> Vec<&'static ToolSpec> {
    register();
    set_manifests(FIXTURE).unwrap();
    rustgis_analysis::external_tools_of(SOURCE).iter().collect()
}

#[test]
fn manifests_become_tools_with_forms() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let tools = loaded();
    assert_eq!(tools.len(), 5);
    let buffer = tools.iter().find(|t| t.id == "qgis:native:buffer").unwrap();
    assert_eq!(buffer.toolbox, "QGIS › QGIS (native c++) › Vector geometry");
    let names: Vec<&str> = buffer.params.iter().map(|p| p.name).collect();
    // Outputs aren't in the form; QGIS's upper-case names start lower case.
    assert_eq!(names, ["iNPUT", "dISTANCE", "sEGMENTS", "eND_CAP_STYLE", "jOIN_STYLE", "mITER_LIMIT", "dISSOLVE", "sEPARATE_DISJOINT"]);
    let cap = buffer.params.iter().find(|p| p.name == "eND_CAP_STYLE").unwrap();
    assert!(matches!(cap.kind, ParamKind::Choice(c) if c == ["Round", "Flat", "Square"]));
    assert_eq!(cap.default, "\"Round\"");
    assert!(matches!(buffer.params[0].kind, ParamKind::VectorLayer));
    assert_eq!(buffer.params[1].default, "10");
    let slope = tools.iter().find(|t| t.id == "qgis:gdal:slope").unwrap();
    assert!(matches!(slope.params[0].kind, ParamKind::RasterLayer));
    let merge = tools.iter().find(|t| t.id == "qgis:native:mergevectorlayers").unwrap();
    assert!(matches!(merge.params[0].kind, ParamKind::VectorLayers));
}

#[test]
fn a_job_stages_layers_in_utm_and_names_outputs() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let tools = loaded();
    let buffer = tools.iter().find(|t| t.id == "qgis:native:buffer").unwrap();
    let params = json!({"iNPUT": "pts", "dISTANCE": "1000", "sEGMENTS": "8", "eND_CAP_STYLE": "Square", "dISSOLVE": "true"});
    let job = prepare(buffer, &Fixture::new(), &params).unwrap();
    assert_eq!(job.files.len(), 1);
    let staged: Value = serde_json::from_slice(&job.files[0].1).unwrap();
    // Phnom Penh is in UTM zone 48 north.
    assert_eq!(staged["crs"]["properties"]["name"], "urn:ogc:def:crs:EPSG::32648");
    let r = job.request("/tmp/w");
    assert_eq!(r["alg"], "native:buffer");
    assert_eq!(r["params"]["INPUT"], format!("/tmp/w/{}", job.files[0].0));
    assert_eq!(r["params"]["DISTANCE"], json!(1000.0));
    assert_eq!(r["params"]["SEGMENTS"], json!(8));
    assert_eq!(r["params"]["END_CAP_STYLE"], json!(2));
    assert_eq!(r["params"]["DISSOLVE"], json!(true));
    assert_eq!(r["vector_outputs"]["OUTPUT"], "/tmp/w/out-OUTPUT.geojson");
    assert!(prepare(buffer, &Fixture::new(), &json!({"iNPUT": "nowhere"})).unwrap_err().contains("nowhere"));
}

#[test]
fn finished_runs_come_back_as_layers_and_messages() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let tools = loaded();
    let buffer = tools.iter().find(|t| t.id == "qgis:native:buffer").unwrap();
    let job = prepare(buffer, &Fixture::new(), &json!({"iNPUT": "pts", "dISTANCE": "5"})).unwrap();
    let out = br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":1},"geometry":{"type":"Polygon","coordinates":[[[104.9,11.5],[105.0,11.5],[105.0,11.6],[104.9,11.5]]]}}]}"#;
    let r = finish(
        &job,
        &[("out-OUTPUT.geojson".into(), out.to_vec())],
        &json!({"results": {"OUTPUT": "/tmp/w/out-OUTPUT.geojson", "COUNT": 3}, "notes": []}),
        &["Processing feature 1".into()],
    );
    let Output::Layers(layers) = r.output else { panic!("layers expected") };
    assert!(matches!(&layers[..], [OutputLayer::Vector(ds)] if ds.name == "Buffer" && ds.features.len() == 1));
    assert!(r.messages.iter().any(|m| m == "COUNT: 3"));
    assert!(!r.messages.iter().any(|m| m.contains("out-OUTPUT")));
}

#[test]
fn bad_manifests_are_refused_or_skipped() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    assert!(set_manifests("not json").is_err());
    assert!(set_manifests("{}").is_err());
    // An algorithm without an id, and junk parameters, are skipped rather than crashing.
    let n = set_manifests(r#"{"algorithms":[{"name":"x"},{"id":"a:b","params":[1,{"name":5},{"name":"X","type":"enum","options":[]}]}]}"#)
        .unwrap();
    assert_eq!(n, 2);
    let tools = rustgis_analysis::external_tools_of(SOURCE);
    assert_eq!(tools.len(), 1);
    assert!(matches!(tools[0].params[0].kind, ParamKind::Text));
}

/// A real QGIS, end to end: connect, then buffer a point in Phnom Penh by 1 km. Skipped unless
/// `KHGIS_QGIS_PYTHON` names a Python with PyQGIS.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_real_qgis_runs_an_algorithm() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(python) = std::env::var_os(runner::ENV).filter(|p| !p.is_empty()) else {
        eprintln!("skipped: set {} to QGIS's Python", runner::ENV);
        return;
    };
    // Connecting caches the manifests in the app-data folder: set RUSTGIS_APP_DATA_DIR to keep
    // the test's apart from yours.
    register();
    runner::choose(Some(std::path::PathBuf::from(python)));
    let status = runner::connect().unwrap();
    assert!(status["algorithms"].as_u64().unwrap() > 100, "{status}");
    let buffer = rustgis_analysis::external_tools_of(SOURCE).iter().find(|t| t.id == "qgis:native:buffer").unwrap();
    let r = rustgis_analysis::run(buffer.id, &Fixture::new(), &json!({"iNPUT": "pts", "dISTANCE": 1000, "sEGMENTS": 16})).unwrap();
    let Output::Layers(layers) = r.output else { panic!("layers expected: {:?}", r.messages) };
    let [OutputLayer::Vector(ds)] = &layers[..] else { panic!("one vector layer expected: {:?}", r.messages) };
    // A 1 km circle around the point, back in longitude/latitude: about 0.018° across.
    let e = ds.extent();
    assert!((e.width() - 0.0184).abs() < 0.002 && (e.center().x - 104.92).abs() < 0.001, "{e:?}");
    clear_manifests();
}
