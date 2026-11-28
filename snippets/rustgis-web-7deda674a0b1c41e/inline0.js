
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
