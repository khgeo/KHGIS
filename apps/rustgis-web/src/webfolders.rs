//! Local folders in the Browser panel (Chrome, Edge: the File System Access API). The user
//! picks a folder once; the browser keeps its handle (in IndexedDB) so it is listed again on the
//! next visit, and asks again for leave to read it when it is opened. Files are read as a pick
//! reads them (`web::read_together`), a Shapefile's parts taken from beside its `.shp`.

use rustgis_ui_egui::{Inbox, Slot};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::parts::Notes;

#[wasm_bindgen(inline_js = r#"
const DB = "khgis-folders";
function db() {
  return new Promise((res, rej) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore("h", { keyPath: "id" });
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function all() {
  const d = await db();
  return new Promise((res, rej) => {
    const q = d.transaction("h").objectStore("h").getAll();
    q.onsuccess = () => res(q.result);
    q.onerror = () => rej(q.error);
  });
}
async function write(f) {
  const d = await db();
  return new Promise((res, rej) => {
    const tx = d.transaction("h", "readwrite");
    f(tx.objectStore("h"));
    tx.oncomplete = () => res();
    tx.onerror = () => rej(tx.error);
  });
}
const pairs = (list) => list.map((r) => [r.id, r.handle.name]);
export function khgis_folders_supported() {
  return typeof window.showDirectoryPicker === "function" && window.self === window.top;
}
export async function khgis_folders_known() {
  try { return pairs(await all()); } catch (e) { return []; }
}
export async function khgis_folder_connect() {
  const h = await window.showDirectoryPicker({ id: "khgis-browser", mode: "read" });
  const list = await all();
  for (const r of list) {
    if (await r.handle.isSameEntry(h)) return pairs(list);
  }
  const id = list.reduce((m, r) => Math.max(m, r.id), 0) + 1;
  await write((s) => s.put({ id, handle: h }));
  return [...pairs(list), [id, h.name]];
}
async function dirOf(id, path) {
  const r = (await all()).find((r) => r.id === id);
  if (!r) throw new Error("this folder is no longer connected");
  let h = r.handle;
  let p = await h.queryPermission({ mode: "read" });
  if (p !== "granted") p = await h.requestPermission({ mode: "read" });
  if (p !== "granted") throw new Error("the browser wasn't allowed to read this folder: open it again to allow it");
  for (const s of path.split("/").filter(Boolean)) h = await h.getDirectoryHandle(s);
  return h;
}
export async function khgis_folder_list(id, path) {
  const h = await dirOf(id, path);
  const out = [];
  for await (const [n, e] of h.entries()) {
    if (n.startsWith(".")) continue;
    out.push([n, e.kind === "directory"]);
    if (out.length >= 5000) break;
  }
  return out;
}
export async function khgis_folder_files(id, path, name) {
  const h = await dirOf(id, path);
  const files = [await (await h.getFileHandle(name)).getFile()];
  const lower = name.toLowerCase();
  if (lower.endsWith(".shp")) {
    const base = lower.slice(0, -4);
    for await (const [n, e] of h.entries()) {
      const l = n.toLowerCase();
      if (e.kind === "file" && l !== lower && l.startsWith(base + ".") && ["dbf", "shx", "prj", "cpg"].includes(l.slice(base.length + 1))) {
        files.push(await e.getFile());
      }
    }
  }
  return files;
}
export async function khgis_folder_forget(id) {
  await write((s) => s.delete(id));
}
"#)]
extern "C" {
    fn khgis_folders_supported() -> bool;
    async fn khgis_folders_known() -> JsValue;
    #[wasm_bindgen(catch)]
    async fn khgis_folder_connect() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn khgis_folder_list(id: u32, path: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn khgis_folder_files(id: u32, path: &str, name: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn khgis_folder_forget(id: u32) -> Result<JsValue, JsValue>;
}

fn js_error(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => String::from(err.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    }
}

/// `[[id, name], …]` as pairs.
fn folders(v: &JsValue) -> Vec<(u32, String)> {
    let Some(a) = v.dyn_ref::<js_sys::Array>() else { return Vec::new() };
    a.iter()
        .filter_map(|p| {
            let p = p.dyn_into::<js_sys::Array>().ok()?;
            Some((p.get(0).as_f64()? as u32, p.get(1).as_string()?))
        })
        .collect()
}

pub fn supported() -> bool {
    khgis_folders_supported()
}

fn put<T: 'static>(slot: &Slot<T>, v: T, ctx: &egui::Context) {
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
    ctx.request_repaint();
}

pub fn known(slot: Slot<Result<Vec<(u32, String)>, String>>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let v = khgis_folders_known().await;
        put(&slot, Ok(folders(&v)), &ctx);
    });
}

pub fn connect(slot: Slot<Result<Vec<(u32, String)>, String>>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let r = match khgis_folder_connect().await {
            Ok(v) => Ok(folders(&v)),
            // Cancelled: the folders as they were.
            Err(e) if js_error(&e).contains("abort") || format!("{e:?}").contains("AbortError") => {
                Ok(folders(&khgis_folders_known().await))
            }
            Err(e) => Err(js_error(&e)),
        };
        put(&slot, r, &ctx);
    });
}

pub fn list(id: u32, path: String, slot: Slot<Result<Vec<(String, bool)>, String>>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let r = khgis_folder_list(id, &path).await.map_err(|e| js_error(&e)).map(|v| {
            v.dyn_ref::<js_sys::Array>()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| {
                            let p = p.dyn_into::<js_sys::Array>().ok()?;
                            Some((p.get(0).as_string()?, p.get(1).as_bool().unwrap_or(false)))
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        put(&slot, r, &ctx);
    });
}

pub fn open(id: u32, path: String, name: String, inbox: Inbox, notes: Notes, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        match khgis_folder_files(id, &path, &name).await {
            Ok(v) => {
                let files: Vec<web_sys::File> =
                    v.dyn_ref::<js_sys::Array>().map(|a| a.iter().filter_map(|f| f.dyn_into().ok()).collect()).unwrap_or_default();
                crate::web::read_together(files, "", &inbox, notes).await;
            }
            Err(e) => notes.lock().unwrap_or_else(|e| e.into_inner()).push((format!("{name}: {}", js_error(&e)), true)),
        }
        ctx.request_repaint();
    });
}

pub fn forget(id: u32) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = khgis_folder_forget(id).await;
    });
}
