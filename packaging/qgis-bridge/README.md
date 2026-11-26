# KHGIS QGIS bridge service

KHGIS runs QGIS Processing algorithms — QGIS's own (native, GDAL, …) and those of any QGIS plugin
that provides Processing algorithms — through the user's QGIS. On the desktop it starts the QGIS
installed on the computer. In the browser there is no QGIS, so this image runs one as a small web
service the web app calls.

```sh
# from the repository root
docker build -f packaging/qgis-bridge/Dockerfile -t khgis-qgis-bridge .
docker run --rm -p 8080:8080 -e KHGIS_BRIDGE_ORIGINS=http://localhost:8080 khgis-qgis-bridge
```

In KHGIS: **Plugins › QGIS Plugins & Tools**, enter the service's address, **Connect**. Its
algorithms appear in the Processing Toolbox under **QGIS ›** and in the Vector and Raster menus.

## Plugins in the image

- `--build-arg QGIS_PLUGINS="Name One,Name Two"` installs plugins from plugins.qgis.org by name.
- Plugin `.zip` files placed in `packaging/qgis-bridge/plugins/` are installed as well (your own
  plugins, for example).

Only plugins with `hasProcessingProvider=yes` in their `metadata.txt` add tools; a plugin that is
only a dock panel or dialog works in desktop QGIS but adds nothing here.

## Settings (environment)

| Variable | Default | Meaning |
|---|---|---|
| `PORT` | `8080` | Port to listen on (Cloud Run sets it). |
| `KHGIS_BRIDGE_ORIGINS` | `*` | Comma-separated web origins allowed to call it (set your KHGIS site, e.g. `https://khgeo.github.io`). |
| `KHGIS_BRIDGE_TOKEN` | — | When set, `/manifests` and `/run` need `Authorization: Bearer <token>`. |
| `KHGIS_BRIDGE_MAX_MB` | `200` | Largest request accepted. |

## Cloud Run

```sh
gcloud builds submit --tag REGION-docker.pkg.dev/PROJECT/khgis/qgis-bridge -f packaging/qgis-bridge/Dockerfile .
gcloud run deploy khgis-qgis-bridge --image REGION-docker.pkg.dev/PROJECT/khgis/qgis-bridge \
  --memory 2Gi --cpu 2 --concurrency 1 --timeout 900 \
  --set-env-vars KHGIS_BRIDGE_ORIGINS=https://khgeo.github.io
```

`--concurrency 1` because QGIS runs one algorithm at a time per instance; Cloud Run starts more
instances for more users. The service keeps no data: each run's files are deleted when it ends.

## API

- `GET /health` → `{"ok": {"qgis": "3.40.…", "bridge": 1}}`
- `GET /manifests` → every algorithm and its parameters, and the plugins loaded.
- `POST /run` with `{"job": {...}, "files": [{"name", "data": base64}]}` → `{"ok": {"results",
  "notes"}, "files": [...]}` or `{"error": "..."}`.

`khgis_qgis_bridge.py` uses only the public PyQGIS API and is licensed GPL-2.0-or-later, like
PyQGIS; it runs as a separate program from KHGIS (Apache-2.0).
