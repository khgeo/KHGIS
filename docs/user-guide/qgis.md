# QGIS layout and QGIS plugins

KHGIS opens in a layout QGIS users know: a menu bar, icon toolbars, the **Browser** and
**Layers** docked on the left, the **Processing Toolbox** on the right and the status bar along
the bottom. It also runs the Processing algorithms of your own QGIS — QGIS's toolbox and every
installed QGIS plugin that adds Processing algorithms — on KHGIS's layers.

## The layout

| QGIS | KHGIS |
|---|---|
| Project, Edit, View, Layer, Settings, Plugins, Vector, Raster, Web, Processing, Help menus | the same menus, plus **Cambodia** (load a province or district, one-click workflows, A4 map) |
| Project, Map Navigation, Attributes, Digitizing, Data Source Manager and Plugins toolbars | the same toolbars, moved as in QGIS: drag a toolbar by its grip (the dots at its start) between two others, onto another row, or above or below the rows for a row of its own. A row too long for the window goes on over another line, so no button is hidden. Right-click the toolbars (or **View › Toolbars**) to show or hide each, or **Reset Toolbar Positions**. KHGIS remembers where they are |
| Browser and Layers panels | docked on the left; drag the bar between panels to resize. The Layers panel has QGIS's toolbar: Layer Styling, Add Group (with the layers picked), Add Layer, Attribute Table, Expand All, Collapse All and **Remove Layer** |
| Basemap | the **Basemap** row at the bottom of the Layers panel: click it to switch between Esri Shaded Relief (the default), Esri World Imagery, Esri World Topographic, Esri Light Gray Canvas, OpenStreetMap and the basemaps KHGIS draws without internet |
| Status bar: Coordinate, Scale, Rotation, CRS | the same, in that order: the pointer's coordinates (click to change their format), the scale (type one or pick it), the rotation in degrees, and the coordinate system (click to change it) |
| Dock panels: drag a panel's title to tab it with another, stack it, or move it to the other side | the same: drag a panel's tab onto another panel's tabs or middle to tab them, onto its top or bottom edge to stack, or to the window's left or right edge; right-click a tab to move it without dragging. KHGIS remembers the arrangement |
| Processing Toolbox | the Tools pane on the right (**Processing › Toolbox**) |
| Locator bar | **Command Search** (Ctrl+K), top right |
| Layer Styling panel | **Layer › Layer Styling**: the layer at the top, Symbology / Labels / Information down the left, the renderer (Single Symbol, Categorized, Graduated, Proportional Symbols) in a list |
| Feature Attributes form after digitizing | the same: finishing a point, line or polygon (double-click, right-click or Enter) opens a form with a box for each field; **OK** adds the feature with its values (one step to undo), **Cancel** leaves it out. A text field's **▾** lists the values it already has (the classes of a training layer). **Remember the last values** fills the form with the last ones; **Settings › Suppress Attribute Form After Feature Creation** turns the form off |
| Print Layout window | opening a layout swaps in the layout's own menus (Layout, Edit, View, Add Item, Atlas) and toolbar, and shows only Item Properties; **← Back to Map** returns |

Every menu entry and toolbar button runs a KHGIS command, so nothing is lost from the ribbon.
**View › Toolbars › Ribbon** shows the ribbon below the toolbars at any time.

The **ខ្មែរ | EN** switch at the top right changes the interface language in one click.

**Settings › Interface** switches between the QGIS layout and the ribbon; KHGIS remembers the
choice. **Settings › Options…** has the rest of the settings (theme, interface scale, language,
start-up project, autosave…).

## Layer Styling: Live update

At the foot of **Layer Styling**, **Live update** (on by default) shows every change on the map
at once. Turn it off to change the style first and see it on the map when you press **Apply**,
as in QGIS. A raster's style always shows at once.

## Continue in QGIS: Save As GeoPackage for QGIS

**Project › Save As GeoPackage for QGIS…** saves the map as one `.gpkg` file that QGIS opens and
keeps working with — on the desktop and in the browser (where it downloads):

- every vector layer as a GeoPackage table, in longitude/latitude (EPSG:4326);
- each layer's style (single symbol, categories, graduated classes, proportional symbols, labels,
  transparency, scale range) as the table's default style, so the style comes with a layer
  however it is added in QGIS;
- every raster layer (a downloaded image, an index, a class map) as GeoPackage tiles, drawn as
  KHGIS shows it, so a false-colour composite or a class map looks the same in QGIS; its values
  stay in KHGIS (export a GeoTIFF with **Layer › Save As** for analysis in QGIS);
- the print layouts, as QGIS print layouts (**Project › Layout Manager** in QGIS): map frames
  with their extent, grid and extent indicator, text, legend, scale bar, north arrow, pictures,
  rectangles, ellipses and lines, where they were on the page; tables, charts and colourbars are
  named in a message, as QGIS has no matching item here;
- a QGIS project inside the GeoPackage with the layers in their order, the XYZ web tile layers
  (satellite imagery, OpenStreetMap…) and the view.

In QGIS: **Project › Open From › GeoPackage**, or drag the file into QGIS and open the project
listed under it in the Browser. QGIS can edit the layers and save the project back into the same
file. Keep the file's name: the project refers to it. In the browser each save gets a name of its
own with the date and time (a second download of the same name would become `name (1).gpkg`,
which the project couldn't find). If the file was renamed, QGIS shows **Handle Unavailable
Layers**: press **Auto-Find**, then **Apply Changes**. What isn't saved is listed in the messages.

KHGIS projects themselves save as `.kgis` (older `.rgis` projects still open).

## LEE RS Lab

The **LEE RS Lab** panel (a tab beside Layer Styling; also **Raster › LEE RS Lab** and the
Cambodia menu) brings the LEE RS Lab QGIS plugin's lessons into KHGIS, so a class can do them in
the browser with no QGIS installed. Slow steps run in the background on the desktop, with their
progress and Cancel at the top of the lab; a run of steps (download, merge, clip…) goes one after
another by itself.

**1 Data**

- **Study area**: a province, district or commune (the 2024 boundaries are bundled: 25
  provinces, 209 districts and 1,702 communes, named in English and Khmer, so no network is
  needed to pick one), the map view, or a polygon layer of your own. *Clip to the area's shape*
  cuts the image to the boundary, not just its box.
- **Datasets** (Microsoft Planetary Computer, no account): Sentinel-2 (10 m), Landsat 8–9 and
  Landsat 4–7 (30 m), Sentinel-1 radar VV/VH in dB, the Copernicus 30 m DEM (with slope), ESA
  WorldCover and Esri land cover (10 m, class names filled in), and JRC surface water occurrence.
- Imagery is searched by date and scene cloud cover. *Mask clouds and shadows* uses Sentinel-2's
  scene classes (SCL) or Landsat's QA_PIXEL bits. *Dates in the composite* above 1 makes a median
  composite of the picked date and the next clearest ones, which fills the clouds.
- Each tile is downloaded clipped to the area, and the tiles merged into one image: six bands of
  surface reflectance (Blue, Green, Red, NIR, SWIR1, SWIR2) for optical imagery. A large area gets
  larger cells (at most 2,500 across), and the lab says so.
- **Multi-season stack** (Sentinel-2 and Landsat): a median composite of each season of one year
  (early dry Dec–Feb, late dry Mar–May, wet Jun–Oct) and its NDVI, all in one image with bands
  named `dry1_NIR`, `wet_NDVI`…, ready to classify: rice, other crops and forest differ most
  between seasons.

**2 Enhance**: band combinations (natural colour, colour infrared, agriculture, urban,
land/water, vegetation, or any R/G/B); stretches (min–max, percent clip, standard deviation,
histogram equalization); ten indices (NDVI, GNDVI, SAVI, EVI, NDWI, MNDWI, NDMI, NDBI, BSI, NBR);
band math with your own formula; spatial filters (mean, median, Gaussian, sharpen, Sobel,
Laplacian, emboss, minimum, maximum); principal components (with each one's explained
variance); and a **feature stack** for classification: the image's bands plus NDVI, NDWI, NDBI,
GLCM texture of the NIR band (contrast, homogeneity, entropy) and another layer (another date,
radar or elevation).
**Look at the image**: the **spectral profile tool** (click the map; each click adds that pixel's
value in every band to a line chart, up to six), a histogram of any band and a **feature space**
plot (the density of two bands' values against each other).

**3 Classify**

- **Classes (legend)**: one class list (id, name, colour) for the whole tab — cluster labelling,
  the magic wand, classification colours, point labelling and change. Start from a scheme (RS Lab
  default 6 classes, LULC level 1, Auto 7 from WorldCover, ESA WorldCover's 11), edit, add or
  remove classes, and save or open the list as CSV.
- Unsupervised: K-means or ISODATA (clusters split and merged between a fewest and most). The
  clusters are then listed with their area, %, a **hint** read from their spectrum (water,
  built-up, bare, crop, forest…; from the seasons in a multi-season stack) and a **→ class**
  box. Click a cluster to zoom to an example pixel of it on imagery (again for another);
  **Suggest classes from hints** fills the empty boxes; **Split selected cluster into** 2–6
  re-clusters a mixed one; **Plot cluster spectra** draws their mean spectra; **Create labelled
  map** merges the clusters into their classes, named and coloured from the list.
- Supervised, trained from polygons you draw (**Create training layer**, then Toggle Editing and
  Add Polygon Feature; when the polygon is finished, type its class in the form's name box) or **from a land cover map** (WorldCover or Esri
  downloaded in 1 Data): Random Forest (with each band's importance), k-Nearest Neighbours,
  Maximum Likelihood, Mahalanobis Distance, Minimum Distance, Spectral Angle Mapper and
  Parallelepiped. **Training signatures** plots each training class's mean spectrum (classes
  whose lines lie close will be confused). The **magic wand** draws training polygons for you:
  press a class's button (or type the class) and click the map, **Undo last** takes the last one
  back, and each button counts its polygons; the pixels around the click that are like it (within the similarity
  tolerance, in standard deviations of each band) become one polygon of that class. The training accuracy is shown (optimistic: the pixels taught it).
- Clean-up: a majority filter and a minimum mapping unit (sieve). **Class map to polygons** turns
  each patch into a polygon with its class and hectares.
- A classification is drawn in the class list's colours; **Apply class colours** does the same
  for any class map, and the class areas show as a bar chart.
- Accuracy: **Generate validation points** with a sample design — Cochran's total for the
  accuracy expected and its margin, shared out by class area with a minimum per class
  (recommended), or equally, or a fixed number per class. **Label the points** goes through
  them one by one: the map zooms to each, a button per class labels it, **Next unlabelled**
  moves on, and the map's class can be hidden for blind labelling. Then **Compute accuracy**: error matrix, overall accuracy,
  kappa, user's and producer's accuracy, and area-adjusted estimates with 95 % confidence
  intervals (Olofsson et al. 2014). Polygons of known classes work as reference too. **Export CSV**
  saves the error matrix and the figures.

**4 Change**: two class maps compared from → to: the change map, a Sankey diagram, gains and
losses per class and the largest changes. Maps numbered differently (WorldCover and a
classification trained on it) are matched by class name. Image differencing (difference, ratio,
percent) compares two images or indices.

**5 LEE**: Earth-Engine-style scripts, as in LEE RS Lab, run here with no Python installed and
no account. Ten examples (true colour and NDVI, water with MNDWI and Otsu, terrain, WorldCover
statistics, land-cover change, burn severity dNBR, Sentinel-1 floods, K-means, surface water, an
indices pack) load into the editor; **Run script** runs it. The study area of 1 Data is `aoi`.

```python
s2 = (lee.ImageCollection("sentinel-2").filterDate("2025-11-01", "2026-03-31")
        .filterMetadata("eo:cloud_cover", "lt", 30).limit(6))
img = s2.median()
ndvi = img.normalizedDifference("nir", "red").rename("NDVI")
Map.addLayer(ndvi, {"min": 0, "max": 0.9, "palette": "RdYlGn"}, "NDVI")
print("mean NDVI:", ndvi.reduceRegion("mean"))
```

The language is the part of Python these scripts use: assignments (also `d[k] = v`), lists,
dicts, `for` loops, `print`, `%` formatting, `round`, `len`, `min`, `max`, `sum`, `sorted`,
`range`. Images: `lee.ImageCollection(name)` with `filterDate`, `filterBounds`,
`filterMetadata("eo:cloud_cover", "lt", n)`, `limit`, `select`, `size`, `median`, `mean`,
`mosaic`, `first`; `lee.Image.load("dem"|"worldcover"|"lulc-annual"|"surface-water", year)`;
`lee.Image("a layer")`; and on images `normalizedDifference`, `expression` (band names in the
formula), `select`, `rename`, `+ - * / **`, `add`, `subtract`, `multiply`, `divide`, `gt`, `lt`,
`gte`, `lte`, `eq`, `neq`, `And`, `Or`, `Not`, `updateMask`, `clip`, `slope`, `aspect`,
`hillshade`, `cluster(k=…)`, `otsu()`, `reduceRegion("mean"|"median"|"min"|"max"|"sum"|"count"|"stdDev")`,
`classAreas()`, `bandNames()` and `grid.scale`. `Map.addLayer(image, vis, name)` takes `bands`,
`min`, `max` and a `palette` name (ColorBrewer and matplotlib names, `_r` reversed);
`Map.centerObject(aoi)`; `Chart.bars(dict, title)`; `lee.config(scale=30)` sets the cell size.
Images are made only when needed (downloads and steps run in the background, then the script
runs again from the top, with what it already made, printed or drew kept). **Save…** and **Open…** keep scripts as `.py` files; **API** lists what scripts can use. Not yet: `lambda`,
`map` over a collection, `if`, `def` and `import`.

**6 Map**: the A4 layout with a title made from the area and the date (in Khmer, e.g.
*ផែនទីការប្រើប្រាស់ដី និងគម្របដី ស្រុក/ខណ្ឌ … ខេត្ត… ឆ្នាំ២០២៥*), the **PDF report** (an A4
page with the map, north arrow, scale bar and legend, the class areas as a table and a bar
chart, the accuracy and the largest changes, and the data credits, then saved as PDF), and
Cambodia 3D.

Commands the lab uses, which scripts can use too: `raster.process` (`stack`, and `merge` with
`maskBand`/`maskValues`/`maskBits`, `combine: "median"`, `nodata`, `scale`, `offset`,
`decibels`), `raster.classify` (`method: "isodata"`), `raster.classifySupervised` (`method`,
`trainingRaster`), `raster.filter`, `raster.pca`, `raster.texture`, `raster.sieve`,
`raster.crossTab`, `raster.classAreas`, `raster.accuracy`, `raster.samplePoints` and
`raster.classNames`, `raster.magicWand {layer, lon, lat, tolerance, addTo, class}` and
`raster.toPolygons`; band math also takes comparisons (`>`, `==`…, giving 1 or 0), `&`, `|`, `^`
and `sqrt`, `abs`, `log`, `ln`, `log10`, `exp`, `min`, `max`.


## Cambodia 3D

**View › Open in Cambodia 3D** (also in the Cambodia menu and the LEE RS Lab's Map step) opens
[Cambodia 3D](https://khgeo.github.io/cambodia-3d/) on the middle of the map view, up to 4 × 4 km:
terrain on satellite imagery, with trees, houses, roads and land cover built from it.
`ui.cambodia3d {"bbox": [w, s, e, n]}` opens a given area.

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
