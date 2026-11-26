//! The Feature Attributes form, as QGIS opens one when a feature is drawn: a box for each field,
//! filled in before the feature is added. OK adds the feature with its values (one step to undo);
//! Cancel leaves it out. Settings › "Suppress attribute form after feature creation" adds features
//! at once, with empty values, as before.

use egui::{Color32, Stroke};
use rustgis_engine::data::FieldType;
use rustgis_engine::geom::Coord;
use serde_json::{Map, Value, json};

use crate::RustGisApp;

/// The preference that turns the form off.
pub const SUPPRESS_PREF: &str = "feature_form_suppress";
/// The preference that fills the form with the values entered last.
pub const REUSE_PREF: &str = "feature_form_reuse";

/// A feature drawn and waiting for its values.
pub struct Pending {
    /// The layer it goes in, by name.
    layer: String,
    /// `edit.addFeature`'s parameters, without the attributes.
    params: Value,
    /// The fields, their types and the text in each box.
    fields: Vec<(String, String, FieldType)>,
    values: Vec<String>,
    /// Its outline on the map, while the form is open.
    pub points: Vec<Coord>,
    pub closed: bool,
    /// The first box takes the keyboard once.
    focus: bool,
    error: Option<String>,
}

#[derive(Default)]
pub struct State {
    pub pending: Option<Pending>,
    /// Features are added at once, with no form.
    pub suppress: bool,
    /// The boxes start with the values entered last (per layer).
    pub reuse: bool,
    last: std::collections::HashMap<(String, String), String>,
}

/// Reads the form's preferences.
pub fn load(app: &mut RustGisApp) {
    let load = |k: &str| app.services.load_pref.as_ref().and_then(|l| l(k));
    app.ui.qgis.form.suppress = load(SUPPRESS_PREF).as_deref() == Some("on");
    app.ui.qgis.form.reuse = load(REUSE_PREF).as_deref() == Some("on");
}

fn save(app: &RustGisApp, key: &str, on: bool) {
    if let Some(s) = &app.services.save_pref {
        s(key, if on { "on" } else { "off" });
    }
}

/// Adds a feature drawn on the map: with the form first in the QGIS layout, when its layer has
/// fields to fill in; at once otherwise.
pub fn add(app: &mut RustGisApp, params: Value, points: Vec<Coord>, closed: bool) {
    let layer = app.session.active_layer.and_then(|id| app.session.map().layer(id)).map(|l| {
        let fields: Vec<(String, String, FieldType)> =
            l.dataset.fields.iter().map(|f| (f.name.clone(), f.alias.clone().unwrap_or_else(|| f.name.clone()), f.ty)).collect();
        (l.name.clone(), fields, l.dataset.features.len())
    });
    let Some((name, fields, _)) = layer.filter(|l| !l.1.is_empty()) else {
        let _ = app.run("edit.addFeature", params);
        return;
    };
    // A feature template carries its own values; a form already open is finished first (with what
    // it has), as a second feature is drawn.
    if !app.ui.qgis_layout || app.ui.qgis.form.suppress || params.get("template").is_some() {
        let _ = app.run("edit.addFeature", params);
        return;
    }
    if app.ui.qgis.form.pending.is_some() {
        ok(app);
        if app.ui.qgis.form.pending.is_some() {
            // Its values didn't fit its fields: it stays, and the new feature is added plainly.
            let _ = app.run("edit.addFeature", params);
            return;
        }
    }
    let form = &app.ui.qgis.form;
    let values = fields
        .iter()
        .map(|f| if form.reuse { form.last.get(&(name.clone(), f.0.clone())).cloned().unwrap_or_default() } else { String::new() })
        .collect();
    app.ui.qgis.form.pending = Some(Pending { layer: name, params, fields, values, points, closed, focus: true, error: None });
}

/// The parameters with the values typed, checked against the fields' types.
fn with_values(p: &Pending) -> Result<Value, String> {
    let mut attrs = Map::new();
    for ((name, label, ty), text) in p.fields.iter().zip(&p.values) {
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        let v = match ty {
            FieldType::Integer => {
                json!(t.parse::<i64>().map_err(|_| crate::i18n::fmt("{field} takes a whole number", &[("field", label)]))?)
            }
            FieldType::Double => {
                json!(t.replace(',', ".").parse::<f64>().map_err(|_| crate::i18n::fmt("{field} takes a number", &[("field", label)]))?)
            }
            FieldType::Boolean => json!(matches!(t, "true" | "1" | "yes")),
            FieldType::Text => json!(text),
        };
        attrs.insert(name.clone(), v);
    }
    let mut params = p.params.clone();
    params["layer"] = json!(p.layer);
    params["attributes"] = Value::Object(attrs);
    Ok(params)
}

/// OK: adds the feature with its values.
fn ok(app: &mut RustGisApp) {
    let Some(p) = app.ui.qgis.form.pending.as_mut() else { return };
    let params = match with_values(p) {
        Ok(v) => v,
        Err(e) => {
            p.error = Some(e);
            return;
        }
    };
    let Some(p) = app.ui.qgis.form.pending.take() else { return };
    for ((name, ..), v) in p.fields.iter().zip(&p.values) {
        app.ui.qgis.form.last.insert((p.layer.clone(), name.clone()), v.clone());
    }
    if let Err(e) = app.run("edit.addFeature", params) {
        app.session.notify(e, true);
    }
    app.map.invalidate();
}

/// The values a text field already has in the layer, for picking one again (as a class name).
fn used_values(app: &RustGisApp, layer: &str, field: &str) -> Vec<String> {
    let Some(l) = app.session.map().layer_by_name(layer) else { return Vec::new() };
    let Some(i) = l.dataset.field_index(field) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for f in l.dataset.features.iter().rev() {
        if let Some(v) = f.attrs.get(i) {
            let s = v.to_string();
            if !s.is_empty() && s != "NULL" && !out.contains(&s) {
                out.push(s);
                if out.len() >= 40 {
                    break;
                }
            }
        }
    }
    out.sort();
    out
}

/// The form's window.
pub fn window(app: &mut RustGisApp, ctx: &egui::Context) {
    let Some(p) = app.ui.qgis.form.pending.as_ref() else { return };
    let layer = p.layer.clone();
    let choices: Vec<Vec<String>> =
        p.fields.iter().map(|f| if f.2 == FieldType::Text { used_values(app, &layer, &f.0) } else { Vec::new() }).collect();
    let mut open = true;
    let (mut accept, mut cancel) = (false, false);
    let title = crate::i18n::fmt("{layer} — Feature Attributes", &[("layer", &layer)]);
    let reuse_before = app.ui.qgis.form.reuse;
    egui::Window::new(title)
        .id(egui::Id::unique("feature_form"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let form = &mut app.ui.qgis.form;
            let Some(p) = form.pending.as_mut() else { return };
            egui::Grid::new("feature_form_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                for (k, ((_, label, ty), value)) in p.fields.iter().zip(p.values.iter_mut()).enumerate() {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        if *ty == FieldType::Boolean {
                            let mut on = matches!(value.trim(), "true" | "1" | "yes");
                            if ui.checkbox(&mut on, "").changed() {
                                *value = if on { "true" } else { "false" }.into();
                            }
                        } else {
                            let hint = match ty {
                                FieldType::Integer => "NULL (whole number)",
                                FieldType::Double => "NULL (number)",
                                _ => "NULL",
                            };
                            let r = ui.add(egui::TextEdit::singleline(value).desired_width(200.0).hint_text(tl!(hint)));
                            if k == 0 && p.focus {
                                r.request_focus();
                                p.focus = false;
                            }
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                accept = true;
                            }
                            let used = &choices[k];
                            if !used.is_empty() {
                                ui.menu_button("▾", |ui| {
                                    for u in used {
                                        if ui.button(u).clicked() {
                                            value.clone_from(u);
                                            ui.close();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(tl!("Values already in this field"));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            if let Some(e) = &p.error {
                ui.colored_label(crate::theme::Tokens::get(ui.ctx()).red, e);
            }
            ui.checkbox(&mut form.reuse, tl!("Remember the last values"));
            ui.separator();
            ui.horizontal(|ui| {
                accept |= ui.button(egui::RichText::new(tl!("OK")).strong()).clicked();
                cancel |= ui.button(tl!("Cancel")).clicked();
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
    if app.ui.qgis.form.reuse != reuse_before {
        save(app, REUSE_PREF, app.ui.qgis.form.reuse);
    }
    if accept {
        ok(app);
    } else if cancel || !open {
        app.ui.qgis.form.pending = None;
        app.map.invalidate();
    }
}

/// The feature waiting for its values, outlined on the map.
pub fn draw(app: &RustGisApp, p: &egui::Painter, rect: egui::Rect) {
    let Some(f) = &app.ui.qgis.form.pending else { return };
    let screen: Vec<egui::Pos2> = f.points.iter().filter_map(|c| crate::mapview::lonlat_to_screen(app, rect, *c)).collect();
    let colour = Color32::from_rgb(0x16, 0x9A, 0x5A);
    match screen.len() {
        0 => {}
        1 => {
            p.circle_filled(screen[0], 5.0, colour);
            p.circle_stroke(screen[0], 5.0, Stroke::new(1.5, Color32::WHITE));
        }
        _ if f.closed && screen.len() >= 3 => {
            p.add(egui::Shape::closed_line(screen.clone(), Stroke::new(2.0, colour)));
            for s in &screen {
                p.circle_filled(*s, 3.0, colour);
            }
        }
        _ => {
            p.add(egui::Shape::line(screen, Stroke::new(2.0, colour)));
        }
    }
}

/// Settings › "Suppress attribute form after feature creation".
pub fn set_suppress(app: &mut RustGisApp, on: bool) {
    app.ui.qgis.form.suppress = on;
    save(app, SUPPRESS_PREF, on);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> RustGisApp {
        let mut app = RustGisApp::new(rustgis_engine::Session::new(rustgis_engine::Project::default()));
        app.ui.qgis_layout = true;
        app.run("layer.template", json!({"template": "polygons", "name": "training"})).unwrap();
        app.run("edit.enable", json!({"on": true})).unwrap();
        app
    }

    fn square() -> (Value, Vec<Coord>) {
        let c = [[104.0, 11.0], [104.01, 11.0], [104.01, 11.01], [104.0, 11.0]];
        (json!({"geometry": {"type": "Polygon", "coordinates": [c]}}), c.iter().map(|p| Coord::new(p[0], p[1])).collect())
    }

    /// A polygon drawn waits for its class; OK adds it with the value typed, Cancel leaves it out.
    #[test]
    fn a_drawn_feature_waits_for_its_values() {
        let mut app = app();
        let count = |app: &RustGisApp| app.session.map().layer_by_name("training").unwrap().dataset.features.len();
        let (p, pts) = square();
        add(&mut app, p.clone(), pts.clone(), true);
        assert_eq!(count(&app), 0, "nothing added before OK");
        let form = app.ui.qgis.form.pending.as_mut().unwrap();
        let k = form.fields.iter().position(|f| f.0 == "name").unwrap();
        form.values[k] = "Rice".into();
        ok(&mut app);
        assert!(app.ui.qgis.form.pending.is_none());
        let l = app.session.map().layer_by_name("training").unwrap();
        assert_eq!(l.dataset.features.len(), 1);
        let i = l.dataset.field_index("name").unwrap();
        assert_eq!(l.dataset.features[0].attrs[i].to_string(), "Rice");
        assert_eq!(used_values(&app, "training", "name"), vec!["Rice".to_string()]);
        // Cancel: nothing added.
        add(&mut app, p.clone(), pts.clone(), true);
        app.ui.qgis.form.pending = None;
        assert_eq!(count(&app), 1);
        // Suppressed: added at once.
        app.ui.qgis.form.suppress = true;
        add(&mut app, p, pts, true);
        assert_eq!(count(&app), 2);
    }

    #[test]
    fn numbers_are_checked_before_adding() {
        let p = Pending {
            layer: "x".into(),
            params: json!({}),
            fields: vec![("n".into(), "n".into(), FieldType::Integer), ("v".into(), "v".into(), FieldType::Double)],
            values: vec!["12".into(), "1,5".into()],
            points: Vec::new(),
            closed: false,
            focus: false,
            error: None,
        };
        let v = with_values(&p).unwrap();
        assert_eq!(v["attributes"], json!({"n": 12, "v": 1.5}));
        let bad = Pending { values: vec!["twelve".into(), String::new()], ..p };
        assert!(with_values(&bad).is_err());
    }
}
