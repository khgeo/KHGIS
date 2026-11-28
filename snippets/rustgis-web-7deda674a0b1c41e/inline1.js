
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
