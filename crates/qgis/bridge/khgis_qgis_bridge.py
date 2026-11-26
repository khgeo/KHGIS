# SPDX-License-Identifier: GPL-2.0-or-later
"""KHGIS <-> QGIS bridge.

KHGIS runs this script with the Python that comes with QGIS (``python-qgis.bat`` on Windows,
``QGIS.app/Contents/MacOS/bin/python3`` on macOS, ``python3`` with PyQGIS on Linux). It starts
QGIS without a window, loads QGIS Processing and every installed QGIS plugin that provides
Processing algorithms, and then:

    python khgis_qgis_bridge.py version      -> one JSON line: QGIS and Python versions
    python khgis_qgis_bridge.py manifests    -> one JSON line: every algorithm and its parameters
    python khgis_qgis_bridge.py plugins      -> one JSON line: the installed QGIS plugins
    python khgis_qgis_bridge.py run          -> reads a job (one JSON line) on standard input
    python khgis_qgis_bridge.py install ZIP  -> unpacks a QGIS plugin .zip into the user's profile
    python khgis_qgis_bridge.py serve PORT   -> the same over HTTP, for the KHGIS web app

Every answer is a line of JSON: ``{"ok": ...}`` or ``{"error": "..."}``. While a job runs it also
prints ``{"progress": 0-100}`` and ``{"log": "..."}`` lines. Anything else QGIS or a plugin
prints goes to standard error, so standard output stays readable.

This file only uses the public PyQGIS API; it is licensed GPL-2.0-or-later like PyQGIS itself
and runs as a separate program from KHGIS.
"""

import json
import os
import sys
import traceback
import zipfile

BRIDGE_VERSION = 1

# Standard output carries the protocol only: everything QGIS and plugins print goes to stderr.
_PROTOCOL = sys.stdout
sys.stdout = sys.stderr


def emit(obj):
    _PROTOCOL.write(json.dumps(obj, default=str, ensure_ascii=False) + "\n")
    _PROTOCOL.flush()


def _extend_sys_path():
    """Makes `import qgis` work where the QGIS Python doesn't set it up itself (macOS bundle)."""
    prefix = os.environ.get("KHGIS_QGIS_PREFIX", "")
    candidates = []
    if prefix:
        candidates += [
            os.path.join(prefix, "python"),
            os.path.join(prefix, "..", "Resources", "python"),
            os.path.join(prefix, "share", "qgis", "python"),
        ]
    for c in candidates:
        c = os.path.normpath(c)
        if os.path.isdir(os.path.join(c, "qgis")) and c not in sys.path:
            sys.path.insert(0, c)


_extend_sys_path()
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

try:
    from qgis.core import (  # noqa: E402
        Qgis,
        QgsApplication,
        QgsCoordinateReferenceSystem,
        QgsCoordinateTransformContext,
        QgsProcessingContext,
        QgsProcessingFeedback,
        QgsProcessingParameterDefinition,
        QgsProcessingParameterEnum,
        QgsProcessingParameterField,
        QgsProcessingParameterMultipleLayers,
        QgsProcessingParameterNumber,
        QgsProcessingParameterFileDestination,
        QgsProject,
        QgsVectorFileWriter,
        QgsVectorLayer,
    )
except Exception as _e:  # not QGIS's Python, or QGIS is broken
    emit({"error": "this Python can't load QGIS (%s: %s)" % (type(_e).__name__, _e)})
    sys.exit(1)

_APP = None
_PLUGINS = []


def _start_qgis():
    """Starts QGIS headless, with Processing and the plugins that provide algorithms."""
    global _APP
    if _APP is not None:
        return
    prefix = os.environ.get("KHGIS_QGIS_PREFIX", "")
    if prefix:
        QgsApplication.setPrefixPath(prefix, True)
    _APP = QgsApplication([], False)
    _APP.initQgis()
    # The Processing framework and QGIS's own Python plugins (processing, grassprovider, …).
    plugin_dirs = [os.path.join(QgsApplication.pkgDataPath(), "python", "plugins")]
    for d in plugin_dirs:
        if os.path.isdir(d) and d not in sys.path:
            sys.path.append(d)
    from processing.core.Processing import Processing

    Processing.initialize()
    _load_plugins()


def _user_plugin_dir():
    return os.path.join(QgsApplication.qgisSettingsDirPath(), "python", "plugins")


def _metadata(path):
    """A plugin's metadata.txt as a dict (lower-case keys), or {}."""
    meta = {}
    f = os.path.join(path, "metadata.txt")
    if not os.path.isfile(f):
        return meta
    try:
        import configparser

        cp = configparser.ConfigParser(interpolation=None, strict=False)
        with open(f, encoding="utf-8", errors="replace") as fh:
            cp.read_file(fh)
        if cp.has_section("general"):
            meta = {k.lower(): v for k, v in cp.items("general")}
    except Exception:  # a broken metadata file names no plugin
        pass
    return meta


def _load_plugins():
    """Loads every user plugin that says it has a Processing provider, as qgis_process does."""
    import qgis.utils

    user_dir = _user_plugin_dir()
    if os.path.isdir(user_dir) and user_dir not in sys.path:
        sys.path.append(user_dir)
    seen = set()
    for base in [user_dir] + list(getattr(qgis.utils, "plugin_paths", [])):
        if not os.path.isdir(base):
            continue
        for name in sorted(os.listdir(base)):
            path = os.path.join(base, name)
            if name in seen or not os.path.isfile(os.path.join(path, "__init__.py")):
                continue
            seen.add(name)
            meta = _metadata(path)
            processing = meta.get("hasprocessingprovider", "no").strip().lower() in ("yes", "true", "1")
            entry = {
                "name": name,
                "title": meta.get("name", name),
                "version": meta.get("version", ""),
                "description": meta.get("description", ""),
                "author": meta.get("author", ""),
                "homepage": meta.get("homepage", ""),
                "processing": processing,
                "folder": path,
                "loaded": False,
                "error": None,
            }
            if processing:
                try:
                    if qgis.utils.loadPlugin(name) and qgis.utils.startProcessingPlugin(name):
                        entry["loaded"] = True
                    else:
                        entry["error"] = "the plugin didn't start its Processing provider"
                except Exception as e:  # one broken plugin mustn't hide the others
                    entry["error"] = "%s: %s" % (type(e).__name__, e)
            _PLUGINS.append(entry)


# ---------------------------------------------------------------------------------------------
# Manifests
# ---------------------------------------------------------------------------------------------

DESTINATIONS = {"sink", "vectorDestination", "rasterDestination", "fileDestination", "folderDestination", "pointCloudDestination", "vectorTileDestination"}


def _layer_kind(p):
    """'vector', 'raster' or 'any' for a layer parameter."""
    t = p.type()
    if t in ("source", "vector", "sink", "vectorDestination"):
        return "vector"
    if t in ("raster", "rasterDestination"):
        return "raster"
    if t == "multilayer":
        # QgsProcessing.TypeRaster (Qgis.ProcessingSourceType.Raster) is 3; 4 is plain files.
        try:
            lt = int(p.layerType())
        except Exception:
            lt = -1
        return "raster" if lt == 3 else ("file" if lt == 4 else "vector")
    return "any"


def _geometry_kind(p):
    """'point', 'line', 'polygon' or '' for a vector input that accepts one geometry type."""
    types = []
    try:
        types = [int(x) for x in p.dataTypes()]
    except Exception:
        return ""
    names = {0: "point", 1: "line", 2: "polygon"}
    kinds = {names[t] for t in types if t in names}
    return kinds.pop() if len(kinds) == 1 and len(types) == 1 else ""


def _default(p):
    d = p.defaultValue()
    if d is None:
        return None
    if isinstance(d, (bool, int, float, str)):
        return d
    if isinstance(d, (list, tuple)):
        return [x if isinstance(x, (bool, int, float, str)) else str(x) for x in d]
    return str(d)


def _param(p):
    t = p.type()
    flags = int(p.flags())
    optional_flag = int(getattr(QgsProcessingParameterDefinition, "FlagOptional", 8))
    hidden_flag = int(getattr(QgsProcessingParameterDefinition, "FlagHidden", 2))
    advanced_flag = int(getattr(QgsProcessingParameterDefinition, "FlagAdvanced", 1))
    out = {
        "name": p.name(),
        "description": p.description(),
        "help": p.help() if hasattr(p, "help") else "",
        "type": t,
        "optional": bool(flags & optional_flag),
        "hidden": bool(flags & hidden_flag),
        "advanced": bool(flags & advanced_flag),
        "destination": t in DESTINATIONS,
        "default": _default(p),
    }
    if t in ("source", "vector", "raster", "multilayer", "layer", "sink", "vectorDestination", "rasterDestination"):
        out["layer"] = _layer_kind(p)
    if t in ("source", "vector"):
        out["geometry"] = _geometry_kind(p)
    if isinstance(p, QgsProcessingParameterEnum):
        out["options"] = list(p.options())
        out["multiple"] = p.allowMultiple()
    if isinstance(p, QgsProcessingParameterField):
        out["parent"] = p.parentLayerParameterName()
        out["multiple"] = p.allowMultiple()
    if isinstance(p, QgsProcessingParameterMultipleLayers):
        out["multiple"] = True
    if isinstance(p, QgsProcessingParameterNumber):
        out["integer"] = int(p.dataType()) == 0  # Integer = 0, Double = 1
    if isinstance(p, (QgsProcessingParameterFileDestination,)):
        out["extension"] = p.defaultFileExtension()
    if t == "band":
        out["parent"] = p.parentLayerParameterName()
    return out


def manifests():
    _start_qgis()
    algs = []
    for alg in QgsApplication.processingRegistry().algorithms():
        try:
            provider = alg.provider()
            algs.append(
                {
                    "id": alg.id(),
                    "name": alg.displayName(),
                    "group": alg.group() or "",
                    "provider": provider.name() if provider else "",
                    "provider_id": provider.id() if provider else "",
                    "help": (alg.shortHelpString() or alg.shortDescription() or "")[:4000],
                    "params": [_param(p) for p in alg.parameterDefinitions()],
                    "outputs": [{"name": o.name(), "type": o.type(), "description": o.description()} for o in alg.outputDefinitions()],
                }
            )
        except Exception as e:
            print("skipping %s: %s" % (alg.id(), e), file=sys.stderr)
    return {"bridge": BRIDGE_VERSION, "qgis": Qgis.QGIS_VERSION, "algorithms": algs, "plugins": _PLUGINS}


# ---------------------------------------------------------------------------------------------
# Running an algorithm
# ---------------------------------------------------------------------------------------------


class _Feedback(QgsProcessingFeedback):
    def __init__(self):
        super().__init__()
        self._last = -1.0
        self.progressChanged.connect(self._progress)

    def _progress(self, value):
        if value - self._last >= 1.0 or value >= 100.0:
            self._last = value
            emit({"progress": round(float(value), 1)})

    def pushInfo(self, info):
        emit({"log": info})

    def reportError(self, error, fatalError=False):
        emit({"log": "Error: " + error})

    def pushWarning(self, warning):
        emit({"log": "Warning: " + warning})


def _to_geojson(src, dest):
    """Writes the layer at `src` (any OGR file) to `dest` as GeoJSON in longitude/latitude."""
    layer = QgsVectorLayer(src, "out", "ogr")
    if not layer.isValid():
        return "the output %s couldn't be opened" % os.path.basename(src)
    opts = QgsVectorFileWriter.SaveVectorOptions()
    opts.driverName = "GeoJSON"
    opts.fileEncoding = "UTF-8"
    from qgis.core import QgsCoordinateTransform

    wgs84 = QgsCoordinateReferenceSystem("EPSG:4326")
    if layer.crs().isValid() and layer.crs() != wgs84:
        opts.ct = QgsCoordinateTransform(layer.crs(), wgs84, QgsProject.instance())
    opts.layerOptions = ["RFC7946=YES", "WRITE_BBOX=NO"]
    if hasattr(QgsVectorFileWriter, "writeAsVectorFormatV3"):
        res = QgsVectorFileWriter.writeAsVectorFormatV3(layer, dest, QgsCoordinateTransformContext(), opts)
    else:
        res = QgsVectorFileWriter.writeAsVectorFormatV2(layer, dest, QgsCoordinateTransformContext(), opts)
    code = res[0] if isinstance(res, tuple) else res
    if code != QgsVectorFileWriter.NoError:
        return "the output %s couldn't be converted: %s" % (os.path.basename(src), res[1] if isinstance(res, tuple) and len(res) > 1 else code)
    return None


def run(job):
    """Runs job = {"alg", "params", "vector_outputs": {name: geojson path}}; returns results."""
    _start_qgis()
    import processing

    alg_id = job.get("alg", "")
    alg = QgsApplication.processingRegistry().algorithmById(alg_id)
    if alg is None:
        raise ValueError("QGIS has no algorithm '%s'" % alg_id)
    params = dict(job.get("params") or {})
    # Vector outputs are written as GeoPackages first, then turned into GeoJSON in lon/lat.
    vector_outputs = job.get("vector_outputs") or {}
    staged = {}
    for name, dest in vector_outputs.items():
        tmp = os.path.splitext(dest)[0] + ".gpkg"
        params[name] = tmp
        staged[name] = (tmp, dest)
    ctx = QgsProcessingContext()
    ctx.setProject(QgsProject.instance())
    fb = _Feedback()
    results = processing.run(alg_id, params, feedback=fb, context=ctx)
    notes = []
    for name, (tmp, dest) in staged.items():
        got = results.get(name)
        src = got if isinstance(got, str) and os.path.exists(got.split("|")[0]) else tmp
        if os.path.exists(src.split("|")[0]):
            err = _to_geojson(src, dest)
            if err:
                notes.append(err)
            results[name] = dest
    clean = {}
    for k, v in results.items():
        if isinstance(v, (bool, int, float, str)) or v is None:
            clean[k] = v
        elif isinstance(v, (list, tuple)):
            clean[k] = [x if isinstance(x, (bool, int, float, str)) else str(x) for x in v]
        else:
            clean[k] = str(v)
    return {"results": clean, "notes": notes}


# ---------------------------------------------------------------------------------------------
# Plugins
# ---------------------------------------------------------------------------------------------


def install(zip_path):
    """Unpacks a QGIS plugin .zip (one top-level folder with metadata.txt) into the profile."""
    _start_qgis()
    dest = _user_plugin_dir()
    os.makedirs(dest, exist_ok=True)
    with zipfile.ZipFile(zip_path) as z:
        names = [n for n in z.namelist() if n and not n.startswith("__MACOSX")]
        tops = {n.split("/")[0] for n in names}
        if len(tops) != 1:
            raise ValueError("a QGIS plugin .zip holds one folder; this one holds %d" % len(tops))
        top = tops.pop()
        if top + "/metadata.txt" not in names:
            raise ValueError("%s has no metadata.txt: it isn't a QGIS plugin" % top)
        root = os.path.realpath(dest)
        for n in names:
            target = os.path.realpath(os.path.join(dest, n))
            if not (target == root or target.startswith(root + os.sep)):
                raise ValueError("the .zip names a file outside its folder: %s" % n)
        z.extractall(dest, members=names)
    meta = _metadata(os.path.join(dest, top))
    return {"name": top, "title": meta.get("name", top), "version": meta.get("version", ""), "folder": os.path.join(dest, top),
            "processing": meta.get("hasprocessingprovider", "no").strip().lower() in ("yes", "true", "1")}


# ---------------------------------------------------------------------------------------------
# HTTP server for the web app (one job at a time; QGIS isn't thread-safe)
# ---------------------------------------------------------------------------------------------


def serve(port):
    """A small HTTP API: GET /manifests, POST /run (multipart-free JSON with base64 files)."""
    import base64
    import shutil
    import tempfile
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    _start_qgis()
    cached = json.dumps({"ok": manifests()}, default=str).encode("utf-8")
    lock = threading.Lock()
    token = os.environ.get("KHGIS_BRIDGE_TOKEN", "")
    origins = [o.strip() for o in os.environ.get("KHGIS_BRIDGE_ORIGINS", "*").split(",") if o.strip()]
    max_body = int(os.environ.get("KHGIS_BRIDGE_MAX_MB", "200")) * 1024 * 1024

    class Handler(BaseHTTPRequestHandler):
        def _cors(self):
            origin = self.headers.get("Origin", "")
            if "*" in origins:
                self.send_header("Access-Control-Allow-Origin", "*")
            elif origin in origins:
                self.send_header("Access-Control-Allow-Origin", origin)
                self.send_header("Vary", "Origin")
            self.send_header("Access-Control-Allow-Headers", "Content-Type, Authorization")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")

        def _send(self, code, body):
            self.send_response(code)
            self._cors()
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _authorized(self):
            return not token or self.headers.get("Authorization", "") == "Bearer " + token

        def do_OPTIONS(self):
            self.send_response(204)
            self._cors()
            self.end_headers()

        def do_GET(self):
            if self.path.rstrip("/") in ("", "/health"):
                return self._send(200, json.dumps({"ok": {"qgis": Qgis.QGIS_VERSION, "bridge": BRIDGE_VERSION}}).encode())
            if not self._authorized():
                return self._send(401, b'{"error":"unauthorized"}')
            if self.path.rstrip("/") == "/manifests":
                return self._send(200, cached)
            self._send(404, b'{"error":"not found"}')

        def do_POST(self):
            if not self._authorized():
                return self._send(401, b'{"error":"unauthorized"}')
            if self.path.rstrip("/") != "/run":
                return self._send(404, b'{"error":"not found"}')
            n = int(self.headers.get("Content-Length", "0"))
            if n <= 0 or n > max_body:
                return self._send(413, b'{"error":"the request is empty or too large"}')
            try:
                req = json.loads(self.rfile.read(n))
            except Exception:
                return self._send(400, b'{"error":"not JSON"}')
            work = tempfile.mkdtemp(prefix="khgis-qgis-")
            try:
                for f in req.get("files", []):
                    name = os.path.basename(f.get("name", ""))
                    if not name:
                        continue
                    with open(os.path.join(work, name), "wb") as fh:
                        fh.write(base64.b64decode(f.get("data", "")))
                job = json.loads(json.dumps(req.get("job", {})).replace("@WORK@", work.replace("\\", "/")))
                with lock:
                    result = run(job)
                files = []
                for name in sorted(os.listdir(work)):
                    p = os.path.join(work, name)
                    # Not the inputs, nor the GeoPackages vector outputs were written to first.
                    if name.endswith((".gpkg", ".gpkg-wal", ".gpkg-shm")):
                        continue
                    if os.path.isfile(p) and name not in {os.path.basename(f.get("name", "")) for f in req.get("files", [])}:
                        with open(p, "rb") as fh:
                            files.append({"name": name, "data": base64.b64encode(fh.read()).decode()})
                out = {"ok": result, "files": files}
                self._send(200, json.dumps(out, default=str).replace(work.replace("\\", "/"), "@WORK@").encode("utf-8"))
            except Exception as e:
                self._send(200, json.dumps({"error": "%s: %s" % (type(e).__name__, e)}).encode())
            finally:
                shutil.rmtree(work, ignore_errors=True)

        def log_message(self, fmt, *args):
            print(fmt % args, file=sys.stderr)

    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    print("KHGIS QGIS bridge on port %d (QGIS %s)" % (port, Qgis.QGIS_VERSION), file=sys.stderr)
    server.serve_forever()


def main(argv):
    cmd = argv[1] if len(argv) > 1 else "version"
    try:
        if cmd == "version":
            emit({"ok": {"qgis": Qgis.QGIS_VERSION, "python": sys.version.split()[0], "bridge": BRIDGE_VERSION}})
        elif cmd == "manifests":
            emit({"ok": manifests()})
        elif cmd == "plugins":
            _start_qgis()
            emit({"ok": {"plugins": _PLUGINS, "folder": _user_plugin_dir()}})
        elif cmd == "run":
            line = sys.stdin.readline()
            emit({"ok": run(json.loads(line))})
        elif cmd == "install" and len(argv) > 2:
            emit({"ok": install(argv[2])})
        elif cmd == "serve":
            serve(int(argv[2]) if len(argv) > 2 else int(os.environ.get("PORT", "8080")))
        else:
            emit({"error": "unknown bridge command '%s'" % cmd})
    except Exception as e:
        traceback.print_exc()
        emit({"error": "%s: %s" % (type(e).__name__, e)})
    finally:
        if _APP is not None:
            try:
                _APP.exitQgis()
            except Exception:
                pass


if __name__ == "__main__":
    main(sys.argv)
