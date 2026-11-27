//! Add Data's picker where the browser has the File System Access API (Chrome, Edge): a
//! Shapefile is picked by its `.shp` alone, as on the desktop, and its `.dbf`, `.shx`, `.prj`
//! and `.cpg` are read from its folder, which the browser asks once to let KHGIS read. Elsewhere
//! (Firefox, Safari) the ordinary picker is used, where the parts are picked together.

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const SIDECARS = ["dbf", "shx", "prj", "cpg"];
export async function khgis_pick_data(exts, multiple) {
  if (typeof window.showOpenFilePicker !== "function" || window.self !== window.top) return null;
  const accept = exts.filter((e) => /^[a-z0-9]{1,15}$/i.test(e)).map((e) => "." + e);
  let handles;
  try {
    handles = await window.showOpenFilePicker({
      id: "khgis-data",
      multiple,
      excludeAcceptAllOption: false,
      types: [{ description: "Vector and raster data", accept: { "application/octet-stream": accept } }],
    });
  } catch (e) {
    // Cancelled: nothing. Refused for another reason (a policy, a sandbox): the ordinary picker.
    return e && e.name === "AbortError" ? [] : null;
  }
  const files = [];
  for (const h of handles) files.push(await h.getFile());
  const have = new Set(files.map((f) => f.name.toLowerCase()));
  const base = (n) => n.slice(0, -4).toLowerCase();
  const lone = handles.filter((h, i) => /\.shp$/i.test(h.name) && !have.has(base(h.name) + ".dbf"));
  if (lone.length === 0 || typeof window.showDirectoryPicker !== "function") return files;
  let dir;
  try {
    dir = await window.showDirectoryPicker({ id: "khgis-data", startIn: lone[0], mode: "read" });
  } catch (e) {
    return files; // Not allowed: the shapes alone, said when read.
  }
  // The parts of each lone .shp that is in that folder (the same file: its name and size).
  const sizes = new Map(lone.map((h, i) => [h.name.toLowerCase(), files[handles.indexOf(h)].size]));
  const inDir = new Map();
  for await (const [name, entry] of dir.entries()) {
    if (entry.kind === "file") inDir.set(name.toLowerCase(), entry);
  }
  for (const [shp, size] of sizes) {
    const e = inDir.get(shp);
    if (!e || (await e.getFile()).size !== size) continue;
    for (const ext of SIDECARS) {
      const n = shp.slice(0, -4) + "." + ext;
      const part = inDir.get(n);
      if (part && !have.has(n)) {
        files.push(await part.getFile());
        have.add(n);
      }
    }
  }
  return files;
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn khgis_pick_data(exts: Vec<String>, multiple: bool) -> Result<JsValue, JsValue>;
}

/// The files picked, a Shapefile's parts read from its folder; `None` where this picker isn't
/// there (or the browser refused it), for the ordinary one.
pub async fn pick(exts: &[&str], multiple: bool) -> Option<Vec<web_sys::File>> {
    // The parts come from the .shp's folder, so they aren't listed (All files still shows them).
    let exts: Vec<String> = exts.iter().filter(|e| !matches!(**e, "dbf" | "shx" | "prj" | "cpg")).map(|e| e.to_string()).collect();
    let picked = khgis_pick_data(exts, multiple).await.ok()?;
    if picked.is_null() || picked.is_undefined() {
        return None;
    }
    let array: js_sys::Array = picked.dyn_into().ok()?;
    Some(array.iter().filter_map(|f| f.dyn_into::<web_sys::File>().ok()).collect())
}
