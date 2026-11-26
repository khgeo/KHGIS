# SPDX-License-Identifier: GPL-2.0-or-later
"""Installs QGIS plugins by name from the official repository (plugins.qgis.org) into the
default QGIS profile, for the KHGIS QGIS bridge image: python3 install_plugins.py "A,B"."""

import io
import os
import sys
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

REPO = "https://plugins.qgis.org/plugins/plugins.xml?qgis=3.34"


def main(names):
    wanted = {n.strip().lower() for n in names.split(",") if n.strip()}
    with urllib.request.urlopen(REPO, timeout=120) as r:
        root = ET.fromstring(r.read())
    dest = os.path.expanduser("~/.local/share/QGIS/QGIS3/profiles/default/python/plugins")
    os.makedirs(dest, exist_ok=True)
    found = set()
    for p in root.findall("pyqgis_plugin"):
        name = (p.get("name") or "").strip()
        if name.lower() not in wanted or name.lower() in found:
            continue
        url = (p.findtext("download_url") or "").strip()
        if not url.startswith("https://"):
            continue
        with urllib.request.urlopen(url, timeout=300) as r:
            data = r.read()
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            root_dir = os.path.realpath(dest)
            for n in z.namelist():
                target = os.path.realpath(os.path.join(dest, n))
                if not target.startswith(root_dir + os.sep):
                    raise SystemExit("unsafe path in %s: %s" % (name, n))
            z.extractall(dest)
        found.add(name.lower())
        print("installed", name, p.get("version"))
    missing = wanted - found
    if missing:
        raise SystemExit("not in the QGIS plugin repository: " + ", ".join(sorted(missing)))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "")
