# syntax=docker/dockerfile:1
#
# The KHGIS QGIS bridge service: a headless QGIS that runs Processing algorithms (QGIS's own and
# those of the QGIS plugins installed in the image) for the KHGIS web app. Build from the
# repository root:
#
#   docker build -f packaging/qgis-bridge/Dockerfile -t khgis-qgis-bridge .
#   docker run --rm -p 8080:8080 -e KHGIS_BRIDGE_ORIGINS=https://khgeo.github.io khgis-qgis-bridge
#
# Then in KHGIS (web): Plugins › QGIS Plugins & Tools › Connect, with the service's address.
# On Cloud Run: gcloud run deploy khgis-qgis-bridge --source . (see README.md beside this file).

# The official QGIS image (LTR): QGIS, PyQGIS, GDAL and the Processing framework.
FROM qgis/qgis:ltr

ENV QT_QPA_PLATFORM=offscreen \
    XDG_RUNTIME_DIR=/tmp/runtime-qgis \
    PYTHONUNBUFFERED=1 \
    PORT=8080 \
    KHGIS_BRIDGE_ORIGINS=* \
    KHGIS_BRIDGE_MAX_MB=200

# QGIS plugins to offer, by their plugins.qgis.org name (those with Processing algorithms add
# tools): docker build --build-arg QGIS_PLUGINS="Plugin One,Another" …
ARG QGIS_PLUGINS=""
COPY packaging/qgis-bridge/install_plugins.py /opt/khgis/install_plugins.py
RUN mkdir -p "$XDG_RUNTIME_DIR" && if [ -n "$QGIS_PLUGINS" ]; then python3 /opt/khgis/install_plugins.py "$QGIS_PLUGINS"; fi

COPY crates/qgis/bridge/khgis_qgis_bridge.py /opt/khgis/khgis_qgis_bridge.py

# Plugin .zip files dropped into packaging/qgis-bridge/plugins/ are installed too.
COPY packaging/qgis-bridge/plugins/ /opt/khgis/plugins/
RUN for z in /opt/khgis/plugins/*.zip; do [ -e "$z" ] && python3 /opt/khgis/khgis_qgis_bridge.py install "$z"; done; true

# Lists the algorithms once while building, so a broken plugin fails the build, not a visit.
RUN python3 /opt/khgis/khgis_qgis_bridge.py manifests > /tmp/manifests.json && python3 -c "import json; d=json.load(open('/tmp/manifests.json')); assert 'ok' in d, d; print(len(d['ok']['algorithms']), 'algorithms')"

EXPOSE 8080
CMD ["sh", "-c", "python3 /opt/khgis/khgis_qgis_bridge.py serve ${PORT}"]
