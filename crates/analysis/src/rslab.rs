//! The remote sensing lab's image analysis (LEE RS Lab in KHGIS): training samples, seven
//! supervised classifiers (minimum distance, Mahalanobis distance, maximum likelihood, spectral
//! angle mapper, parallelepiped, k-nearest neighbours, random forest), ISODATA clustering,
//! spatial filters, the sieve, principal components, GLCM texture, from → to change matrices,
//! accuracy assessment with area-adjusted estimates and stratified sample points.
//!
//! Published methods, implemented here from their papers and textbooks: Richards & Jia, *Remote
//! Sensing Digital Image Analysis* (classifiers, ISODATA, PCA); Breiman (2001), *Random Forests*;
//! Haralick, Shanmugam & Dinstein (1973), *Textural Features for Image Classification* (GLCM);
//! Olofsson et al. (2014), *Good practices for estimating area and assessing accuracy of land
//! change* (area-adjusted accuracy and areas with confidence intervals).

// Matrix arithmetic (covariances, inverses, eigenvectors) reads clearest by index.
#![allow(clippy::needless_range_loop)]

use rustgis_data::raster::Raster;
use rustgis_geom::Coord;

use crate::rasterops::{NODATA, new_band, per_index, polygon_mask};

/// The most classes a class map may have (more is a continuous raster, not classes).
pub const MAX_CLASSES: usize = 256;

/// Each class's training pixels: its name and the pixels' values over the bands.
pub type Samples = Vec<(String, Vec<Vec<f64>>)>;

fn band(r: &Raster, b: usize) -> Result<&[f32], String> {
    r.bands.get(b).map(|b| b.data.as_slice()).ok_or_else(|| format!("{} has no band {}", r.name, b + 1))
}

fn bands_of<'a>(r: &'a Raster, bands: &[usize]) -> Result<Vec<&'a [f32]>, String> {
    if bands.is_empty() {
        return Err("give at least one band".into());
    }
    bands.iter().map(|b| band(r, *b)).collect()
}

/// Pixel `i`'s values over the bands into `buf`: false when any is no-data.
fn pixel_into(r: &Raster, data: &[&[f32]], i: usize, buf: &mut Vec<f64>) -> bool {
    buf.clear();
    for d in data {
        match d.get(i).copied().filter(|v| r.is_valid(*v) && *v != NODATA) {
            Some(v) => buf.push(f64::from(v)),
            None => return false,
        }
    }
    true
}

/// A small, fixed random number generator (xorshift64*): the same samples and trees every run.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

/// At most `most` of `v`, evenly spread (the order kept).
fn thin<T: Clone>(v: Vec<T>, most: usize) -> Vec<T> {
    if v.len() <= most || most == 0 {
        return v;
    }
    let step = v.len() as f64 / most as f64;
    (0..most).filter_map(|k| v.get((k as f64 * step) as usize).cloned()).collect()
}

/// The pixels under each class's training polygons (longitude/latitude rings), at most
/// `per_class` a class (spread evenly over them): classes in the order first met, those with no
/// pixel under them left out.
pub fn samples_from_polygons(
    r: &Raster,
    bands: &[usize],
    polygons: &[(String, Vec<Vec<Coord>>)],
    per_class: usize,
) -> Result<Samples, String> {
    let data = bands_of(r, bands)?;
    let w = r.width as usize;
    crate::control::begin(polygons.len());
    let mut out: Samples = Vec::new();
    let mut buf = Vec::with_capacity(data.len());
    for (k, (key, rings)) in polygons.iter().enumerate() {
        crate::control::check()?;
        crate::control::reached(k);
        let Some((cols, rows, mask)) = polygon_mask(r, std::slice::from_ref(rings))? else { continue };
        let mw = (cols.end - cols.start) as usize;
        let mut got = Vec::new();
        for (ri, row) in rows.clone().enumerate() {
            for ci in 0..mw {
                if mask.get(ri * mw + ci).copied().unwrap_or(false)
                    && pixel_into(r, &data, row as usize * w + cols.start as usize + ci, &mut buf)
                {
                    got.push(buf.clone());
                }
            }
        }
        if got.is_empty() {
            continue;
        }
        match out.iter_mut().find(|c| c.0 == *key) {
            Some(c) => c.1.extend(got),
            None => out.push((key.clone(), got)),
        }
    }
    crate::control::finish();
    Ok(out.into_iter().map(|(k, v)| (k, thin(v, per_class))).collect())
}

/// Training pixels from a class raster on `r`'s grid (`labels`, a value a pixel; no-data or 0 is
/// none), such as a land cover map: at most `per_class` a class, spread over the image, named by
/// `name`. Classes with fewer than `fewest` pixels are left out.
pub fn samples_from_labels(
    r: &Raster,
    bands: &[usize],
    labels: &[f32],
    per_class: usize,
    fewest: usize,
    name: impl Fn(f32) -> String,
) -> Result<Samples, String> {
    let data = bands_of(r, bands)?;
    let n = r.width as usize * r.height as usize;
    let mut by: Vec<(f32, Vec<usize>)> = Vec::new();
    for (i, l) in labels.iter().enumerate().take(n) {
        if *l == NODATA || !l.is_finite() || *l == 0.0 {
            continue;
        }
        let len = by.len();
        match by.iter_mut().find(|c| c.0 == *l) {
            Some(c) => c.1.push(i),
            None if len < MAX_CLASSES => by.push((*l, vec![i])),
            None => {
                return Err(format!("the training map has more than {MAX_CLASSES} different values: it isn't a class map (land cover)"));
            }
        }
    }
    crate::control::check()?;
    by.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut buf = Vec::with_capacity(data.len());
    let mut rng = Rng::new(0x5EED);
    for (v, mut idx) in by {
        // Shuffled before thinning, so the pixels come from all over the image.
        for i in (1..idx.len()).rev() {
            idx.swap(i, rng.below(i + 1));
        }
        let pix: Vec<Vec<f64>> = idx
            .into_iter()
            .filter_map(|i| if pixel_into(r, &data, i, &mut buf) { Some(buf.clone()) } else { None })
            .take(per_class.max(1))
            .collect();
        if pix.len() >= fewest.max(1) {
            out.push((name(v), pix));
        }
    }
    Ok(out)
}

/// A supervised classifier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
    MinDistance,
    Mahalanobis,
    MaxLikelihood,
    /// Spectral angle mapper.
    Sam,
    /// Boxes of the mean ± this many standard deviations; pixels in none stay unclassified.
    Parallelepiped(f64),
    /// k nearest neighbours (bands scaled to their spread).
    Knn(usize),
    /// A random forest of this many trees.
    RandomForest(usize),
}

impl Method {
    /// `name` (as the command takes it), with its option.
    pub fn parse(name: &str, option: Option<f64>) -> Result<Method, String> {
        Ok(match name {
            "minDistance" => Method::MinDistance,
            "mahalanobis" => Method::Mahalanobis,
            "maxLikelihood" => Method::MaxLikelihood,
            "sam" => Method::Sam,
            "parallelepiped" => Method::Parallelepiped(option.unwrap_or(2.0).clamp(0.5, 5.0)),
            "knn" => Method::Knn(option.unwrap_or(5.0).clamp(1.0, 25.0) as usize),
            "randomForest" => Method::RandomForest(option.unwrap_or(50.0).clamp(5.0, 300.0) as usize),
            o => {
                return Err(format!(
                    "unknown method '{o}': minDistance, mahalanobis, maxLikelihood, sam, parallelepiped, knn or randomForest"
                ));
            }
        })
    }
}

/// One tree of a random forest: nodes in an array, leaves holding a class.
#[derive(Clone, Debug)]
enum Node {
    Split { feature: usize, at: f64, left: usize, right: usize },
    Leaf(usize),
}

#[derive(Clone, Debug)]
enum Kind {
    Means(Vec<Vec<f64>>),
    Gauss { means: Vec<Vec<f64>>, inv: Vec<Vec<Vec<f64>>>, logdet: Vec<f64>, likelihood: bool },
    Sam(Vec<Vec<f64>>),
    Boxes { lo: Vec<Vec<f64>>, hi: Vec<Vec<f64>>, means: Vec<Vec<f64>> },
    Knn { k: usize, scale: Vec<f64>, points: Vec<(Vec<f64>, usize)> },
    Forest(Vec<Vec<Node>>),
}

/// A trained classifier: the classes (in order: value 1, 2, …) and what it learnt.
#[derive(Clone, Debug)]
pub struct Model {
    pub classes: Vec<String>,
    kind: Kind,
    /// A random forest's importance of each band (summing to 1), else empty.
    pub importance: Vec<f64>,
    /// Pixels classified as unclassified (value 0) possible: the parallelepiped's.
    pub leaves_unclassified: bool,
}

fn mean(v: &[Vec<f64>], d: usize) -> Vec<f64> {
    let mut m = vec![0.0; d];
    for p in v {
        for (a, b) in m.iter_mut().zip(p) {
            *a += b;
        }
    }
    let n = v.len().max(1) as f64;
    m.iter_mut().for_each(|a| *a /= n);
    m
}

fn covariance(v: &[Vec<f64>], m: &[f64]) -> Vec<Vec<f64>> {
    let d = m.len();
    let mut c = vec![vec![0.0; d]; d];
    for p in v {
        for i in 0..d {
            for j in i..d {
                let x = (p.get(i).copied().unwrap_or(0.0) - m[i]) * (p.get(j).copied().unwrap_or(0.0) - m[j]);
                c[i][j] += x;
            }
        }
    }
    let n = (v.len().max(2) - 1) as f64;
    for i in 0..d {
        for j in i..d {
            c[i][j] /= n;
            c[j][i] = c[i][j];
        }
    }
    c
}

/// The inverse and log-determinant of a symmetric matrix, a little ridge added so a class with
/// too few or too alike pixels still has one.
fn invert(c: &[Vec<f64>]) -> (Vec<Vec<f64>>, f64) {
    let d = c.len();
    let trace: f64 = (0..d).map(|i| c[i][i]).sum::<f64>().abs();
    let ridge = (trace / d.max(1) as f64 * 1e-4).max(1e-9);
    let mut a: Vec<Vec<f64>> = c.to_vec();
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += ridge;
    }
    let mut inv: Vec<Vec<f64>> = (0..d).map(|i| (0..d).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    let mut logdet = 0.0;
    for col in 0..d {
        let piv = (col..d).max_by(|x, y| a[*x][col].abs().total_cmp(&a[*y][col].abs())).unwrap_or(col);
        a.swap(col, piv);
        inv.swap(col, piv);
        let p = a[col][col];
        if p.abs() < 1e-300 {
            return ((0..d).map(|i| (0..d).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect(), 0.0);
        }
        logdet += p.abs().ln();
        for j in 0..d {
            a[col][j] /= p;
            inv[col][j] /= p;
        }
        for i in 0..d {
            if i != col {
                let f = a[i][col];
                if f != 0.0 {
                    for j in 0..d {
                        a[i][j] -= f * a[col][j];
                        inv[i][j] -= f * inv[col][j];
                    }
                }
            }
        }
    }
    (inv, logdet)
}

fn quad(inv: &[Vec<f64>], x: &[f64], m: &[f64]) -> f64 {
    let d = m.len();
    let mut s = 0.0;
    for i in 0..d {
        let di = x.get(i).copied().unwrap_or(0.0) - m[i];
        for j in 0..d {
            s += di * inv[i][j] * (x.get(j).copied().unwrap_or(0.0) - m[j]);
        }
    }
    s
}

fn gini(counts: &[usize], n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    1.0 - counts.iter().map(|c| (*c as f64 / n as f64).powi(2)).sum::<f64>()
}

/// Grows one tree on the samples `idx` (bootstrapped), `mtry` bands tried at each split.
fn grow(x: &[Vec<f64>], y: &[usize], idx: Vec<usize>, k: usize, mtry: usize, rng: &mut Rng, importance: &mut [f64]) -> Vec<Node> {
    const MAX_DEPTH: usize = 18;
    let d = x.first().map_or(0, Vec::len);
    let mut nodes: Vec<Node> = Vec::new();
    // (node index to fill, its samples, depth)
    let mut stack: Vec<(usize, Vec<usize>, usize)> = vec![(0, idx, 0)];
    nodes.push(Node::Leaf(0));
    while let Some((at, s, depth)) = stack.pop() {
        let mut counts = vec![0usize; k];
        for i in &s {
            if let Some(c) = y.get(*i).and_then(|c| counts.get_mut(*c)) {
                *c += 1;
            }
        }
        let majority = counts.iter().enumerate().max_by_key(|c| *c.1).map_or(0, |c| c.0);
        let parent = gini(&counts, s.len());
        if depth >= MAX_DEPTH || s.len() < 2 || parent == 0.0 {
            nodes[at] = Node::Leaf(majority);
            continue;
        }
        // The best split over `mtry` bands picked at random.
        let mut best: Option<(f64, usize, f64)> = None;
        let mut order = s.clone();
        for _ in 0..mtry.min(d).max(1) {
            let f = rng.below(d);
            order.sort_by(|a, b| x[*a].get(f).copied().unwrap_or(0.0).total_cmp(&x[*b].get(f).copied().unwrap_or(0.0)));
            let mut left = vec![0usize; k];
            let mut right = counts.clone();
            for w in 0..order.len().saturating_sub(1) {
                let c = y.get(order[w]).copied().unwrap_or(0);
                if let (Some(l), Some(r)) = (left.get_mut(c), right.get_mut(c)) {
                    *l += 1;
                    *r = r.saturating_sub(1);
                }
                let (a, b) = (x[order[w]].get(f).copied().unwrap_or(0.0), x[order[w + 1]].get(f).copied().unwrap_or(0.0));
                if a == b {
                    continue;
                }
                let (nl, nr) = (w + 1, order.len() - w - 1);
                let g = (nl as f64 * gini(&left, nl) + nr as f64 * gini(&right, nr)) / order.len() as f64;
                if best.is_none_or(|bb| g < bb.0) {
                    best = Some((g, f, (a + b) / 2.0));
                }
            }
        }
        let Some((g, feature, split)) = best.filter(|b| b.0 < parent) else {
            nodes[at] = Node::Leaf(majority);
            continue;
        };
        if let Some(imp) = importance.get_mut(feature) {
            *imp += (parent - g) * s.len() as f64;
        }
        let (l, r): (Vec<usize>, Vec<usize>) = s.into_iter().partition(|i| x[*i].get(feature).copied().unwrap_or(0.0) <= split);
        let (li, ri) = (nodes.len(), nodes.len() + 1);
        nodes.push(Node::Leaf(majority));
        nodes.push(Node::Leaf(majority));
        nodes[at] = Node::Split { feature, at: split, left: li, right: ri };
        stack.push((li, l, depth + 1));
        stack.push((ri, r, depth + 1));
    }
    nodes
}

/// Most training pixels a class keeps for k nearest neighbours (each pixel is compared with all).
const KNN_PER_CLASS: usize = 250;

/// Trains `method` on the samples (two classes at least).
pub fn train(samples: &Samples, method: Method) -> Result<Model, String> {
    let samples: Vec<&(String, Vec<Vec<f64>>)> = samples.iter().filter(|c| !c.1.is_empty()).collect();
    if samples.len() < 2 {
        return Err("the training data need at least two classes with pixels".into());
    }
    let d = samples.first().and_then(|c| c.1.first()).map_or(0, Vec::len);
    if d == 0 {
        return Err("give at least one band".into());
    }
    let classes: Vec<String> = samples.iter().map(|c| c.0.clone()).collect();
    let means: Vec<Vec<f64>> = samples.iter().map(|c| mean(&c.1, d)).collect();
    let mut importance = Vec::new();
    let kind = match method {
        Method::MinDistance => Kind::Means(means),
        Method::Mahalanobis | Method::MaxLikelihood => {
            let likelihood = method == Method::MaxLikelihood;
            let mut inv = Vec::new();
            let mut logdet = Vec::new();
            if likelihood {
                for (c, m) in samples.iter().zip(&means) {
                    let (i, l) = invert(&covariance(&c.1, m));
                    inv.push(i);
                    logdet.push(l);
                }
            } else {
                // Mahalanobis: one covariance, pooled over the classes.
                let mut pooled = vec![vec![0.0; d]; d];
                let mut total = 0.0;
                for (c, m) in samples.iter().zip(&means) {
                    let cv = covariance(&c.1, m);
                    let w = c.1.len() as f64;
                    for i in 0..d {
                        for j in 0..d {
                            pooled[i][j] += cv[i][j] * w;
                        }
                    }
                    total += w;
                }
                pooled.iter_mut().flatten().for_each(|v| *v /= total.max(1.0));
                let (i, l) = invert(&pooled);
                inv = vec![i; means.len()];
                logdet = vec![l; means.len()];
            }
            Kind::Gauss { means, inv, logdet, likelihood }
        }
        Method::Sam => Kind::Sam(means),
        Method::Parallelepiped(k) => {
            let (mut lo, mut hi) = (Vec::new(), Vec::new());
            for (c, m) in samples.iter().zip(&means) {
                let cv = covariance(&c.1, m);
                lo.push((0..d).map(|i| m[i] - k * cv[i][i].sqrt()).collect());
                hi.push((0..d).map(|i| m[i] + k * cv[i][i].sqrt()).collect());
            }
            Kind::Boxes { lo, hi, means }
        }
        Method::Knn(k) => {
            let all: Vec<&Vec<f64>> = samples.iter().flat_map(|c| c.1.iter()).collect();
            let m = mean(&all.iter().map(|v| (*v).clone()).collect::<Vec<_>>(), d);
            let scale: Vec<f64> = (0..d)
                .map(|i| {
                    let var = all.iter().map(|p| (p.get(i).copied().unwrap_or(0.0) - m[i]).powi(2)).sum::<f64>() / all.len().max(1) as f64;
                    if var > 0.0 { 1.0 / var.sqrt() } else { 1.0 }
                })
                .collect();
            let points = samples
                .iter()
                .enumerate()
                .flat_map(|(ci, c)| {
                    let scale = &scale;
                    thin(c.1.clone(), KNN_PER_CLASS).into_iter().map(move |p| (p.iter().zip(scale).map(|(a, s)| a * s).collect(), ci))
                })
                .collect();
            Kind::Knn { k: k.max(1), scale, points }
        }
        Method::RandomForest(trees) => {
            let mut x = Vec::new();
            let mut y = Vec::new();
            for (ci, c) in samples.iter().enumerate() {
                for p in &c.1 {
                    x.push(p.clone());
                    y.push(ci);
                }
            }
            let mtry = ((d as f64).sqrt().ceil() as usize).max(1);
            let mut rng = Rng::new(0x00C0_FFEE);
            let mut imp = vec![0.0; d];
            let mut forest = Vec::with_capacity(trees);
            crate::control::begin(trees);
            for t in 0..trees {
                crate::control::check()?;
                crate::control::reached(t);
                let idx: Vec<usize> = (0..x.len()).map(|_| rng.below(x.len())).collect();
                forest.push(grow(&x, &y, idx, classes.len(), mtry, &mut rng, &mut imp));
            }
            let total: f64 = imp.iter().sum();
            importance = imp.iter().map(|v| if total > 0.0 { v / total } else { 0.0 }).collect();
            Kind::Forest(forest)
        }
    };
    Ok(Model { classes, kind, importance, leaves_unclassified: matches!(method, Method::Parallelepiped(_)) })
}

fn nearest(means: &[Vec<f64>], p: &[f64]) -> Option<usize> {
    means
        .iter()
        .enumerate()
        .map(|(i, m)| (i, m.iter().zip(p).map(|(a, b)| (a - b).powi(2)).sum::<f64>()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|x| x.0)
}

/// The class (its index) the model gives the pixel `p`, or none (unclassified).
pub fn predict(model: &Model, p: &[f64]) -> Option<usize> {
    match &model.kind {
        Kind::Means(m) => nearest(m, p),
        Kind::Gauss { means, inv, logdet, likelihood } => means
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let q = inv.get(i).map_or(f64::INFINITY, |inv| quad(inv, p, m));
                (i, if *likelihood { q + logdet.get(i).copied().unwrap_or(0.0) } else { q })
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0),
        Kind::Sam(means) => {
            let np = p.iter().map(|v| v * v).sum::<f64>().sqrt();
            means
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let nm = m.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let dot: f64 = m.iter().zip(p).map(|(a, b)| a * b).sum();
                    (i, if np > 0.0 && nm > 0.0 { (dot / (np * nm)).clamp(-1.0, 1.0).acos() } else { f64::INFINITY })
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|x| x.0)
        }
        Kind::Boxes { lo, hi, means } => {
            let inside: Vec<usize> = (0..lo.len())
                .filter(|c| {
                    p.iter()
                        .enumerate()
                        .all(|(i, v)| *v >= lo[*c].get(i).copied().unwrap_or(0.0) && *v <= hi[*c].get(i).copied().unwrap_or(0.0))
                })
                .collect();
            // Where boxes overlap, the nearest mean of those it falls in.
            inside.into_iter().min_by(|a, b| {
                let d = |c: usize| means[c].iter().zip(p).map(|(m, v)| (m - v).powi(2)).sum::<f64>();
                d(*a).total_cmp(&d(*b))
            })
        }
        Kind::Knn { k, scale, points } => {
            let q: Vec<f64> = p.iter().zip(scale).map(|(a, s)| a * s).collect();
            let mut best: Vec<(f64, usize)> = Vec::with_capacity(k + 1);
            for (pt, c) in points {
                let dd: f64 = pt.iter().zip(&q).map(|(a, b)| (a - b).powi(2)).sum();
                if best.len() < *k || best.last().is_some_and(|l| dd < l.0) {
                    let at = best.partition_point(|b| b.0 <= dd);
                    best.insert(at, (dd, *c));
                    best.truncate(*k);
                }
            }
            let mut votes = vec![0usize; model.classes.len()];
            for (_, c) in &best {
                if let Some(v) = votes.get_mut(*c) {
                    *v += 1;
                }
            }
            votes.iter().enumerate().max_by_key(|v| *v.1).map(|v| v.0)
        }
        Kind::Forest(trees) => {
            let mut votes = vec![0usize; model.classes.len()];
            for t in trees {
                let mut at = 0;
                while let Some(n) = t.get(at) {
                    match n {
                        Node::Leaf(c) => {
                            if let Some(v) = votes.get_mut(*c) {
                                *v += 1;
                            }
                            break;
                        }
                        Node::Split { feature, at: s, left, right } => {
                            at = if p.get(*feature).copied().unwrap_or(0.0) <= *s { *left } else { *right };
                        }
                    }
                }
            }
            votes.iter().enumerate().max_by_key(|v| *v.1).map(|v| v.0)
        }
    }
}

/// Classifies every pixel: its class's value (1, 2, … in the model's order), 0 unclassified,
/// no-data where a band has none.
pub fn classify(r: &Raster, bands: &[usize], model: &Model) -> Result<Vec<f32>, String> {
    let data = bands_of(r, bands)?;
    let n = r.width as usize * r.height as usize;
    let cost = match &model.kind {
        Kind::Knn { points, .. } => points.len().saturating_mul(data.len()) / 4 + 1,
        Kind::Forest(t) => t.len() * 2,
        _ => model.classes.len().saturating_mul(data.len() * data.len()).max(1),
    };
    per_index(
        n,
        r.width as usize,
        cost,
        || Vec::with_capacity(data.len()),
        |buf: &mut Vec<f64>, i| {
            if !pixel_into(r, &data, i, buf) {
                return NODATA;
            }
            predict(model, buf).map_or(0.0, |c| (c + 1) as f32)
        },
    )
}

/// The share of the training pixels the model classifies as their own class (resubstitution:
/// optimistic, as the pixels taught it).
pub fn training_accuracy(model: &Model, samples: &Samples) -> f64 {
    let (mut right, mut all) = (0usize, 0usize);
    for (name, pix) in samples {
        let Some(c) = model.classes.iter().position(|n| n == name) else { continue };
        for p in pix.iter().take(2000) {
            all += 1;
            if predict(model, p) == Some(c) {
                right += 1;
            }
        }
    }
    if all == 0 { 0.0 } else { right as f64 / all as f64 }
}

/// ISODATA clustering: k-means whose clusters are split when too spread out and merged when too
/// close, ending with `min_k` to `max_k` clusters; values 1.. numbered by the first band's
/// centre, low to high.
pub fn isodata(r: &Raster, bands: &[usize], min_k: usize, max_k: usize, iterations: usize) -> Result<Vec<f32>, String> {
    let data = bands_of(r, bands)?;
    let (min_k, max_k) = (min_k.clamp(2, 50), max_k.clamp(2, 50).max(min_k.clamp(2, 50)));
    let n = r.width as usize * r.height as usize;
    let mut buf = Vec::with_capacity(data.len());
    let sample: Vec<Vec<f64>> = (0..n)
        .step_by((n / 20_000).max(1))
        .filter_map(|i| if pixel_into(r, &data, i, &mut buf) { Some(buf.clone()) } else { None })
        .collect();
    if sample.len() < max_k * 2 {
        return Err("too few valid pixels to classify".into());
    }
    let d = data.len();
    let min_size = (sample.len() / (max_k * 10)).max(2);
    // Start with the midpoint of the range, spread along the first band.
    let start = (min_k + max_k) / 2;
    let mut sorted = sample.clone();
    sorted.sort_by(|a, b| a.first().copied().unwrap_or(0.0).total_cmp(&b.first().copied().unwrap_or(0.0)));
    let mut centres: Vec<Vec<f64>> = (0..start).filter_map(|i| sorted.get((i * 2 + 1) * sorted.len() / (2 * start)).cloned()).collect();
    // The spread of the whole sample sets the split and merge thresholds.
    let all_mean = mean(&sample, d);
    let all_sd: f64 = (0..d).map(|i| covariance(&sample, &all_mean)[i][i].sqrt()).sum::<f64>() / d as f64;
    let split_sd = all_sd * 0.5;
    let merge_dist = all_sd * 0.5;
    crate::control::begin(iterations.max(1));
    for it in 0..iterations.clamp(2, 50) {
        crate::control::check()?;
        crate::control::reached(it);
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); centres.len()];
        for (i, p) in sample.iter().enumerate() {
            if let Some(c) = nearest(&centres, p)
                && let Some(m) = members.get_mut(c)
            {
                m.push(i);
            }
        }
        // Small clusters dropped, centres moved to their members' mean.
        let mut next: Vec<(Vec<f64>, Vec<usize>)> = members
            .into_iter()
            .filter(|m| m.len() >= min_size)
            .map(|m| (mean(&m.iter().map(|i| sample[*i].clone()).collect::<Vec<_>>(), d), m))
            .collect();
        if next.is_empty() {
            break;
        }
        let last = it + 1 == iterations.clamp(2, 50);
        if !last && (next.len() < max_k && (it % 2 == 0 || next.len() < min_k)) {
            // Split the most spread-out cluster(s) along their widest band.
            let mut split = Vec::new();
            for (c, m) in &next {
                let pts: Vec<Vec<f64>> = m.iter().map(|i| sample[*i].clone()).collect();
                let cv = covariance(&pts, c);
                let (band, sd) = (0..d).map(|i| (i, cv[i][i].sqrt())).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, 0.0));
                if sd > split_sd && m.len() > 2 * min_size && next.len() + split.len() < max_k {
                    let mut a = c.clone();
                    let mut b = c.clone();
                    a[band] -= sd * 0.5;
                    b[band] += sd * 0.5;
                    split.push((a, b));
                } else {
                    split.push((c.clone(), Vec::new()));
                }
            }
            centres = split.into_iter().flat_map(|(a, b)| if b.is_empty() { vec![a] } else { vec![a, b] }).collect();
        } else if !last && next.len() > min_k {
            // Merge the closest pair when closer than the threshold.
            let mut pair: Option<(usize, usize, f64)> = None;
            for i in 0..next.len() {
                for j in i + 1..next.len() {
                    let dd = next[i].0.iter().zip(&next[j].0).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
                    if pair.is_none_or(|p| dd < p.2) {
                        pair = Some((i, j, dd));
                    }
                }
            }
            if let Some((i, j, dd)) = pair.filter(|p| p.2 < merge_dist) {
                let _ = dd;
                let (ni, nj) = (next[i].1.len() as f64, next[j].1.len() as f64);
                let merged: Vec<f64> = next[i].0.iter().zip(&next[j].0).map(|(a, b)| (a * ni + b * nj) / (ni + nj)).collect();
                next[i].0 = merged;
                next.remove(j);
            }
            centres = next.into_iter().map(|c| c.0).collect();
        } else {
            centres = next.into_iter().map(|c| c.0).collect();
        }
    }
    crate::control::finish();
    centres.sort_by(|a, b| a.first().copied().unwrap_or(0.0).total_cmp(&b.first().copied().unwrap_or(0.0)));
    let cost = centres.len().saturating_mul(d).max(1);
    per_index(
        n,
        r.width as usize,
        cost,
        || Vec::with_capacity(d),
        |buf: &mut Vec<f64>, i| {
            if !pixel_into(r, &data, i, buf) {
                return NODATA;
            }
            nearest(&centres, buf).map_or(NODATA, |c| (c + 1) as f32)
        },
    )
}

/// A spatial filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Mean,
    Median,
    Gaussian,
    /// The image plus its detail (itself less its mean).
    Sharpen,
    Sobel,
    Laplacian,
    Emboss,
    /// The most common value: cleans a class map.
    Majority,
    Min,
    Max,
}

impl Filter {
    pub fn parse(s: &str) -> Result<Filter, String> {
        Ok(match s {
            "mean" => Filter::Mean,
            "median" => Filter::Median,
            "gaussian" => Filter::Gaussian,
            "sharpen" => Filter::Sharpen,
            "sobel" => Filter::Sobel,
            "laplacian" => Filter::Laplacian,
            "emboss" => Filter::Emboss,
            "majority" => Filter::Majority,
            "min" => Filter::Min,
            "max" => Filter::Max,
            o => {
                return Err(format!(
                    "unknown filter '{o}': mean, median, gaussian, sharpen, sobel, laplacian, emboss, majority, min or max"
                ));
            }
        })
    }
}

/// Band `b` filtered in a `size` × `size` window (odd, 3 to 15); edge filters use 3 × 3.
pub fn filter(r: &Raster, b: usize, kind: Filter, size: usize) -> Result<Vec<f32>, String> {
    let data = band(r, b)?;
    let (w, h) = (r.width as i64, r.height as i64);
    let size = (size.clamp(3, 15) | 1) as i64;
    let half = size / 2;
    let at = |c: i64, rr: i64| -> Option<f64> {
        if c < 0 || rr < 0 || c >= w || rr >= h {
            return None;
        }
        data.get((rr * w + c) as usize).copied().filter(|v| r.is_valid(*v) && *v != NODATA).map(f64::from)
    };
    let sigma = (size as f64 / 3.0).max(0.5);
    let gauss: Vec<f64> = (-half..=half).map(|k| (-(k * k) as f64 / (2.0 * sigma * sigma)).exp()).collect();
    let n = (w * h) as usize;
    per_index(n, w as usize, (size * size) as usize, Vec::new, |vals: &mut Vec<f64>, i| {
        let (c, rr) = (i as i64 % w.max(1), i as i64 / w.max(1));
        let Some(centre) = at(c, rr) else { return NODATA };
        let out = match kind {
            Filter::Sobel | Filter::Laplacian | Filter::Emboss => {
                let g = |dc: i64, dr: i64| at(c + dc, rr + dr).unwrap_or(centre);
                match kind {
                    Filter::Sobel => {
                        let gx = g(1, -1) + 2.0 * g(1, 0) + g(1, 1) - g(-1, -1) - 2.0 * g(-1, 0) - g(-1, 1);
                        let gy = g(-1, 1) + 2.0 * g(0, 1) + g(1, 1) - g(-1, -1) - 2.0 * g(0, -1) - g(1, -1);
                        gx.hypot(gy)
                    }
                    Filter::Laplacian => 4.0 * centre - g(1, 0) - g(-1, 0) - g(0, 1) - g(0, -1),
                    _ => centre + g(1, 1) - g(-1, -1),
                }
            }
            _ => {
                vals.clear();
                let (mut sum, mut wsum) = (0.0, 0.0);
                for dr in -half..=half {
                    for dc in -half..=half {
                        if let Some(v) = at(c + dc, rr + dr) {
                            vals.push(v);
                            let wt = if kind == Filter::Gaussian {
                                gauss.get((dc + half) as usize).copied().unwrap_or(0.0)
                                    * gauss.get((dr + half) as usize).copied().unwrap_or(0.0)
                            } else {
                                1.0
                            };
                            sum += v * wt;
                            wsum += wt;
                        }
                    }
                }
                let mean = if wsum > 0.0 { sum / wsum } else { centre };
                match kind {
                    Filter::Mean | Filter::Gaussian => mean,
                    Filter::Sharpen => centre + (centre - mean),
                    Filter::Median => {
                        vals.sort_by(f64::total_cmp);
                        vals.get(vals.len() / 2).copied().unwrap_or(centre)
                    }
                    Filter::Min => vals.iter().copied().fold(f64::INFINITY, f64::min),
                    Filter::Max => vals.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => {
                        // Majority: the most common value, ties kept as the centre's.
                        vals.sort_by(f64::total_cmp);
                        let (mut best, mut best_n, mut run_v, mut run_n) = (centre, 0usize, f64::NAN, 0usize);
                        for v in vals.iter() {
                            if *v == run_v {
                                run_n += 1;
                            } else {
                                run_v = *v;
                                run_n = 1;
                            }
                            if run_n > best_n || (run_n == best_n && run_v == centre) {
                                best = run_v;
                                best_n = run_n;
                            }
                        }
                        best
                    }
                }
            }
        };
        out as f32
    })
}

/// The sieve: regions of one class (8-connected) smaller than `min_pixels` take the class most
/// common along their edge. Returns the cleaned classes and how many pixels changed.
pub fn sieve(r: &Raster, b: usize, min_pixels: usize) -> Result<(Vec<f32>, usize), String> {
    let data = band(r, b)?;
    let (w, h) = (r.width as usize, r.height as usize);
    let mut out = data.to_vec();
    let mut label = new_band(w, h, 0u32)?;
    let mut next = 1u32;
    let mut changed = 0usize;
    let valid = |v: f32| v != NODATA && v.is_finite() && r.is_valid(v);
    crate::control::begin(h);
    let mut stack = Vec::new();
    let mut region = Vec::new();
    for start in 0..w * h {
        if start % w.max(1) == 0 {
            crate::control::check()?;
            crate::control::reached(start / w.max(1));
        }
        let v = data.get(start).copied().unwrap_or(NODATA);
        if label.get(start).copied().unwrap_or(1) != 0 || !valid(v) {
            continue;
        }
        region.clear();
        stack.push(start);
        if let Some(l) = label.get_mut(start) {
            *l = next;
        }
        while let Some(i) = stack.pop() {
            region.push(i);
            let (c, rr) = ((i % w) as i64, (i / w) as i64);
            for dr in -1..=1i64 {
                for dc in -1..=1i64 {
                    let (nc, nr) = (c + dc, rr + dr);
                    if nc < 0 || nr < 0 || nc >= w as i64 || nr >= h as i64 {
                        continue;
                    }
                    let j = nr as usize * w + nc as usize;
                    if label.get(j) == Some(&0) && data.get(j) == Some(&v) {
                        if let Some(l) = label.get_mut(j) {
                            *l = next;
                        }
                        stack.push(j);
                    }
                }
            }
        }
        if region.len() < min_pixels {
            // The class most common next to the region (in the output so far).
            let mut around: Vec<(f32, usize)> = Vec::new();
            for &i in &region {
                let (c, rr) = ((i % w) as i64, (i / w) as i64);
                for (dc, dr) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let (nc, nr) = (c + dc, rr + dr);
                    if nc < 0 || nr < 0 || nc >= w as i64 || nr >= h as i64 {
                        continue;
                    }
                    let j = nr as usize * w + nc as usize;
                    let o = out.get(j).copied().unwrap_or(NODATA);
                    if label.get(j) != Some(&next) && valid(o) && o != v {
                        match around.iter_mut().find(|a| a.0 == o) {
                            Some(a) => a.1 += 1,
                            None => around.push((o, 1)),
                        }
                    }
                }
            }
            if let Some((to, _)) = around.iter().max_by_key(|a| a.1) {
                for &i in &region {
                    if let Some(o) = out.get_mut(i) {
                        *o = *to;
                    }
                }
                changed += region.len();
            }
        }
        next = next.wrapping_add(1).max(1);
    }
    crate::control::finish();
    Ok((out, changed))
}

/// Principal components of `bands`: the components (as many as asked, largest first), each
/// component's eigenvalue and share of the variance, and the loadings (a row a component).
pub struct Pca {
    pub components: Vec<Vec<f32>>,
    pub eigenvalues: Vec<f64>,
    pub explained: Vec<f64>,
    pub loadings: Vec<Vec<f64>>,
}

/// Eigenvalues and eigenvectors (columns) of a symmetric matrix, by Jacobi rotations.
fn eigen(a: &[Vec<f64>]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let d = a.len();
    let mut m = a.to_vec();
    let mut v: Vec<Vec<f64>> = (0..d).map(|i| (0..d).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    for _ in 0..100 {
        let mut off = 0.0;
        for i in 0..d {
            for j in i + 1..d {
                off += m[i][j] * m[i][j];
            }
        }
        if off < 1e-18 {
            break;
        }
        for p in 0..d {
            for q in p + 1..d {
                if m[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..d {
                    let (mkp, mkq) = (m[k][p], m[k][q]);
                    m[k][p] = c * mkp - s * mkq;
                    m[k][q] = s * mkp + c * mkq;
                }
                for k in 0..d {
                    let (mpk, mqk) = (m[p][k], m[q][k]);
                    m[p][k] = c * mpk - s * mqk;
                    m[q][k] = s * mpk + c * mqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    ((0..d).map(|i| m[i][i]).collect(), v)
}

pub fn pca(r: &Raster, bands: &[usize], components: usize) -> Result<Pca, String> {
    let data = bands_of(r, bands)?;
    if data.len() < 2 {
        return Err("principal components need two bands or more".into());
    }
    let d = data.len();
    let n = r.width as usize * r.height as usize;
    let mut buf = Vec::with_capacity(d);
    let sample: Vec<Vec<f64>> = (0..n)
        .step_by((n / 50_000).max(1))
        .filter_map(|i| if pixel_into(r, &data, i, &mut buf) { Some(buf.clone()) } else { None })
        .collect();
    if sample.len() < d + 1 {
        return Err("too few valid pixels for principal components".into());
    }
    crate::control::check()?;
    let m = mean(&sample, d);
    let (vals, vecs) = eigen(&covariance(&sample, &m));
    let mut order: Vec<usize> = (0..d).collect();
    order.sort_by(|a, b| vals[*b].total_cmp(&vals[*a]));
    let total: f64 = vals.iter().map(|v| v.max(0.0)).sum();
    let k = components.clamp(1, d);
    let loadings: Vec<Vec<f64>> = order.iter().take(k).map(|c| (0..d).map(|i| vecs[i][*c]).collect()).collect();
    let mut comps = Vec::with_capacity(k);
    for l in &loadings {
        crate::control::check()?;
        comps.push(per_index(
            n,
            r.width as usize,
            d,
            || Vec::with_capacity(d),
            |b: &mut Vec<f64>, i| {
                if !pixel_into(r, &data, i, b) {
                    return NODATA;
                }
                b.iter().zip(&m).zip(l).map(|((v, mu), w)| (v - mu) * w).sum::<f64>() as f32
            },
        )?);
    }
    let eigenvalues: Vec<f64> = order.iter().take(k).map(|c| vals[*c]).collect();
    let explained = eigenvalues.iter().map(|v| if total > 0.0 { v.max(0.0) / total * 100.0 } else { 0.0 }).collect();
    Ok(Pca { components: comps, eigenvalues, explained, loadings })
}

/// A GLCM texture measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    Contrast,
    Dissimilarity,
    Homogeneity,
    Entropy,
    /// Angular second moment (energy).
    Asm,
    Mean,
    Variance,
}

impl Texture {
    pub fn parse(s: &str) -> Result<Texture, String> {
        Ok(match s {
            "contrast" => Texture::Contrast,
            "dissimilarity" => Texture::Dissimilarity,
            "homogeneity" => Texture::Homogeneity,
            "entropy" => Texture::Entropy,
            "asm" => Texture::Asm,
            "mean" => Texture::Mean,
            "variance" => Texture::Variance,
            o => return Err(format!("unknown texture '{o}': contrast, dissimilarity, homogeneity, entropy, asm, mean or variance")),
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Texture::Contrast => "contrast",
            Texture::Dissimilarity => "dissimilarity",
            Texture::Homogeneity => "homogeneity",
            Texture::Entropy => "entropy",
            Texture::Asm => "asm",
            Texture::Mean => "mean",
            Texture::Variance => "variance",
        }
    }
}

/// Grey levels the texture is counted in.
const LEVELS: u32 = 32;

/// GLCM texture of band `b` in a `window` × `window` window (odd, 3 to 11): grey levels from its
/// 2nd to 98th percentile, pairs one pixel apart across and down, both ways round.
pub fn texture(r: &Raster, b: usize, window: usize, measures: &[Texture]) -> Result<Vec<Vec<f32>>, String> {
    let data = band(r, b)?;
    let (w, h) = (r.width as i64, r.height as i64);
    let n = (w * h) as usize;
    let mut vals: Vec<f32> = data.iter().step_by((n / 50_000).max(1)).copied().filter(|v| r.is_valid(*v) && *v != NODATA).collect();
    if vals.len() < 10 {
        return Err("too few valid pixels for texture".into());
    }
    vals.sort_by(f32::total_cmp);
    let (lo, hi) = (vals[vals.len() * 2 / 100], vals[vals.len() * 98 / 100]);
    let span = if hi > lo { hi - lo } else { 1.0 };
    let level = |v: f32| -> u32 { (((v - lo) / span * LEVELS as f32).floor() as i64).clamp(0, LEVELS as i64 - 1) as u32 };
    let q: Vec<u32> = data.iter().map(|v| if r.is_valid(*v) && *v != NODATA { level(*v) } else { u32::MAX }).collect();
    let half = ((window.clamp(3, 11) | 1) / 2) as i64;
    let mut out = Vec::with_capacity(measures.len());
    for m in measures {
        crate::control::check()?;
        let m = *m;
        out.push(per_index(n, w as usize, ((2 * half + 1) * (2 * half + 1) * 4) as usize, Vec::new, |pairs: &mut Vec<u32>, i| {
            let (c, rr) = (i as i64 % w.max(1), i as i64 / w.max(1));
            if q.get(i).copied().unwrap_or(u32::MAX) == u32::MAX {
                return NODATA;
            }
            pairs.clear();
            let g = |cc: i64, r2: i64| -> Option<u32> {
                if cc < 0 || r2 < 0 || cc >= w || r2 >= h {
                    return None;
                }
                q.get((r2 * w + cc) as usize).copied().filter(|v| *v != u32::MAX)
            };
            for dr in -half..=half {
                for dc in -half..=half {
                    let Some(a) = g(c + dc, rr + dr) else { continue };
                    for (oc, or) in [(1, 0), (0, 1)] {
                        if dc + oc > half || dr + or > half {
                            continue;
                        }
                        if let Some(bb) = g(c + dc + oc, rr + dr + or) {
                            pairs.push(a * LEVELS + bb);
                            pairs.push(bb * LEVELS + a);
                        }
                    }
                }
            }
            if pairs.is_empty() {
                return NODATA;
            }
            let total = pairs.len() as f64;
            let v = match m {
                Texture::Entropy | Texture::Asm => {
                    pairs.sort_unstable();
                    let mut acc = 0.0;
                    let mut k = 0;
                    while k < pairs.len() {
                        let mut e = k;
                        while e < pairs.len() && pairs[e] == pairs[k] {
                            e += 1;
                        }
                        let p = (e - k) as f64 / total;
                        acc += if m == Texture::Entropy { -p * p.ln() } else { p * p };
                        k = e;
                    }
                    acc
                }
                Texture::Mean | Texture::Variance => {
                    let mu = pairs.iter().map(|x| f64::from(x / LEVELS)).sum::<f64>() / total;
                    if m == Texture::Mean { mu } else { pairs.iter().map(|x| (f64::from(x / LEVELS) - mu).powi(2)).sum::<f64>() / total }
                }
                _ => {
                    pairs
                        .iter()
                        .map(|x| {
                            let d = (f64::from(x / LEVELS) - f64::from(x % LEVELS)).abs();
                            match m {
                                Texture::Contrast => d * d,
                                Texture::Dissimilarity => d,
                                _ => 1.0 / (1.0 + d * d),
                            }
                        })
                        .sum::<f64>()
                        / total
                }
            };
            v as f32
        })?);
    }
    Ok(out)
}

/// A pixel's area in hectares on `r`'s grid.
pub fn pixel_hectares(r: &Raster) -> f64 {
    let (x, y) = r.cell_size_m();
    (x * y).abs() / 10_000.0
}

/// How many pixels each value of band `b` has (no-data left out), values ascending.
pub fn counts(r: &Raster, b: usize) -> Result<Vec<(f32, usize)>, String> {
    let data = band(r, b)?;
    let mut out: Vec<(f32, usize)> = Vec::new();
    for v in data {
        if !r.is_valid(*v) || *v == NODATA {
            continue;
        }
        let len = out.len();
        match out.iter_mut().find(|c| c.0 == *v) {
            Some(c) => c.1 += 1,
            None if len < MAX_CLASSES => out.push((*v, 1)),
            // A continuous raster: stops at once rather than listing its values.
            None => return Err(format!("{} has more than {MAX_CLASSES} different values: it isn't a class map", r.name)),
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(out)
}

/// From → to: pixels of each pair of values, `from` and `to` on one grid, and the change map
/// (from × 1000 + to where they differ, 0 where they don't).
pub fn cross_tab(from: &[f32], to: &[f32]) -> Result<(Vec<((f32, f32), usize)>, Vec<f32>), String> {
    let mut m: Vec<((f32, f32), usize)> = Vec::new();
    let mut map = Vec::with_capacity(from.len());
    for (a, b) in from.iter().zip(to) {
        let ok = |v: &f32| *v != NODATA && v.is_finite();
        if !ok(a) || !ok(b) {
            map.push(NODATA);
            continue;
        }
        let len = m.len();
        match m.iter_mut().find(|x| x.0 == (*a, *b)) {
            Some(x) => x.1 += 1,
            None if len < MAX_CLASSES * 4 => m.push(((*a, *b), 1)),
            None => return Err(format!("more than {} from → to pairs: compare two class maps (land cover), not images", MAX_CLASSES * 4)),
        }
        map.push(if a == b { 0.0 } else { a * 1000.0 + b });
    }
    m.sort_by(|x, y| x.0.0.total_cmp(&y.0.0).then(x.0.1.total_cmp(&y.0.1)));
    Ok((m, map))
}

/// An accuracy assessment of a class map against reference samples.
#[derive(Clone, Debug, Default)]
pub struct Accuracy {
    pub classes: Vec<String>,
    /// Map class (row) × reference class (column): sample counts.
    pub matrix: Vec<Vec<usize>>,
    pub overall: f64,
    pub kappa: f64,
    pub users: Vec<f64>,
    pub producers: Vec<f64>,
    /// Area-weighted (Olofsson et al. 2014): overall accuracy, producer's accuracy, each class's
    /// estimated area and its 95 % confidence interval (ha), from the mapped areas.
    pub weighted_overall: f64,
    pub weighted_producers: Vec<f64>,
    pub estimated_ha: Vec<f64>,
    pub ci95_ha: Vec<f64>,
    pub mapped_ha: Vec<f64>,
}

/// The accuracy of `pairs` (map class, reference class) given the mapped area of each class (ha,
/// in `classes`' order).
pub fn accuracy(classes: &[String], pairs: &[(usize, usize)], mapped_ha: &[f64]) -> Accuracy {
    let k = classes.len();
    let mut matrix = vec![vec![0usize; k]; k];
    for (m, rf) in pairs {
        if let Some(c) = matrix.get_mut(*m).and_then(|row| row.get_mut(*rf)) {
            *c += 1;
        }
    }
    let n: usize = pairs.iter().filter(|(m, r)| *m < k && *r < k).count();
    let diag: usize = (0..k).map(|i| matrix[i][i]).sum();
    let row = |i: usize| matrix[i].iter().sum::<usize>();
    let col = |j: usize| (0..k).map(|i| matrix[i][j]).sum::<usize>();
    let nn = n.max(1) as f64;
    let overall = diag as f64 / nn;
    let pe: f64 = (0..k).map(|i| row(i) as f64 * col(i) as f64).sum::<f64>() / (nn * nn);
    let kappa = if pe < 1.0 { (overall - pe) / (1.0 - pe) } else { 1.0 };
    let users = (0..k).map(|i| if row(i) > 0 { matrix[i][i] as f64 / row(i) as f64 } else { 0.0 }).collect();
    let producers = (0..k).map(|j| if col(j) > 0 { matrix[j][j] as f64 / col(j) as f64 } else { 0.0 }).collect();
    let total_ha: f64 = mapped_ha.iter().sum();
    let wi: Vec<f64> = (0..k).map(|i| if total_ha > 0.0 { mapped_ha.get(i).copied().unwrap_or(0.0) / total_ha } else { 0.0 }).collect();
    // Estimated proportions p_ij = W_i n_ij / n_i.
    let p = |i: usize, j: usize| if row(i) > 0 { wi[i] * matrix[i][j] as f64 / row(i) as f64 } else { 0.0 };
    let pj: Vec<f64> = (0..k).map(|j| (0..k).map(|i| p(i, j)).sum()).collect();
    let weighted_overall = (0..k).map(|i| p(i, i)).sum();
    let weighted_producers = (0..k).map(|j| if pj[j] > 0.0 { p(j, j) / pj[j] } else { 0.0 }).collect();
    let estimated_ha = pj.iter().map(|v| v * total_ha).collect();
    let ci95_ha = (0..k)
        .map(|j| {
            let var: f64 = (0..k)
                .map(|i| {
                    let ni = row(i) as f64;
                    if ni < 2.0 {
                        return 0.0;
                    }
                    let f = matrix[i][j] as f64 / ni;
                    wi[i] * wi[i] * f * (1.0 - f) / (ni - 1.0)
                })
                .sum();
            1.96 * var.sqrt() * total_ha
        })
        .collect();
    Accuracy {
        classes: classes.to_vec(),
        matrix,
        overall,
        kappa,
        users,
        producers,
        weighted_overall,
        weighted_producers,
        estimated_ha,
        ci95_ha,
        mapped_ha: mapped_ha.to_vec(),
    }
}

/// Stratified random points: up to `per_class` pixel centres of each value of band `b`, as
/// (longitude, latitude, value).
pub fn sample_points(r: &Raster, b: usize, per_class: usize, seed: u64) -> Result<Vec<(Coord, f32)>, String> {
    let data = band(r, b)?;
    let w = r.width as usize;
    let mut by: Vec<(f32, Vec<usize>)> = Vec::new();
    for (i, v) in data.iter().enumerate() {
        if !r.is_valid(*v) || *v == NODATA {
            continue;
        }
        let len = by.len();
        match by.iter_mut().find(|c| c.0 == *v) {
            Some(c) => c.1.push(i),
            None if len < MAX_CLASSES => by.push((*v, vec![i])),
            None => return Err(format!("{} has more than {MAX_CLASSES} different values: it isn't a class map", r.name)),
        }
    }
    by.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut rng = Rng::new(seed);
    let tr = r.transformer();
    let mut out = Vec::new();
    for (v, mut idx) in by {
        for i in (1..idx.len()).rev() {
            idx.swap(i, rng.below(i + 1));
        }
        for i in idx.into_iter().take(per_class) {
            let world = r.transform.to_world((i % w.max(1)) as f64 + 0.5, (i / w.max(1)) as f64 + 0.5);
            if let Some(ll) = tr.to_lonlat(world) {
                out.push((ll, v));
            }
        }
    }
    Ok(out)
}

/// The magic wand: the pixels connected (4-way) to `seed` (column, row) whose values over `bands`
/// lie within `tolerance` of the seed's, each band measured in its own standard deviations (so
/// bands of different ranges count alike), up to `max_pixels`. Returns the region as a mask over
/// the raster's grid (row by row) and its pixel count.
pub fn region_grow(r: &Raster, bands: &[usize], seed: (usize, usize), tolerance: f64, max_pixels: usize) -> Result<(Vec<bool>, usize), String> {
    let data = bands_of(r, bands)?;
    let (w, h) = (r.width as usize, r.height as usize);
    if seed.0 >= w || seed.1 >= h {
        return Err("the point is outside the image".into());
    }
    let n = w * h;
    let mut buf = Vec::with_capacity(data.len());
    // Each band's spread, from a sample.
    let sample: Vec<Vec<f64>> =
        (0..n).step_by((n / 20_000).max(1)).filter_map(|i| if pixel_into(r, &data, i, &mut buf) { Some(buf.clone()) } else { None }).collect();
    let m = mean(&sample, data.len());
    let cov = covariance(&sample, &m);
    let sd: Vec<f64> = (0..data.len()).map(|i| cov[i][i].sqrt().max(1e-12)).collect();
    let start = seed.1 * w + seed.0;
    if !pixel_into(r, &data, start, &mut buf) {
        return Err("the image has no value at that point".into());
    }
    let centre = buf.clone();
    let mut mask = new_band(w, h, false)?;
    let mut stack = vec![start];
    mask[start] = true;
    let mut count = 0usize;
    while let Some(i) = stack.pop() {
        count += 1;
        if count >= max_pixels {
            break;
        }
        let (c, rr) = (i % w, i / w);
        let mut next = |j: usize, mask: &mut Vec<bool>, stack: &mut Vec<usize>| {
            if !mask[j] && pixel_into(r, &data, j, &mut buf) {
                let d: f64 = buf.iter().zip(&centre).zip(&sd).map(|((v, c0), s)| ((v - c0) / s).powi(2)).sum::<f64>().sqrt();
                if d <= tolerance {
                    mask[j] = true;
                    stack.push(j);
                }
            }
        };
        if c > 0 {
            next(i - 1, &mut mask, &mut stack);
        }
        if c + 1 < w {
            next(i + 1, &mut mask, &mut stack);
        }
        if rr > 0 {
            next(i - w, &mut mask, &mut stack);
        }
        if rr + 1 < h {
            next(i + w, &mut mask, &mut stack);
        }
    }
    let count = mask.iter().filter(|m| **m).count();
    Ok((mask, count))
}

/// The outlines of a mask's cells (row by row, `w` across): polygons of rings of pixel corners
/// (column, row), the first ring of each its outside and the others its holes. Straight runs
/// keep only their ends.
pub fn trace(mask: &[bool], w: usize, h: usize) -> Vec<Vec<Vec<(f64, f64)>>> {
    let at = |c: i64, r: i64| c >= 0 && r >= 0 && (c as usize) < w && (r as usize) < h && mask.get(r as usize * w + c as usize).copied().unwrap_or(false);
    // Edges with the inside on their right (rows grow downwards): each cell's sides on the
    // boundary, clockwise round the cell.
    let mut from: std::collections::HashMap<(i64, i64), Vec<(i64, i64)>> = std::collections::HashMap::new();
    for r in 0..h as i64 {
        for c in 0..w as i64 {
            if !at(c, r) {
                continue;
            }
            if !at(c, r - 1) {
                from.entry((c, r)).or_default().push((c + 1, r));
            }
            if !at(c + 1, r) {
                from.entry((c + 1, r)).or_default().push((c + 1, r + 1));
            }
            if !at(c, r + 1) {
                from.entry((c + 1, r + 1)).or_default().push((c, r + 1));
            }
            if !at(c - 1, r) {
                from.entry((c, r + 1)).or_default().push((c, r));
            }
        }
    }
    let mut rings: Vec<Vec<(i64, i64)>> = Vec::new();
    let mut starts: Vec<(i64, i64)> = from.keys().copied().collect();
    starts.sort_unstable();
    for s0 in starts {
        while let Some(first) = from.get_mut(&s0).and_then(Vec::pop) {
            let mut ring = vec![s0];
            let (mut prev, mut cur) = (s0, first);
            while cur != s0 {
                ring.push(cur);
                let dir = (cur.0 - prev.0, cur.1 - prev.1);
                let Some(nexts) = from.get_mut(&cur) else { break };
                if nexts.is_empty() {
                    break;
                }
                // At a corner two regions share, turn right: keeps them apart.
                let right = (-dir.1, dir.0);
                let k = nexts.iter().position(|n| (n.0 - cur.0, n.1 - cur.1) == right).unwrap_or(0);
                let n = nexts.swap_remove(k);
                prev = cur;
                cur = n;
            }
            if ring.len() >= 4 {
                rings.push(ring);
            }
        }
    }
    // Straight runs to their ends.
    let simplify = |ring: &[(i64, i64)]| -> Vec<(f64, f64)> {
        let n = ring.len();
        let mut out: Vec<(f64, f64)> = (0..n)
            .filter(|i| {
                let (a, b, c) = (ring[(i + n - 1) % n], ring[*i], ring[(i + 1) % n]);
                (b.0 - a.0) * (c.1 - b.1) != (b.1 - a.1) * (c.0 - b.0)
            })
            .map(|i| (ring[i].0 as f64, ring[i].1 as f64))
            .collect();
        if let Some(f) = out.first().copied() {
            out.push(f);
        }
        out
    };
    let area = |ring: &[(f64, f64)]| ring.windows(2).map(|p| p[0].0 * p[1].1 - p[1].0 * p[0].1).sum::<f64>() / 2.0;
    let simple: Vec<Vec<(f64, f64)>> = rings.iter().map(|r| simplify(r)).filter(|r| r.len() >= 4).collect();
    // Clockwise on screen (positive here) is an outside; the others are holes.
    let (outer, holes): (Vec<_>, Vec<_>) = simple.into_iter().partition(|r| area(r) > 0.0);
    let mut polys: Vec<Vec<Vec<(f64, f64)>>> = outer.into_iter().map(|o| vec![o]).collect();
    let inside = |pt: (f64, f64), ring: &[(f64, f64)]| {
        let mut hit = false;
        for k in 0..ring.len().saturating_sub(1) {
            let (a, b) = (ring[k], ring[k + 1]);
            if (a.1 > pt.1) != (b.1 > pt.1) && pt.0 < (b.0 - a.0) * (pt.1 - a.1) / (b.1 - a.1) + a.0 {
                hit = !hit;
            }
        }
        hit
    };
    for hole in holes {
        // A point just inside the hole's first edge: on no ring's line.
        let (a, b) = (hole[0], hole[1]);
        let probe = ((a.0 + b.0) / 2.0 + (b.1 - a.1) * 0.25, (a.1 + b.1) / 2.0 - (b.0 - a.0) * 0.25);
        let owner = polys
            .iter()
            .enumerate()
            .filter(|(_, p)| inside(probe, &p[0]))
            .min_by(|x, y| area(&x.1[0]).total_cmp(&area(&y.1[0])))
            .map(|(i, _)| i);
        if let Some(i) = owner {
            polys[i].push(hole);
        }
    }
    polys
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustgis_data::raster::{ColorHint, Crs, GeoTransform};

    fn img(w: u32, h: u32, bands: Vec<Vec<f32>>) -> Raster {
        let t = GeoTransform { origin_x: 104.0, origin_y: 12.0, pixel_width: 0.001, pixel_height: -0.001 };
        Raster::new("t", w, h, bands, t, Crs::LonLat, Some(NODATA as f64), ColorHint::Gray, "Float32").unwrap()
    }

    /// Two classes apart in two bands: every classifier separates them.
    #[test]
    fn every_classifier_separates_two_clear_classes() {
        let mut rng = Rng::new(7);
        let mut jitter = || (rng.below(100) as f64 - 50.0) / 5000.0;
        let water: Vec<Vec<f64>> = (0..60).map(|_| vec![0.05 + jitter(), 0.02 + jitter()]).collect();
        let field: Vec<Vec<f64>> = (0..60).map(|_| vec![0.30 + jitter(), 0.45 + jitter()]).collect();
        let s: Samples = vec![("water".into(), water), ("field".into(), field)];
        for m in [
            Method::MinDistance,
            Method::Mahalanobis,
            Method::MaxLikelihood,
            Method::Sam,
            Method::Parallelepiped(3.0),
            Method::Knn(3),
            Method::RandomForest(20),
        ] {
            let model = train(&s, m).unwrap();
            assert_eq!(predict(&model, &[0.06, 0.03]), Some(0), "{m:?}");
            assert_eq!(predict(&model, &[0.29, 0.44]), Some(1), "{m:?}");
            assert!(training_accuracy(&model, &s) > 0.95, "{m:?}");
        }
        assert!(predict(&train(&s, Method::Parallelepiped(1.0)).unwrap(), &[5.0, 5.0]).is_none());
        let rf = train(&s, Method::RandomForest(10)).unwrap();
        assert!((rf.importance.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn isodata_finds_the_groups() {
        let mut a = Vec::new();
        for i in 0..400 {
            a.push(
                if i % 3 == 0 {
                    0.1
                } else if i % 3 == 1 {
                    0.5
                } else {
                    0.9
                } + (i % 7) as f32 * 0.002,
            );
        }
        let r = img(20, 20, vec![a]);
        let c = isodata(&r, &[0], 2, 6, 10).unwrap();
        let mut seen: Vec<f32> = c.clone();
        seen.sort_by(f32::total_cmp);
        seen.dedup();
        assert!((2..=6).contains(&seen.len()), "{seen:?}");
        // Pixels of the lowest group share the lowest class.
        assert_eq!(c[0], 1.0);
        assert_eq!(c[0], c[3]);
    }

    #[test]
    fn filters_sieve_and_texture() {
        let mut v = vec![1.0f32; 25];
        v[12] = 9.0;
        let r = img(5, 5, vec![v]);
        assert_eq!(filter(&r, 0, Filter::Median, 3).unwrap()[12], 1.0);
        assert_eq!(filter(&r, 0, Filter::Majority, 3).unwrap()[12], 1.0);
        assert!((filter(&r, 0, Filter::Mean, 3).unwrap()[12] - 17.0 / 9.0).abs() < 1e-5);
        let (s, changed) = sieve(&r, 0, 2).unwrap();
        assert_eq!((s[12], changed), (1.0, 1));
        let t = texture(&r, 0, 3, &[Texture::Contrast, Texture::Homogeneity, Texture::Entropy]).unwrap();
        assert!(t[0][12] > t[0][0] && t[1][12] < t[1][0] && t[2][0] >= 0.0);
    }

    #[test]
    fn pca_puts_the_shared_variance_first() {
        let a: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..100).map(|i| i as f32 * 2.0 + (i % 3) as f32 * 0.1).collect();
        let p = pca(&img(10, 10, vec![a, b]), &[0, 1], 2).unwrap();
        assert!(p.explained[0] > 99.0, "{:?}", p.explained);
        assert_eq!(p.components.len(), 2);
    }

    /// The area-adjusted estimates (Olofsson et al. 2014, eq. 9–11), against a hand calculation.
    #[test]
    fn accuracy_follows_olofsson() {
        let classes: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let m = [[66, 0, 1, 2], [0, 55, 0, 1], [5, 8, 153, 9], [4, 12, 11, 313]];
        let mut pairs = Vec::new();
        for (i, row) in m.iter().enumerate() {
            for (j, n) in row.iter().enumerate() {
                pairs.extend(std::iter::repeat_n((i, j), *n));
            }
        }
        let mapped = [200_000.0, 150_000.0, 3_200_000.0, 6_450_000.0];
        let a = accuracy(&classes, &pairs, &mapped);
        assert!((a.weighted_overall - 0.907_413).abs() < 1e-5, "{}", a.weighted_overall);
        assert!((a.estimated_ha[0] - 358_615.27).abs() < 0.1, "{:?}", a.estimated_ha);
        assert!((a.ci95_ha[0] - 108_857.86).abs() < 0.1, "{:?}", a.ci95_ha);
        assert!((a.overall - 587.0 / 640.0).abs() < 1e-12);
    }

    #[test]
    fn magic_wand_grows_a_region_and_traces_it() {
        // A bright 3 × 3 square with a dark hole in a dark image.
        let mut v = vec![0.0f32; 49];
        for r in 2..5 {
            for c in 2..5 {
                v[r * 7 + c] = 10.0;
            }
        }
        v[3 * 7 + 3] = 0.0;
        let r = img(7, 7, vec![v]);
        let (mask, n) = region_grow(&r, &[0], (2, 2), 0.5, 1000).unwrap();
        assert_eq!(n, 8);
        assert!(!mask[3 * 7 + 3]);
        let polys = trace(&mask, 7, 7);
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].len(), 2, "an outside and a hole: {polys:?}");
        assert_eq!(polys[0][0].len(), 5);
        // Two cells touching at a corner stay two polygons.
        let diag = [true, false, false, true];
        assert_eq!(trace(&diag, 2, 2).len(), 2);
    }

    #[test]
    fn cross_tab_counts_and_maps_changes() {
        let (m, map) = cross_tab(&[1.0, 1.0, 2.0, NODATA], &[1.0, 2.0, 2.0, 1.0]).unwrap();
        assert_eq!(m, vec![((1.0, 1.0), 1), ((1.0, 2.0), 1), ((2.0, 2.0), 1)]);
        assert_eq!(map, vec![0.0, 1002.0, 0.0, NODATA]);
    }
}
