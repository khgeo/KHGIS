
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
