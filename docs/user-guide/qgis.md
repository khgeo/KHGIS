# QGIS layout and QGIS plugins

KHGIS opens in a layout QGIS users know: a menu bar, icon toolbars, the **Browser** and
**Layers** docked on the left, the **Processing Toolbox** on the right and the status bar along
the bottom. It also runs the Processing algorithms of your own QGIS — QGIS's toolbox and every
installed QGIS plugin that adds Processing algorithms — on KHGIS's layers.

## The layout

| QGIS | KHGIS |
|---|---|
| Project, Edit, View, Layer, Settings, Plugins, Vector, Raster, Web, Processing, Help menus | the same menus, plus **Cambodia** (load a province or district, one-click workflows, A4 map) |
| Project, Map Navigation, Attributes, Digitizing, Data Source Manager and Plugins toolbars | the same toolbars; **View › Toolbars** shows or hides each |
| Browser and Layers panels | docked on the left; drag the bar between panels to resize |
| Dock panels: drag a panel's title to tab it with another, stack it, or move it to the other side | the same: drag a panel's tab onto another panel's tabs or middle to tab them, onto its top or bottom edge to stack, or to the window's left or right edge; right-click a tab to move it without dragging. KHGIS remembers the arrangement |
| Processing Toolbox | the Tools pane on the right (**Processing › Toolbox**) |
| Locator bar | **Command Search** (Ctrl+K), top right |
| Layer Styling panel | **Layer › Layer Styling**: the layer at the top, Symbology / Labels / Information down the left, the renderer (Single Symbol, Categorized, Graduated, Proportional Symbols) in a list |
| Print Layout window | opening a layout swaps in the layout's own menus (Layout, Edit, View, Add Item, Atlas) and toolbar, and shows only Item Properties; **← Back to Map** returns |

Every menu entry and toolbar button runs a KHGIS command, so nothing is lost from the ribbon.
**View › Toolbars › Ribbon** shows the ribbon below the toolbars at any time.

The **ខ្មែរ | EN** switch at the top right changes the interface language in one click.

**Settings › Interface** switches between the QGIS layout and the ribbon; KHGIS remembers the
choice. **Settings › Options…** has the rest of the settings (theme, interface scale, language,
start-up project, autosave…).

## QGIS plugins and algorithms

QGIS plugins are Python programs written for QGIS's own window (PyQt) and API (PyQGIS), so they
can't run inside KHGIS itself. KHGIS runs **your QGIS** instead, without a window, and uses what
QGIS plugins add to its Processing toolbox:

- every QGIS algorithm (native, GDAL, GRASS and SAGA when installed…), and
- every algorithm of an installed QGIS plugin whose `metadata.txt` says
  `hasProcessingProvider=yes`.

A plugin's own dialogs and dock panels stay in QGIS.

### Connecting QGIS (desktop)

1. Install [QGIS](https://qgis.org) (3.22 or later; the LTR is a good choice).
2. In KHGIS: **Plugins › QGIS Plugins & Tools… › Connect QGIS**.

KHGIS finds QGIS in the usual places: `C:\Program Files\QGIS …\bin\python-qgis-ltr.bat` (or
`python-qgis.bat`, or OSGeo4W's) on Windows, `/Applications/QGIS*.app/Contents/MacOS/bin/python3`
on macOS, and `python3` with PyQGIS on Linux. Pick another with **Browse…**, or set
`KHGIS_QGIS_PYTHON`. Connecting starts QGIS, loads its plugins and lists every algorithm (this
takes a few seconds to a minute); the list is kept, so later starts are instant. Connect again
after installing or updating a QGIS plugin.

The algorithms appear in the Processing Toolbox under **QGIS › provider › group**, in the
**Vector** and **Raster** menus under **QGIS Processing**, and in Command Search. Their forms are
made from QGIS's parameter definitions.

**Install a QGIS plugin** takes a plugin `.zip` (from [plugins.qgis.org](https://plugins.qgis.org)
or your own) and puts it in your QGIS profile, where QGIS and KHGIS both find it. Plugins run
Python code on your computer: install only those you trust.

### How a run works

KHGIS writes the input layers (their selection, if any) to a scratch folder — vector layers as
GeoJSON in the UTM zone of the data, so distances are in metres, rasters as GeoTIFF or by their
file — and QGIS runs the algorithm there. Vector outputs come back as layers in
longitude/latitude, rasters as raster layers, and numbers or reports as messages. The run shows
its progress and can be cancelled, and it is one undo step.

### In the browser

The web app has no QGIS on the computer, so it uses a **KHGIS QGIS bridge service**: the same
bridge script serving a headless QGIS over HTTPS. `packaging/qgis-bridge` builds it as a Docker
image (for Cloud Run or any server), with the QGIS plugins you choose. Enter its address in
**Plugins › QGIS Plugins & Tools** and **Connect**; the web app reconnects on later visits.
