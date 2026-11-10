<h1 align="center">KHGIS</h1>
<h2 align="center">ប្រព័ន្ធព័ត៌មានភូមិសាស្ត្រសម្រាប់កម្ពុជា</h2>

<p align="center">
  <b>A desktop GIS for Cambodia, in Khmer and English.</b><br>
  Cambodian boundaries, OpenStreetMap data and one-click workflows built in, with a full Khmer
  interface and print-ready A4 maps in either language.
</p>

<p align="center">
  <img alt="written in Rust" src="https://img.shields.io/badge/written%20in-Rust-b7410e">
  <img alt="licence Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue">
  <img alt="interface Khmer and English" src="https://img.shields.io/badge/interface-ខ្មែរ%20%7C%20English-0891B2">
  <img alt="built on RustGIS" src="https://img.shields.io/badge/built%20on-RustGIS-555">
</p>

---

KHGIS is a fork of [RustGIS](https://github.com/opengeos/RustGIS) that makes the app usable for
Cambodian work without setup: the interface speaks Khmer, the country's provinces and districts
are bundled, and the everyday tasks — load a province, find villages far from a school, print an
A4 map — are single commands.

## What KHGIS adds

- **Khmer interface.** 1,255 translated strings (`crates/ui-egui/src/i18n/km.tsv`). Switch the
  whole app between English and ភាសាខ្មែរ; anything untranslated falls back to English.
- **Khmer text that renders correctly.** Noto Sans Khmer is bundled and the renderer shapes
  Khmer properly, so labels, legends and exported maps are readable rather than broken clusters.
- **Cambodia built in.** Province (ADM1) and district (ADM2) boundaries ship with the app — no
  download, no projection fiddling before you start.
- **One-click area loading.** `khgis.loadArea` takes a province or district and adds its
  boundary plus roads, rivers, schools, health facilities and villages from OpenStreetMap,
  clipped to the area, with layer names in Khmer or English.
- **Workflows.** `khgis.recipe` runs four chains end to end:
  | id | what it answers |
  |---|---|
  | `floodExposure` | villages within a distance of rivers and water |
  | `schoolAccess` | villages farther than a distance from any school |
  | `healthAccess` | villages farther than a distance from any health facility |
  | `districtSchools` | schools counted per district, as a choropleth |
- **A4 maps in one step.** `khgis.autoLayout` produces a print-ready layout — title, legend,
  scale bar, north arrow, regional inset, data sources and date — in Khmer or English.
- **A Cambodia-aware assistant.** The bundled `cambodia` skill understands province names in
  Khmer, English and loose romanisations ("Kompong Cham", "Siemreap"), and replies in Khmer when
  you write in Khmer.

## Khmer place names

All 25 provinces are matched in either language, so ភ្នំពេញ, Phnom Penh and "Phnom Penh Capital"
all resolve to the same area. `khgis.areas` lists them with both names, and a province's
districts.

## Get KHGIS

Build from source (Rust 1.90 or later):

```sh
git clone https://github.com/khgeo/KHGIS
cd KHGIS
cargo run --release -p rustgis
```

> The crate and binary names are still RustGIS's; only the branding and the Cambodia features
> are KHGIS's. See [Relationship to RustGIS](#relationship-to-rustgis).

## Quick start

```sh
cargo run --release -p rustgis                  # the desktop app
cargo run --release -p rustgis -- data.geojson  # open data (or a .rgis project)
```

Every action is a command, so the Cambodia tools work from the command line too:

```sh
rustgis-cli run \
  --cmd 'project.cambodia={"khmer":true}' \
  --cmd 'khgis.loadArea={"province":"Kandal","themes":["water","places"],"lang":"km"}' \
  --cmd 'khgis.recipe={"id":"floodExposure","distance":3,"lang":"km"}' \
  --cmd 'khgis.autoLayout={"lang":"km"}' \
  --save kandal.rgis
```

## Relationship to RustGIS

KHGIS tracks [opengeos/RustGIS](https://github.com/opengeos/RustGIS) and adds a Cambodia and
Khmer layer on top. The underlying crates (`rustgis-geom`, `rustgis-data`, `rustgis-map`,
`rustgis-engine` and the rest) keep their upstream names and architecture, which is what lets
KHGIS pull in upstream improvements.

Upstream features documented in RustGIS's docs — the web app, Python package, plugin registry,
Microsoft Store builds and package-manager installs — belong to RustGIS and are not published
under the KHGIS name. Use the upstream project for those.

## License and credits

KHGIS is licensed under [Apache-2.0](LICENSE), the same licence as RustGIS.

Built on [RustGIS](https://github.com/opengeos/RustGIS) by Qiusheng Wu ([@giswqs](https://github.com/giswqs))
and its contributors. Thanks to that project for the engine this work stands on.

- Noto Sans Khmer — Google, [SIL Open Font License 1.1](third-party/NotoSansKhmer-OFL.txt)
- Cambodian administrative boundaries, OpenStreetMap data via Overpass — see [ATTRIBUTION.md](ATTRIBUTION.md)
- Every other bundled asset and dependency licence: [ATTRIBUTION.md](ATTRIBUTION.md) and
  [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt)
