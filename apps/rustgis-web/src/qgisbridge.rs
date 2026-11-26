//! QGIS algorithms in the browser: a KHGIS QGIS bridge service (`khgis_qgis_bridge.py serve`,
//! packaged in `packaging/qgis-bridge`) runs them in a real QGIS and sends the files back.
//!
//! Connecting reads the service's manifests (`GET /manifests`) into the toolbox; a run posts the
//! job with its staged input files (`POST /run`) and lands the output files it gets back.

use std::sync::Mutex;

use base64::Engine as _;
use rustgis_ui_egui::Slot;
use rustgis_ui_egui::rustgis_engine::cmd::gp::{QgisOutput, QgisRun};
use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
async function call(url, init) {
  const r = await fetch(url, init);
  const text = await r.text();
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}: ${text.slice(0, 300)}`);
  return text;
}
export async function qgis_get(url) {
  return call(url, { method: "GET", mode: "cors", credentials: "omit" });
}
export async function qgis_post(url, body) {
  return call(url, { method: "POST", mode: "cors", credentials: "omit", headers: { "Content-Type": "application/json" }, body });
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn qgis_get(url: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn qgis_post(url: &str, body: &str) -> Result<JsValue, JsValue>;
}

/// The service connected last, for runs.
static SERVICE: Mutex<String> = Mutex::new(String::new());

fn js_error(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => String::from(err.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    }
}

/// Connects the service at `url`: its algorithms join the toolbox. The slot gets the bridge's
/// status (QGIS version, algorithm count, plugins) or the error.
pub fn connect(url: String, slot: Slot<Result<Value, String>>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let r = async {
            if !(url.starts_with("https://") || url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1")) {
                return Err("the QGIS bridge service must be an https:// address (or http://localhost)".to_string());
            }
            let text = qgis_get(&format!("{url}/manifests")).await.map_err(|e| format!("{url}: {}", js_error(&e)))?;
            let text = text.as_string().unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<Value>(&text)
                && let Some(e) = v.get("error")
            {
                return Err(e.as_str().unwrap_or("the service refused").to_string());
            }
            rustgis_qgis::set_manifests(&text)?;
            if let Ok(mut s) = SERVICE.lock() {
                *s = url.clone();
            }
            Ok(rustgis_qgis::status())
        }
        .await;
        if let Ok(mut s) = slot.lock() {
            *s = Some(r);
        }
        ctx.request_repaint();
    });
}

/// Runs one algorithm on the service and puts its new files and answer (or its error) in `slot`.
pub fn run(job: QgisRun, slot: Slot<QgisOutput>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let r = async {
            job.can_start()?;
            let url = SERVICE.lock().map(|s| s.clone()).unwrap_or_default();
            if url.is_empty() {
                return Err("no QGIS bridge service is connected (Plugins › QGIS Plugins & Tools)".to_string());
            }
            let b64 = base64::engine::general_purpose::STANDARD;
            let files: Vec<Value> = job.files.iter().map(|(n, b)| json!({"name": n, "data": b64.encode(b)})).collect();
            let body = serde_json::to_string(&json!({"job": job.request, "files": files})).map_err(|e| e.to_string())?;
            let text = qgis_post(&format!("{url}/run"), &body).await.map_err(|e| format!("{url}: {}", js_error(&e)))?;
            let v: Value = serde_json::from_str(&text.as_string().unwrap_or_default()).map_err(|e| format!("the service's answer: {e}"))?;
            if let Some(e) = v.get("error") {
                return Err(e.as_str().map_or_else(|| e.to_string(), str::to_string));
            }
            let answer = v.get("ok").cloned().unwrap_or(Value::Null);
            let mut out = Vec::new();
            for f in v.get("files").and_then(Value::as_array).into_iter().flatten() {
                let (Some(name), Some(data)) = (f.get("name").and_then(Value::as_str), f.get("data").and_then(Value::as_str)) else {
                    continue;
                };
                let bytes = b64.decode(data).map_err(|e| format!("{name}: {e}"))?;
                out.push((name.to_string(), bytes));
            }
            Ok((out, answer, Vec::new()))
        }
        .await;
        if let Ok(mut s) = slot.lock() {
            *s = Some(r);
        }
        ctx.request_repaint();
    });
}
