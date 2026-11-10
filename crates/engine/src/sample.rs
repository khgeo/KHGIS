//! The built-in sample project: a world map from Natural Earth (public domain) data.

use rustgis_data::io::read_geojson;
use rustgis_geom::{Coord, lonlat_to_merc};
use rustgis_map::{Bookmark, Camera, ClassMethod, Marker, Ramp, Renderer, Rgba, Symbol};

use crate::Project;

pub use rustgis_render::sample_data::{CITIES, COUNTRIES, COUNTRIES_CHN, LAKES, RIVERS, Worldview, countries, set_worldview, worldview};

/// The worldview this system calls for: `RUSTGIS_WORLDVIEW` (`default` or `china`) when set, else
/// China when the system's time zone or first locale is mainland China's. Nothing is looked up
/// over the network. Test builds of the engine don't look at the system.
pub fn system_worldview() -> Worldview {
    if let Some(w) = std::env::var("RUSTGIS_WORLDVIEW").ok().and_then(|v| Worldview::from_code(&v)) {
        return w;
    }
    if cfg!(test) {
        return Worldview::Default;
    }
    let tz = iana_time_zone::get_timezone().ok();
    let locales: Vec<String> = sys_locale::get_locales().take(4).collect();
    Worldview::detect(tz.as_deref(), locales.iter().map(String::as_str))
}

/// Applies a worldview preference: `auto` (or anything unknown) follows the system.
pub fn apply_worldview(pref: &str) -> Worldview {
    let w = Worldview::from_code(pref).unwrap_or_else(system_worldview);
    set_worldview(w);
    w
}

fn cam(lon: f64, lat: f64, res: f64) -> Camera {
    Camera { center: lonlat_to_merc(Coord::new(lon, lat)), res, bearing: 0.0, pitch: 0.0 }
}

/// The built-in countries in a worldview.
fn countries_in(w: Worldview) -> &'static str {
    match w {
        Worldview::Default => COUNTRIES,
        Worldview::China => COUNTRIES_CHN,
    }
}

/// Swaps an untouched sample project's countries from worldview `from` to `to`: only in a project
/// that is unsaved, unchanged and has nothing to undo, and only a layer named `Countries` holding
/// exactly the built-in countries of `from`. Returns whether a layer was swapped.
pub fn swap_countries(s: &mut crate::Session, from: Worldview, to: Worldview) -> bool {
    if from == to || s.dirty() || s.path.is_some() || s.can_undo() {
        return false;
    }
    let read = |w| read_geojson(countries_in(w), "Countries").ok().map(|(ds, _)| ds);
    let (Some(old), Some(new)) = (read(from), read(to)) else {
        return false;
    };
    let Some(l) = s.map_mut().layers.iter_mut().find(|l| l.name == "Countries" && l.dataset.features == old.features) else {
        return false;
    };
    l.dataset = new;
    l.selection.clear();
    l.touch_geometry();
    true
}

/// Slate gray country borders: visible against both the near-white lowest population class and the ocean.
const COUNTRY_BORDER: Rgba = Rgba::rgb(0x7D, 0x8C, 0x96);

/// "World Atlas": countries by population, cities sized by population, rivers and lakes.
pub fn world_project() -> Project {
    let mut p = Project::new("World Atlas");
    let map = p.map_mut();
    map.name = "World".into();
    map.basemap = rustgis_map::Basemap::Ocean;
    let mut add = |name: &str, text: &str| read_geojson(text, name).ok().map(|(ds, _)| map.add_dataset(ds));
    let countries = add("Countries", countries());
    let lakes = add("Lakes", LAKES);
    let rivers = add("Rivers", RIVERS);
    let cities = add("Cities", CITIES);
    if let Some(l) = countries.and_then(|id| map.layer_mut(id)) {
        let base = Symbol { fill: Rgba::rgb(200, 200, 200), stroke: COUNTRY_BORDER, stroke_width: 0.6, size: 0.0, marker: Marker::Circle };
        if let Ok(r) = Renderer::graduated(&l.dataset, &[], "pop_est", 5, ClassMethod::NaturalBreaks, Ramp::Lagoon, base) {
            l.renderer = r;
            l.renderer.map_symbols(|s| {
                s.stroke = COUNTRY_BORDER;
                s.stroke_width = 0.6;
            });
        }
        l.labels.enabled = true;
        l.labels.default.field = "name".into();
        l.labels.default.size = 8.5;
        l.labels.default.color = Rgba::rgb(0x33, 0x3A, 0x44);
    }
    if let Some(l) = lakes.and_then(|id| map.layer_mut(id)) {
        l.renderer = Renderer::Single(Symbol {
            fill: Rgba::rgb(0x9C, 0xC6, 0xE0),
            stroke: Rgba::rgb(0x6E, 0xA4, 0xC8),
            stroke_width: 0.5,
            size: 0.0,
            marker: Marker::Circle,
        });
    }
    if let Some(l) = rivers.and_then(|id| map.layer_mut(id)) {
        l.renderer = Renderer::Single(Symbol {
            fill: Rgba::rgb(0x4F, 0x8F, 0xC6),
            stroke: Rgba::rgb(0x4F, 0x8F, 0xC6),
            stroke_width: 1.2,
            size: 0.0,
            marker: Marker::Circle,
        });
    }
    if let Some(l) = cities.and_then(|id| map.layer_mut(id)) {
        let base = Symbol {
            fill: Rgba::new(0xE0, 0x6A, 0x3B, 220),
            stroke: Rgba::rgb(255, 255, 255),
            stroke_width: 0.8,
            size: 6.0,
            marker: Marker::Circle,
        };
        if let Ok(r) = Renderer::proportional(&l.dataset, &[], "pop_max", base) {
            l.renderer = r;
            if let Renderer::Proportional { min_size, max_size, .. } = &mut l.renderer {
                *min_size = 3.5;
                *max_size = 18.0;
            }
        }
        l.labels.default.field = "name".into();
        l.labels.default.size = 8.0;
        l.labels.default.bold = true;
        l.labels.default.color = Rgba::rgb(0x5A, 0x2A, 0x14);
        l.max_scale = None;
    }
    map.bookmarks = vec![
        Bookmark::new("World", cam(10.0, 15.0, 26_000.0)),
        Bookmark::new("Europe", cam(12.0, 50.0, 5_200.0)),
        Bookmark::new("Africa", cam(19.0, 2.0, 8_500.0)),
        Bookmark::new("Americas", cam(-80.0, 12.0, 15_000.0)),
        Bookmark::new("East Asia", cam(118.0, 32.0, 6_500.0)),
    ];
    map.camera = cam(10.0, 18.0, 26_000.0);
    p
}

/// Cambodia's official administrative boundaries at full resolution, unsimplified: the country
/// (ADM0), its 25 provinces (ADM1, English and Khmer names) and 197 districts (ADM2, with their
/// province). Bundled so KHGIS opens on Cambodia, and picks a province, without a network.
///
/// The source punches Tonle Sap out of the national outline; those interior rings are filled, so
/// the country boundary covers the lake. The lake itself is a layer of its own.
pub const CAMBODIA_COUNTRY: &str = include_str!("../data/cambodia_adm0.geojson");
pub const CAMBODIA_PROVINCES: &str = include_str!("../data/cambodia_adm1.geojson");
pub const CAMBODIA_DISTRICTS: &str = include_str!("../data/cambodia_adm2.geojson");
/// Tonle Sap, which the administrative boundaries leave as a gap between provinces.
pub const CAMBODIA_TONLESAP: &str = include_str!("../data/cambodia_tonlesap.geojson");

/// "Cambodia": KHGIS's home map. Satellite imagery under glowing province outlines labelled in
/// Khmer and English, districts drawn faintly, with a bookmark per region.
pub fn cambodia_project(khmer: bool) -> Project {
    let mut p = Project::new(if khmer { "កម្ពុជា" } else { "Cambodia" });
    let map = p.map_mut();
    map.name = if khmer { "កម្ពុជា".into() } else { "Cambodia".into() };
    map.basemap = rustgis_map::Basemap::Midnight;
    let mut imagery = rustgis_map::tiles::TileLayer::new(
        "Satellite (Esri World Imagery)",
        rustgis_map::tiles::TileKind::Raster,
        "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    );
    imagery.attribution = "Esri, Maxar, Earthstar Geographics, and the GIS User Community".into();
    imagery.max_zoom = 19;
    imagery.id = "esri-imagery".into();
    map.tiles.push(imagery);

    let mut add = |name: &str, text: &str| read_geojson(text, name).ok().map(|(ds, _)| map.add_dataset(ds));
    // Bottom to top: the lake sits in the gap the provinces leave, then districts, provinces, and
    // the national outline drawn over them.
    let lake = add(if khmer { "បឹងទន្លេសាប" } else { "Tonle Sap Lake" }, CAMBODIA_TONLESAP);
    let districts = add(if khmer { "ស្រុក/ខណ្ឌ" } else { "Districts" }, CAMBODIA_DISTRICTS);
    let provinces = add(if khmer { "ខេត្ត/រាជធានី" } else { "Provinces" }, CAMBODIA_PROVINCES);
    let country = add(if khmer { "ព្រំដែនជាតិ" } else { "National Boundary" }, CAMBODIA_COUNTRY);
    let cyan = Rgba::rgb(0x22, 0xD3, 0xEE);
    if let Some(l) = lake.and_then(|id| map.layer_mut(id)) {
        l.renderer = Renderer::Single(Symbol { fill: Rgba::new(0x38, 0xBD, 0xF8, 110), stroke: Rgba::new(0x7D, 0xD3, 0xFC, 180), stroke_width: 0.8, size: 0.0, marker: Marker::Circle });
    }
    if let Some(l) = country.and_then(|id| map.layer_mut(id)) {
        l.renderer = Renderer::Single(Symbol { fill: Rgba::new(0, 0, 0, 0), stroke: Rgba::new(0xE6, 0xFB, 0xFF, 220), stroke_width: 2.2, size: 0.0, marker: Marker::Circle });
    }
    if let Some(l) = districts.and_then(|id| map.layer_mut(id)) {
        l.renderer = Renderer::Single(Symbol { fill: Rgba::new(0, 0, 0, 0), stroke: Rgba::new(0xCF, 0xFA, 0xFE, 90), stroke_width: 0.5, size: 0.0, marker: Marker::Circle });
    }
    if let Some(l) = provinces.and_then(|id| map.layer_mut(id)) {
        l.renderer = Renderer::Single(Symbol { fill: Rgba::new(0x22, 0xD3, 0xEE, 18), stroke: cyan, stroke_width: 1.6, size: 0.0, marker: Marker::Circle });
        l.labels.enabled = true;
        l.labels.default.field = if khmer { "name_km".into() } else { "name".into() };
        l.labels.default.size = 10.0;
        l.labels.default.bold = true;
        l.labels.default.color = Rgba::rgb(0xE6, 0xFB, 0xFF);
        l.labels.default.halo = true;
        l.labels.default.halo_color = Some(Rgba::new(0x05, 0x0B, 0x1A, 220));
    }
    map.bookmarks = vec![
        Bookmark::new(if khmer { "កម្ពុជា" } else { "Cambodia" }, cam(104.95, 12.55, 820.0)),
        Bookmark::new(if khmer { "ភ្នំពេញ" } else { "Phnom Penh" }, cam(104.92, 11.56, 18.0)),
        Bookmark::new(if khmer { "សៀមរាប និងអង្គរ" } else { "Siem Reap & Angkor" }, cam(103.86, 13.41, 25.0)),
        Bookmark::new(if khmer { "បឹងទន្លេសាប" } else { "Tonle Sap" }, cam(104.1, 12.85, 160.0)),
        Bookmark::new(if khmer { "ឆ្នេរសមុទ្រ" } else { "Coast" }, cam(103.7, 10.8, 160.0)),
        Bookmark::new(if khmer { "ភូមិភាគឦសាន" } else { "Northeast" }, cam(107.0, 13.6, 220.0)),
    ];
    map.camera = cam(104.95, 12.55, 820.0);
    p
}

#[cfg(test)]
mod cambodia_tests {
    #[test]
    fn the_cambodia_project_has_provinces_districts_and_imagery() {
        for khmer in [false, true] {
            let p = super::cambodia_project(khmer);
            let m = &p.maps[0];
            assert_eq!(m.layers.len(), 2);
            assert_eq!(m.tiles.len(), 1);
            let provinces = m.layers.iter().find(|l| l.labels.enabled).unwrap();
            assert_eq!(provinces.dataset.features.len(), 25);
        }
    }
}
