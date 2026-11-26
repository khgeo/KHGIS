//! The QGIS-style layout: its menus, toolbars and docks, switching to and from the ribbon, and
//! the preference that keeps the choice.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable;
use rustgis_ui_egui::{RustGisApp, Services};
use serde_json::json;

type Store = Arc<Mutex<HashMap<String, String>>>;

fn services(store: &Store, qgis_default: bool) -> Services {
    let (load, save) = (store.clone(), store.clone());
    Services {
        load_pref: Some(Box::new(move |k| load.lock().unwrap().get(k).cloned())),
        save_pref: Some(Box::new(move |k, v| {
            save.lock().unwrap().insert(k.to_string(), v.to_string());
        })),
        qgis_layout_default: qgis_default,
        ..Default::default()
    }
}

/// The app in English (KHGIS starts in Khmer), with the layout drawn.
fn harness(mut app: RustGisApp) -> egui_kittest::Harness<'static, RustGisApp> {
    app.run("ui.language", json!({"code": "en"})).unwrap();
    egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 860.0)).with_max_steps(64).build_ui_state(
        |ui, app: &mut RustGisApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

#[test]
fn the_apps_open_in_the_qgis_layout_and_keep_the_users_choice() {
    let store = Store::default();
    let app = RustGisApp::with_sample().with_services(services(&store, true));
    assert!(app.ui.qgis_layout, "the QGIS layout is the apps' default");
    let mut h = harness(app);
    h.run();
    // The menu bar, the docks and the Processing Toolbox.
    for menu in ["Project", "Edit", "View", "Layer", "Settings", "Plugins", "Vector", "Raster", "Processing", "Help"] {
        assert!(h.query_all_by_label(menu).next().is_some(), "the {menu} menu");
    }
    assert!(h.query_all_by_label("Browser").next().is_some() && h.query_all_by_label("Layers").next().is_some());
    assert!(h.query_all_by_label("Open Attribute Table").next().is_some(), "the Attributes toolbar");

    h.state_mut().run("ui.interface", json!({"style": "ribbon"})).unwrap();
    h.run();
    assert!(!h.state().ui.qgis_layout);
    assert_eq!(store.lock().unwrap().get("interface_style").map(String::as_str), Some("ribbon"));
    // A new start keeps the ribbon the user picked.
    let again = RustGisApp::with_sample().with_services(services(&store, true));
    assert!(!again.ui.qgis_layout);
    assert!(h.state_mut().run("ui.interface", json!({"style": "office"})).is_err());
    h.state_mut().run("ui.interface", json!(null)).unwrap();
    assert!(h.state().ui.qgis_layout, "no style switches to the other one");
}

#[test]
fn tests_and_embedders_keep_the_ribbon_unless_asked() {
    let store = Store::default();
    let app = RustGisApp::with_sample().with_services(services(&store, false));
    assert!(!app.ui.qgis_layout);
}

#[test]
fn docks_and_toolbars_show_and_hide() {
    let store = Store::default();
    let mut h = harness(RustGisApp::with_sample().with_services(services(&store, true)));
    h.run();
    // Layers and Browser are docked together, each shown or hidden alone.
    h.state_mut().run("ui.pane.catalog", json!(null)).unwrap();
    h.run();
    assert!(!h.state().ui.qgis.browser && h.state().ui.qgis.layers);
    h.state_mut().run("ui.pane.contents", json!(null)).unwrap();
    h.run();
    assert!(!h.state().ui.qgis.layers);
    // The Data Source Manager takes the left side; Layers brings the docks back.
    h.state_mut().run("ui.pane.sources", json!(null)).unwrap();
    h.run();
    h.state_mut().run("ui.pane.contents", json!(null)).unwrap();
    h.run();
    assert!(h.state().ui.qgis.layers && h.state().ui.left.is_none());

    h.state_mut().run("ui.qgisToolbar", json!({"id": "digitizing"})).unwrap();
    h.run();
    assert!(h.query_all_by_label("Add Point Feature").next().is_none());
    assert!(h.query_all_by_label("Open Attribute Table").next().is_some(), "the other toolbars stay");
    assert_eq!(store.lock().unwrap().get("qgis_toolbars_hidden").map(String::as_str), Some("digitizing"));
    h.state_mut().run("ui.qgisToolbar", json!({"id": "digitizing"})).unwrap();
    assert!(h.state_mut().run("ui.qgisToolbar", json!({"id": "nope"})).is_err());
    // The ribbon under the toolbars, for every KHGIS tool.
    h.state_mut().run("ui.qgisToolbar", json!({"id": "ribbon"})).unwrap();
    h.run();
    assert!(h.state().ui.qgis.ribbon);
    // Options and the QGIS window open and draw in every language KHGIS has a catalog for.
    h.state_mut().run("ui.qgisOptions", json!(null)).unwrap();
    h.state_mut().run("ui.qgisBridge", json!(null)).unwrap();
    for code in ["km", "en"] {
        h.state_mut().run("ui.language", json!({"code": code})).unwrap();
        h.run();
    }
    assert!(
        h.query_all_by_label("QGIS isn't connected.").next().is_some() || h.query_all_by_label("QGIS Plugins & Tools").next().is_some()
    );
}
