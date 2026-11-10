---
name: cambodia
description: KHGIS's Cambodia tools — load a province's or district's data (boundaries, roads, rivers, schools, health facilities, villages), run the one-click workflows (flood exposure, school and health access, schools per district) and make an A4 map in Khmer or English. Use for any request about a Cambodian place, province (ខេត្ត), district (ស្រុក/ខណ្ឌ) or commune (ឃុំ/សង្កាត់), or when the user writes in Khmer.
---

# Cambodia in KHGIS

## Language

- When the user writes in Khmer (ភាសាខ្មែរ), reply in Khmer, and pass `"lang": "km"` to the
  KHGIS commands below so layer names and map texts are in Khmer too. Otherwise reply in English.
- Province names may come in Khmer or English, or romanised in other ways
  ("Kompong Cham", "Siemreap", "Battambong"). `khgis.areas` lists the 25 provinces with both
  names; match the user's word to one of them before calling `khgis.loadArea`.
  Common Khmer names: ភ្នំពេញ Phnom Penh, កណ្ដាល Kandal, សៀមរាប Siem Reap, បាត់ដំបង Battambang,
  កំពង់ចាម Kampong Cham, កំពង់ស្ពឺ Kampong Speu, តាកែវ Takeo, ព្រៃវែង Prey Veng,
  ស្វាយរៀង Svay Rieng, កំពត Kampot, កែប Kep, ព្រះសីហនុ Preah Sihanouk, កោះកុង Koh Kong,
  ពោធិ៍សាត់ Pursat, កំពង់ឆ្នាំង Kampong Chhnang, កំពង់ធំ Kampong Thom, ព្រះវិហារ Preah Vihear,
  ឧត្ដរមានជ័យ Oddar Meanchey, បន្ទាយមានជ័យ Banteay Meanchey, ប៉ៃលិន Pailin, ក្រចេះ Kratie,
  ស្ទឹងត្រែង Stung Treng, រតនគិរី Ratanakiri, មណ្ឌលគិរី Mondulkiri, ត្បូងឃ្មុំ Tbong Khmum.
- GIS words: ផែនទី map, ស្រទាប់ layer, ផ្លូវ road, ទន្លេ river, សាលារៀន school,
  មណ្ឌលសុខភាព health centre, ភូមិ village, ទឹកជំនន់ flood, ប្លង់ layout, តំបន់ជុំវិញ buffer.

## Steps

1. A home map of Cambodia: `project.cambodia` `{"khmer": true|false}` (it replaces the project:
   ask first if the user has unsaved work).
2. An area's data: `khgis.loadArea` `{"province": "Kandal", "district"?: "Kien Svay",
   "themes"?: ["roads", "water", "schools", "health", "places", "worship"], "lang"?: "km"}`.
   It adds the boundary (and a province's districts) at once, then each theme from
   OpenStreetMap clipped to the area, and answers each layer's feature count. It uses the
   internet, so it waits for the user's approval.
3. Workflows on the loaded data: `khgis.recipe` `{"id": "floodExposure" | "schoolAccess" |
   "healthAccess" | "districtSchools", "distance"?: km, "lang"?}`. Read its `summary` and tell
   the user the numbers in their language.
4. A finished map: `khgis.autoLayout` `{"lang": "km"|"en", "title"?: …}` makes an A4 layout with
   title, legend, scale bar, north arrow, an inset and the data sources. Then `layout.export`
   to a PDF if the user asks for a file.

## Notes

- Boundaries are geoBoundaries (© OpenStreetMap contributors, ODbL); features are
  OpenStreetMap, whose coverage of villages and schools varies by province: say so when counts
  look low.
- Plugins add more (Rice SOS Mapper, Solar Siting, EIA Screening): `search_commands` for
  `plugin:` commands.
