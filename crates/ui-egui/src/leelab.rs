//! LEE (Local Earth Engine) in KHGIS: Earth-Engine-style scripts, as LEE RS Lab runs them, here
//! with no Python. A small interpreter for the part of Python the scripts use (assignments, method
//! chains, arithmetic, lists and dicts, `for` loops, `print`) over LEE's objects:
//! `lee.Image("layer")` (a raster of the project), `lee.ImageCollection(…)` with `filterDate`, `filterBounds`, `filterMetadata`, `limit`,
//! `select`, `size`, `median`, `mean`, `mosaic`, `first`; `lee.Image.load(…)` (DEM, WorldCover,
//! Esri land cover, surface water); images with `normalizedDifference`, `expression`, `select`,
//! `rename`, `+ - * /`, `gt lt gte lte eq neq And Or Not`, `updateMask`, `clip`, `slope`, `aspect`,
//! `hillshade`, `cluster`, `otsu`, `reduceRegion`, `classAreas`; `Map.addLayer`,
//! `Map.centerObject`, `Chart.bars`, `lee.config(scale=…)`. `aoi` is the study area of tab 1.
//!
//! Images are recipes, made into layers only when a statement needs them (to draw, to measure):
//! the search, the downloads and each step run as the lab's other work does, and the script runs
//! again from the top when one lands. What it made is kept (by recipe), and what it printed or
//! drew is not done twice, so the second run picks up where the first waited.

use std::collections::HashMap;
use std::rc::Rc;

use serde_json::{Value as Json, json};

use crate::RustGisApp;
use crate::rslab::Dataset;

/// A script's value.
#[derive(Clone, Debug)]
enum V {
    None,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<V>),
    Dict(Vec<(String, V)>),
    Coll(Rc<Coll>),
    Img(Rc<Img>),
    /// A built-in object or function (`lee`, `Map`, `print`, `lee.ImageCollection`…).
    Builtin(String),
    /// A method of a value, to be called.
    Method(Box<V>, String),
    Aoi,
    /// An image's grid (`img.grid.scale`).
    Grid(f64),
}

#[derive(Clone, Debug)]
struct Coll {
    dataset: Dataset,
    start: String,
    end: String,
    cloud: f64,
    limit: usize,
    select: Vec<String>,
}

/// An image, as a recipe.
#[derive(Clone, Debug)]
enum Img {
    Composite(Coll, &'static str),
    Load(Dataset, Option<String>),
    /// A raster layer of the project.
    Layer(String),
    /// A band arithmetic expression over one or two images' bands (`a…` the first's, `b…` the
    /// second's).
    Math(Rc<Img>, Option<Rc<Img>>, String),
    NormDiff(Rc<Img>, String, String),
    Expression(Rc<Img>, String),
    Select(Rc<Img>, Vec<String>),
    Rename(Rc<Img>, Vec<String>),
    Terrain(Rc<Img>, &'static str, f64, f64),
    Cluster(Rc<Img>, usize, Vec<String>),
}

impl Img {
    fn key(&self) -> String {
        format!("{self:?}")
    }
    /// The names its bands go by, when a rename gave them.
    fn renamed(&self) -> Option<Vec<String>> {
        match self {
            Img::Rename(_, n) => Some(n.clone()),
            _ => None,
        }
    }
}

/// What stopped a run: waiting for work, or an error.
enum Stop {
    Wait,
    Err(String),
}

type R<T> = Result<T, Stop>;

fn err<T>(s: impl Into<String>) -> R<T> {
    Err(Stop::Err(s.into()))
}

// ------------------------------------------------------------------------------------ parsing

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Name(String),
    Num(f64),
    Str(String),
    Op(String),
}

fn lex(line: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '#' {
            break;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let s = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == 'e' || ((c[i] == '-' || c[i] == '+') && c[i - 1] == 'e'))
            {
                i += 1;
            }
            let t: String = c[s..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| format!("'{t}' isn't a number"))?));
        } else if ch.is_alphabetic() || ch == '_' {
            let s = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Name(c[s..i].iter().collect()));
        } else if ch == '"' || ch == '\'' {
            let q = ch;
            i += 1;
            let mut t = String::new();
            while i < c.len() && c[i] != q {
                if c[i] == '\\' && i + 1 < c.len() {
                    i += 1;
                    t.push(match c[i] {
                        'n' => '\n',
                        x => x,
                    });
                } else {
                    t.push(c[i]);
                }
                i += 1;
            }
            if i >= c.len() {
                return Err("a string isn't closed".into());
            }
            i += 1;
            out.push(Tok::Str(t));
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            if ["==", "!=", "<=", ">=", "**"].contains(&two.as_str()) {
                out.push(Tok::Op(two));
                i += 2;
            } else if "+-*/%()[]{},:.=<>".contains(ch) {
                out.push(Tok::Op(ch.to_string()));
                i += 1;
            } else {
                return Err(format!("unexpected '{ch}'"));
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
enum E {
    Lit(V),
    Name(String),
    List(Vec<E>),
    Dict(Vec<(E, E)>),
    Attr(Box<E>, String),
    Call(Box<E>, Vec<E>, Vec<(String, E)>),
    Index(Box<E>, Box<E>),
    Bin(String, Box<E>, Box<E>),
    Neg(Box<E>),
    Not(Box<E>),
}

#[derive(Clone, Debug)]
enum S {
    Assign(Vec<String>, E),
    Expr(E),
    /// `d[k] = v` (a dict's entry, or a list's item).
    SetItem(String, E, E),
    For(Vec<String>, E, Vec<S>),
}

struct P<'a> {
    t: &'a [Tok],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is_op(&self, o: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(x)) if x == o)
    }
    fn is_name(&self, n: &str) -> bool {
        matches!(self.peek(), Some(Tok::Name(x)) if x == n)
    }
    fn eat(&mut self, o: &str) -> Result<(), String> {
        if self.is_op(o) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{o}'"))
        }
    }
    fn expr(&mut self) -> Result<E, String> {
        self.or()
    }
    fn or(&mut self) -> Result<E, String> {
        let mut e = self.and()?;
        while self.is_name("or") {
            self.i += 1;
            e = E::Bin("or".into(), Box::new(e), Box::new(self.and()?));
        }
        Ok(e)
    }
    fn and(&mut self) -> Result<E, String> {
        let mut e = self.not()?;
        while self.is_name("and") {
            self.i += 1;
            e = E::Bin("and".into(), Box::new(e), Box::new(self.not()?));
        }
        Ok(e)
    }
    fn not(&mut self) -> Result<E, String> {
        if self.is_name("not") {
            self.i += 1;
            return Ok(E::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }
    fn cmp(&mut self) -> Result<E, String> {
        let e = self.sum()?;
        for o in ["==", "!=", "<=", ">=", "<", ">"] {
            if self.is_op(o) {
                self.i += 1;
                return Ok(E::Bin(o.into(), Box::new(e), Box::new(self.sum()?)));
            }
        }
        Ok(e)
    }
    fn sum(&mut self) -> Result<E, String> {
        let mut e = self.term()?;
        while self.is_op("+") || self.is_op("-") {
            let o = if self.is_op("+") { "+" } else { "-" };
            self.i += 1;
            e = E::Bin(o.into(), Box::new(e), Box::new(self.term()?));
        }
        Ok(e)
    }
    fn term(&mut self) -> Result<E, String> {
        let mut e = self.unary()?;
        while self.is_op("*") || self.is_op("/") || self.is_op("%") {
            let o = match self.peek() {
                Some(Tok::Op(x)) => x.clone(),
                _ => break,
            };
            self.i += 1;
            e = E::Bin(o, Box::new(e), Box::new(self.unary()?));
        }
        Ok(e)
    }
    fn unary(&mut self) -> Result<E, String> {
        if self.is_op("-") {
            self.i += 1;
            return Ok(E::Neg(Box::new(self.unary()?)));
        }
        let e = self.postfix()?;
        if self.is_op("**") {
            self.i += 1;
            return Ok(E::Bin("**".into(), Box::new(e), Box::new(self.unary()?)));
        }
        Ok(e)
    }
    fn postfix(&mut self) -> Result<E, String> {
        let mut e = self.atom()?;
        loop {
            if self.is_op(".") {
                self.i += 1;
                match self.peek().cloned() {
                    Some(Tok::Name(n)) => {
                        self.i += 1;
                        e = E::Attr(Box::new(e), n);
                    }
                    _ => return Err("a name after '.'".into()),
                }
            } else if self.is_op("(") {
                self.i += 1;
                let (mut args, mut kw) = (Vec::new(), Vec::new());
                while !self.is_op(")") {
                    if let (Some(Tok::Name(n)), Some(Tok::Op(o))) = (self.t.get(self.i), self.t.get(self.i + 1))
                        && o == "="
                    {
                        let n = n.clone();
                        self.i += 2;
                        kw.push((n, self.expr()?));
                    } else {
                        args.push(self.expr()?);
                    }
                    if !self.is_op(")") {
                        self.eat(",")?;
                    }
                }
                self.eat(")")?;
                e = E::Call(Box::new(e), args, kw);
            } else if self.is_op("[") {
                self.i += 1;
                let k = self.expr()?;
                self.eat("]")?;
                e = E::Index(Box::new(e), Box::new(k));
            } else {
                return Ok(e);
            }
        }
    }
    fn atom(&mut self) -> Result<E, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(E::Lit(V::Num(n)))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(E::Lit(V::Str(s)))
            }
            Some(Tok::Name(n)) => {
                self.i += 1;
                Ok(match n.as_str() {
                    "True" => E::Lit(V::Bool(true)),
                    "False" => E::Lit(V::Bool(false)),
                    "None" => E::Lit(V::None),
                    "lambda" => return Err("lambda isn't supported in KHGIS's LEE yet: write the steps out".into()),
                    _ => E::Name(n),
                })
            }
            Some(Tok::Op(o)) if o == "(" => {
                self.i += 1;
                let e = self.expr()?;
                if self.is_op(",") {
                    // A tuple: read as a list.
                    let mut v = vec![e];
                    while self.is_op(",") {
                        self.i += 1;
                        if self.is_op(")") {
                            break;
                        }
                        v.push(self.expr()?);
                    }
                    self.eat(")")?;
                    return Ok(E::List(v));
                }
                self.eat(")")?;
                Ok(e)
            }
            Some(Tok::Op(o)) if o == "[" => {
                self.i += 1;
                let mut v = Vec::new();
                while !self.is_op("]") {
                    v.push(self.expr()?);
                    if !self.is_op("]") {
                        self.eat(",")?;
                    }
                }
                self.eat("]")?;
                Ok(E::List(v))
            }
            Some(Tok::Op(o)) if o == "{" => {
                self.i += 1;
                let mut v = Vec::new();
                while !self.is_op("}") {
                    let k = self.expr()?;
                    self.eat(":")?;
                    v.push((k, self.expr()?));
                    if !self.is_op("}") {
                        self.eat(",")?;
                    }
                }
                self.eat("}")?;
                Ok(E::Dict(v))
            }
            other => Err(format!("unexpected {}", other.map_or("end of line".to_string(), |t| format!("{t:?}")))),
        }
    }
}

/// The script's logical lines (brackets joined) with their indentation, and the line each starts on.
fn logical_lines(src: &str) -> Result<Vec<(usize, usize, Vec<Tok>)>, String> {
    let mut out = Vec::new();
    let mut pending: Option<(usize, usize, Vec<Tok>, i32)> = None;
    for (n, raw) in src.lines().enumerate() {
        let toks = lex(raw).map_err(|e| format!("line {}: {e}", n + 1))?;
        let depth_change: i32 = toks
            .iter()
            .map(|t| match t {
                Tok::Op(o) if o == "(" || o == "[" || o == "{" => 1,
                Tok::Op(o) if o == ")" || o == "]" || o == "}" => -1,
                _ => 0,
            })
            .sum();
        match pending.take() {
            Some((indent, line, mut acc, depth)) => {
                acc.extend(toks);
                let d = depth + depth_change;
                if d > 0 { pending = Some((indent, line, acc, d)) } else { out.push((indent, line, acc)) }
            }
            None => {
                if toks.is_empty() {
                    continue;
                }
                let indent = raw.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum();
                if depth_change > 0 { pending = Some((indent, n + 1, toks, depth_change)) } else { out.push((indent, n + 1, toks)) }
            }
        }
    }
    if let Some((_, line, ..)) = pending {
        return Err(format!("line {line}: a bracket isn't closed"));
    }
    Ok(out)
}

fn parse_block(lines: &[(usize, usize, Vec<Tok>)], i: &mut usize, indent: usize) -> Result<Vec<S>, String> {
    let mut out = Vec::new();
    while let Some((ind, line, toks)) = lines.get(*i) {
        if *ind < indent {
            break;
        }
        if *ind > indent {
            return Err(format!("line {line}: unexpected indentation"));
        }
        *i += 1;
        let at = |e: String| format!("line {line}: {e}");
        let mut p = P { t: toks, i: 0 };
        if p.is_name("for") {
            p.i += 1;
            let mut vars = Vec::new();
            loop {
                match p.peek().cloned() {
                    Some(Tok::Name(n)) if n != "in" => {
                        vars.push(n);
                        p.i += 1;
                    }
                    _ => return Err(at("for takes names, then 'in'".into())),
                }
                if p.is_op(",") {
                    p.i += 1;
                } else {
                    break;
                }
            }
            if !p.is_name("in") {
                return Err(at("expected 'in'".into()));
            }
            p.i += 1;
            let it = p.expr().map_err(at)?;
            p.eat(":").map_err(at)?;
            let inner = lines.get(*i).map_or(indent + 1, |l| l.0);
            if inner <= indent {
                return Err(at("a for loop needs an indented block".into()));
            }
            let body = parse_block(lines, i, inner)?;
            out.push(S::For(vars, it, body));
            continue;
        }
        if p.is_name("if") || p.is_name("while") || p.is_name("def") || p.is_name("import") {
            return Err(at("if, while, def and import aren't supported in KHGIS's LEE yet".into()));
        }
        // name[key] = expr
        if let (Some(Tok::Name(n)), Some(Tok::Op(o))) = (toks.first(), toks.get(1))
            && o == "["
        {
            let mut q = P { t: toks, i: 0 };
            if let Ok(E::Index(target, key)) = q.postfix()
                && matches!(*target, E::Name(_))
                && q.is_op("=")
            {
                q.i += 1;
                let e = q.expr().map_err(at)?;
                if q.i != toks.len() {
                    return Err(at("unexpected text at the end".into()));
                }
                out.push(S::SetItem(n.clone(), *key, e));
                continue;
            }
        }
        // name (, name)* = expr
        let mut names = Vec::new();
        let save = p.i;
        while let Some(Tok::Name(n)) = p.peek().cloned() {
            p.i += 1;
            names.push(n);
            if p.is_op(",") {
                p.i += 1;
            } else {
                break;
            }
        }
        if !names.is_empty() && p.is_op("=") {
            p.i += 1;
            let e = p.expr().map_err(at)?;
            if p.i != toks.len() {
                return Err(at("unexpected text at the end".into()));
            }
            out.push(S::Assign(names, e));
            continue;
        }
        p.i = save;
        let e = p.expr().map_err(at)?;
        if p.i != toks.len() {
            return Err(at("unexpected text at the end".into()));
        }
        out.push(S::Expr(e));
    }
    Ok(out)
}

fn parse(src: &str) -> Result<Vec<S>, String> {
    let lines = logical_lines(src)?;
    let mut i = 0;
    let indent = lines.first().map_or(0, |l| l.0);
    let block = parse_block(&lines, &mut i, indent)?;
    if let Some((_, line, _)) = lines.get(i) {
        return Err(format!("line {line}: unexpected indentation"));
    }
    Ok(block)
}

// ------------------------------------------------------------------------------- running

/// A line the script printed, or a bar chart.
#[derive(Clone, Debug)]
pub enum Out {
    Text(String),
    Error(String),
    Bars(String, Vec<(String, f64)>),
}

/// What a run waits for: the search of a collection, or a run of steps making a layer.
enum Waiting {
    Search(String, crate::rslab::Answer, Dataset),
}

/// A script and its state between runs.
#[derive(Default)]
pub struct Lee {
    pub code: String,
    pub example: usize,
    pub out: Vec<Out>,
    running: bool,
    parsed: Vec<S>,
    /// Searches answered: their address → dates found.
    searches: HashMap<String, Vec<crate::rslab::Scene>>,
    /// Images made (or being made): recipe → layer.
    made: HashMap<String, String>,
    waiting: Option<Waiting>,
    /// Layers a script drew (a second draw of one draws a copy).
    drawn: std::collections::HashSet<String>,
    /// Effects (prints, layers drawn) done by earlier runs, not done again.
    effects_done: usize,
    effects_seen: usize,
    scale: f64,
    passes: usize,
}

impl Lee {
    pub fn running(&self) -> bool {
        self.running
    }
}

/// Starts the script in the editor.
pub fn start(app: &mut RustGisApp) {
    let code = app.ui.rslab.lee.code.clone();
    let lee = &mut app.ui.rslab.lee;
    lee.out.clear();
    match parse(&code) {
        Ok(p) => {
            lee.parsed = p;
            lee.running = true;
            lee.effects_done = 0;
            lee.scale = 0.0;
            lee.passes = 0;
            lee.waiting = None;
            step(app);
        }
        Err(e) => lee.out.push(Out::Error(e)),
    }
}

pub fn stop(app: &mut RustGisApp) {
    let lee = &mut app.ui.rslab.lee;
    lee.running = false;
    lee.waiting = None;
    lee.out.push(Out::Error(tl!("cancelled").to_string()));
}

/// Moves a running script on: lands its search, or runs it again once its work is done.
pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    if !app.ui.rslab.lee.running {
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(300));
    if let Some(Waiting::Search(url, slot, ds)) = &app.ui.rslab.lee.waiting {
        let landed = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        let (url, ds) = (url.clone(), *ds);
        match landed {
            None => return,
            Some(r) => {
                app.ui.rslab.lee.waiting = None;
                match r.and_then(|b| crate::rslab::scenes(ds, &b)) {
                    Ok(s) => {
                        app.ui.rslab.lee.searches.insert(url, s);
                    }
                    Err(e) => {
                        app.ui.rslab.lee.out.push(Out::Error(e));
                        app.ui.rslab.lee.running = false;
                        return;
                    }
                }
            }
        }
    }
    if crate::rslab::busy(app) {
        return;
    }
    step(app);
}

/// Runs the script from the top, as far as it goes.
fn step(app: &mut RustGisApp) {
    let stmts = app.ui.rslab.lee.parsed.clone();
    app.ui.rslab.lee.effects_seen = 0;
    app.ui.rslab.lee.passes += 1;
    if app.ui.rslab.lee.passes > 200 {
        app.ui.rslab.lee.out.push(Out::Error("the script waited too often: stopped".into()));
        app.ui.rslab.lee.running = false;
        return;
    }
    let mut env: HashMap<String, V> = HashMap::new();
    env.insert("aoi".into(), V::Aoi);
    for b in ["lee", "Map", "Chart", "print", "round", "len", "str", "abs", "min", "max", "range", "sorted", "sum", "int", "float"] {
        env.insert(b.into(), V::Builtin(b.into()));
    }
    let r = run_block(app, &stmts, &mut env);
    let lee = &mut app.ui.rslab.lee;
    match r {
        Ok(()) => {
            lee.running = false;
            lee.out.push(Out::Text(tl!("Done.").to_string()));
        }
        Err(Stop::Wait) => {}
        Err(Stop::Err(e)) => {
            lee.running = false;
            lee.out.push(Out::Error(e));
        }
    }
}

fn run_block(app: &mut RustGisApp, stmts: &[S], env: &mut HashMap<String, V>) -> R<()> {
    for s in stmts {
        match s {
            S::Assign(names, e) => {
                let v = eval(app, e, env)?;
                if names.len() == 1 {
                    env.insert(names[0].clone(), v);
                } else {
                    let items = as_list(&v)?;
                    if items.len() != names.len() {
                        return err(format!("{} values for {} names", items.len(), names.len()));
                    }
                    for (n, x) in names.iter().zip(items) {
                        env.insert(n.clone(), x);
                    }
                }
            }
            S::Expr(e) => {
                eval(app, e, env)?;
            }
            S::SetItem(name, k, e) => {
                let (k, v) = (eval(app, k, env)?, eval(app, e, env)?);
                match (env.get_mut(name), k) {
                    (Some(V::Dict(d)), k) => {
                        let k = match k {
                            V::Str(s) => s,
                            other => show(&other),
                        };
                        match d.iter_mut().find(|(x, _)| *x == k) {
                            Some(slot) => slot.1 = v,
                            None => d.push((k, v)),
                        }
                    }
                    (Some(V::List(l)), V::Num(i)) => {
                        let i = if i < 0.0 { l.len() as f64 + i } else { i } as usize;
                        match l.get_mut(i) {
                            Some(slot) => *slot = v,
                            None => return err("index out of range"),
                        }
                    }
                    (None, _) => return err(format!("'{name}' isn't defined")),
                    _ => return err(format!("can't set an item of '{name}'")),
                }
            }
            S::For(vars, it, body) => {
                let items = match eval(app, it, env)? {
                    V::Dict(d) => d.into_iter().map(|(k, _)| V::Str(k)).collect(),
                    other => as_list(&other)?,
                };
                for item in items.into_iter().take(1000) {
                    if vars.len() == 1 {
                        env.insert(vars[0].clone(), item);
                    } else {
                        let parts = as_list(&item)?;
                        for (n, x) in vars.iter().zip(parts) {
                            env.insert(n.clone(), x);
                        }
                    }
                    run_block(app, body, env)?;
                }
            }
        }
    }
    Ok(())
}

fn as_list(v: &V) -> R<Vec<V>> {
    match v {
        V::List(l) => Ok(l.clone()),
        V::Dict(d) => Ok(d.iter().map(|(k, x)| V::List(vec![V::Str(k.clone()), x.clone()])).collect()),
        V::Str(s) => Ok(s.chars().map(|c| V::Str(c.to_string())).collect()),
        _ => err("this isn't a list"),
    }
}

fn num(v: &V) -> R<f64> {
    match v {
        V::Num(n) => Ok(*n),
        V::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        _ => err(format!("expected a number, not {}", show(v))),
    }
}

fn text(v: &V) -> R<String> {
    match v {
        V::Str(s) => Ok(s.clone()),
        _ => err(format!("expected text, not {}", show(v))),
    }
}

fn texts(v: &V) -> R<Vec<String>> {
    match v {
        V::Str(s) => Ok(vec![s.clone()]),
        V::List(l) => l.iter().map(text).collect(),
        _ => err("expected a band name or a list of them"),
    }
}

/// A value as `print` shows it.
fn show(v: &V) -> String {
    match v {
        V::None => "None".into(),
        V::Bool(b) => if *b { "True" } else { "False" }.into(),
        V::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                let s = format!("{n:.6}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        V::Str(s) => s.clone(),
        V::List(l) => format!("[{}]", l.iter().map(show_inner).collect::<Vec<_>>().join(", ")),
        V::Dict(d) => format!("{{{}}}", d.iter().map(|(k, x)| format!("'{k}': {}", show_inner(x))).collect::<Vec<_>>().join(", ")),
        V::Coll(c) => format!("ImageCollection({:?})", c.dataset),
        V::Img(_) => "Image".into(),
        V::Builtin(b) => b.clone(),
        V::Method(_, m) => format!("<{m}>"),
        V::Aoi => "aoi".into(),
        V::Grid(s) => format!("grid ({s} m)"),
    }
}

fn show_inner(v: &V) -> String {
    match v {
        V::Str(s) => format!("'{s}'"),
        other => show(other),
    }
}

/// An effect (print, layer drawn) that an earlier run did already: skipped.
fn effect_done_before(app: &mut RustGisApp) -> bool {
    let lee = &mut app.ui.rslab.lee;
    lee.effects_seen += 1;
    if lee.effects_seen <= lee.effects_done {
        return true;
    }
    lee.effects_done = lee.effects_seen;
    false
}

fn eval(app: &mut RustGisApp, e: &E, env: &mut HashMap<String, V>) -> R<V> {
    Ok(match e {
        E::Lit(v) => v.clone(),
        E::Name(n) => match env.get(n) {
            Some(v) => v.clone(),
            None => return err(format!("'{n}' isn't defined")),
        },
        E::List(l) => V::List(l.iter().map(|x| eval(app, x, env)).collect::<R<_>>()?),
        E::Dict(d) => {
            let mut out = Vec::new();
            for (k, v) in d {
                let k = match eval(app, k, env)? {
                    V::Str(s) => s,
                    other => show(&other),
                };
                out.push((k, eval(app, v, env)?));
            }
            V::Dict(out)
        }
        E::Attr(o, name) => {
            let o = eval(app, o, env)?;
            attr(o, name)?
        }
        E::Index(o, k) => {
            let (o, k) = (eval(app, o, env)?, eval(app, k, env)?);
            match (&o, &k) {
                (V::Dict(d), V::Str(s)) => match d.iter().find(|(x, _)| x == s) {
                    Some((_, v)) => v.clone(),
                    // A one-entry result (`reduceRegion`) read by another band name.
                    None if d.len() == 1 => d[0].1.clone(),
                    None => return err(format!("no '{s}' in {}", show(&o))),
                },
                (V::List(l), V::Num(n)) => {
                    let i = if *n < 0.0 { l.len() as f64 + n } else { *n } as usize;
                    l.get(i).cloned().ok_or(Stop::Err("index out of range".into()))?
                }
                _ => return err("can't index that"),
            }
        }
        E::Neg(x) => {
            let v = eval(app, x, env)?;
            match v {
                V::Img(i) => V::Img(Rc::new(Img::Math(i, None, "-a1".into()))),
                other => V::Num(-num(&other)?),
            }
        }
        E::Not(x) => V::Bool(!truthy(&eval(app, x, env)?)),
        E::Bin(op, a, b) => {
            let (a, b) = (eval(app, a, env)?, eval(app, b, env)?);
            binop(op, a, b)?
        }
        E::Call(f, args, kw) => {
            let f = eval(app, f, env)?;
            let args: Vec<V> = args.iter().map(|x| eval(app, x, env)).collect::<R<_>>()?;
            let kw: Vec<(String, V)> = kw.iter().map(|(k, x)| Ok((k.clone(), eval(app, x, env)?))).collect::<R<_>>()?;
            call(app, f, args, kw)?
        }
    })
}

fn truthy(v: &V) -> bool {
    match v {
        V::None => false,
        V::Bool(b) => *b,
        V::Num(n) => *n != 0.0,
        V::Str(s) => !s.is_empty(),
        V::List(l) => !l.is_empty(),
        V::Dict(d) => !d.is_empty(),
        _ => true,
    }
}

fn binop(op: &str, a: V, b: V) -> R<V> {
    let math = |sym: &str| -> R<V> {
        let (x, y) = (&a, &b);
        Ok(match (x, y) {
            (V::Img(i), V::Img(j)) => V::Img(Rc::new(Img::Math(i.clone(), Some(j.clone()), format!("a1 {sym} b1")))),
            (V::Img(i), n) => V::Img(Rc::new(Img::Math(i.clone(), None, format!("a1 {sym} ({})", num(n)?)))),
            (n, V::Img(j)) => V::Img(Rc::new(Img::Math(j.clone(), None, format!("({}) {sym} a1", num(n)?)))),
            _ => unreachable_free(),
        })
    };
    let either_img = matches!(a, V::Img(_)) || matches!(b, V::Img(_));
    match op {
        "+" | "-" | "*" | "/" | "**" if either_img => math(if op == "**" { "^" } else { op }),
        "+" => match (&a, &b) {
            (V::Str(x), _) => Ok(V::Str(format!("{x}{}", show(&b)))),
            (_, V::Str(y)) => Ok(V::Str(format!("{}{y}", show(&a)))),
            (V::List(x), V::List(y)) => Ok(V::List(x.iter().chain(y).cloned().collect())),
            _ => Ok(V::Num(num(&a)? + num(&b)?)),
        },
        "-" => Ok(V::Num(num(&a)? - num(&b)?)),
        "*" => Ok(V::Num(num(&a)? * num(&b)?)),
        "/" => {
            let d = num(&b)?;
            if d == 0.0 { err("division by zero") } else { Ok(V::Num(num(&a)? / d)) }
        }
        "%" => match &a {
            V::Str(fmt) => Ok(V::Str(percent_format(fmt, &b))),
            _ => Ok(V::Num(num(&a)?.rem_euclid(num(&b)?))),
        },
        "**" => Ok(V::Num(num(&a)?.powf(num(&b)?))),
        "and" => Ok(if truthy(&a) { b } else { a }),
        "or" => Ok(if truthy(&a) { a } else { b }),
        "==" => Ok(V::Bool(show(&a) == show(&b))),
        "!=" => Ok(V::Bool(show(&a) != show(&b))),
        "<" => Ok(V::Bool(num(&a)? < num(&b)?)),
        ">" => Ok(V::Bool(num(&a)? > num(&b)?)),
        "<=" => Ok(V::Bool(num(&a)? <= num(&b)?)),
        ">=" => Ok(V::Bool(num(&a)? >= num(&b)?)),
        o => err(format!("'{o}' isn't supported")),
    }
}

/// `None`, standing in where both sides are already known to be images or numbers.
fn unreachable_free() -> V {
    V::None
}

/// Python's `"%s %.1f" % (a, b)`, for the common conversions.
fn percent_format(fmt: &str, args: &V) -> String {
    let args: Vec<V> = match args {
        V::List(l) => l.clone(),
        other => vec![other.clone()],
    };
    let mut out = String::new();
    let mut it = args.iter();
    let c: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '%' && i + 1 < c.len() {
            let s = i + 1;
            let mut j = s;
            while j < c.len() && !c[j].is_ascii_alphabetic() && c[j] != '%' {
                j += 1;
            }
            let spec: String = c[s..j.min(c.len())].iter().collect();
            let conv = c.get(j).copied().unwrap_or('s');
            if conv == '%' {
                out.push('%');
            } else {
                let v = it.next().cloned().unwrap_or(V::None);
                let width: usize = spec.trim_start_matches('-').split('.').next().and_then(|w| w.parse().ok()).unwrap_or(0);
                let prec: Option<usize> = spec.split('.').nth(1).and_then(|p| p.parse().ok());
                let body = match (conv, &v) {
                    ('f', V::Num(n)) => format!("{n:.*}", prec.unwrap_or(6)),
                    ('d', V::Num(n)) => format!("{}", *n as i64),
                    _ => show(&v),
                };
                if spec.starts_with('-') { out.push_str(&format!("{body:<width$}")) } else { out.push_str(&format!("{body:>width$}")) }
            }
            i = j + 1;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

fn attr(o: V, name: &str) -> R<V> {
    Ok(match (&o, name) {
        (V::Builtin(b), _) if b == "lee" && name == "Image" => V::Builtin("lee.Image".into()),
        (V::Img(_), "grid") => V::Grid(0.0),
        (V::Grid(_), "scale") => V::Builtin("grid.scale".into()),
        _ => V::Method(Box::new(o), name.to_string()),
    })
}

fn kwarg<'a>(kw: &'a [(String, V)], name: &str) -> Option<&'a V> {
    kw.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn dataset_named(name: &str) -> R<Dataset> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "sentinel-2" | "sentinel2" | "s2" | "sentinel-2@aws" => Dataset::Sentinel2,
        "landsat-8" | "landsat-9" | "landsat-8-9" | "landsat" => Dataset::Landsat89,
        "landsat-5" | "landsat-7" | "landsat-4" | "landsat-4-7" => Dataset::Landsat47,
        "sentinel-1" | "sentinel1" | "s1" => Dataset::Sentinel1,
        "dem" | "copernicus-dem" => Dataset::Dem,
        "worldcover" | "esa-worldcover" => Dataset::WorldCover,
        "lulc-annual" | "esri-lulc" | "lulc" => Dataset::EsriLandCover,
        "surface-water" | "jrc-water" | "water" => Dataset::Water,
        o => {
            return err(format!(
                "no dataset '{o}': sentinel-2, landsat-9, landsat-8, landsat-7, landsat-5, sentinel-1, dem, worldcover, lulc-annual, surface-water"
            ));
        }
    })
}

fn call(app: &mut RustGisApp, f: V, args: Vec<V>, kw: Vec<(String, V)>) -> R<V> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(V::None);
    match f {
        V::Builtin(b) => builtin(app, &b, &args, &kw),
        V::Grid(_) => err("grid isn't a function"),
        V::Method(obj, m) => {
            let obj = *obj;
            match (obj, m.as_str()) {
                (V::Builtin(b), "ImageCollection") if b == "lee" => Ok(V::Coll(Rc::new(Coll {
                    dataset: dataset_named(&text(&arg(0))?)?,
                    start: crate::stacui::days_ago(365),
                    end: crate::stacui::days_ago(0),
                    cloud: 100.0,
                    limit: 8,
                    select: Vec::new(),
                }))),
                (V::Builtin(b), "config") if b == "lee" => {
                    if let Some(s) = kwarg(&kw, "scale").or(args.first()) {
                        app.ui.rslab.lee.scale = num(s)?.clamp(10.0, 1000.0);
                    }
                    Ok(V::None)
                }
                (V::Builtin(b), "load") if b == "lee.Image" => {
                    let ds = dataset_named(&text(&arg(0))?)?;
                    let year = match args.get(1).or(kwarg(&kw, "year")) {
                        Some(V::Num(n)) => Some(format!("{}", *n as i64)),
                        Some(V::Str(s)) => Some(s.clone()),
                        _ => None,
                    };
                    Ok(V::Img(Rc::new(Img::Load(ds, year))))
                }
                (V::Builtin(b), "addLayer") if b == "Map" => {
                    let V::Img(img) = arg(0) else { return err("Map.addLayer takes an image") };
                    let layer = materialize(app, &img)?;
                    if effect_done_before(app) {
                        return Ok(V::None);
                    }
                    let name = match args.get(2).or(kwarg(&kw, "name")) {
                        Some(V::Str(s)) => s.clone(),
                        _ => img.renamed().and_then(|n| n.first().cloned()).unwrap_or_else(|| layer.clone()),
                    };
                    draw(app, &layer, &name, args.get(1).or(kwarg(&kw, "vis")));
                    Ok(V::None)
                }
                (V::Builtin(b), "centerObject") if b == "Map" => {
                    if !effect_done_before(app)
                        && let Ok(a) = crate::rslab::area_bbox(app)
                    {
                        let _ = app.run("map.zoomToExtent", json!({"extent": a}));
                    }
                    Ok(V::None)
                }
                (V::Builtin(b), "bars") if b == "Chart" => {
                    if effect_done_before(app) {
                        return Ok(V::None);
                    }
                    let V::Dict(d) = arg(0) else { return err("Chart.bars takes a dict of name: value") };
                    let title = match arg(1) {
                        V::Str(s) => s,
                        _ => String::new(),
                    };
                    let bars = d.iter().filter_map(|(k, v)| num(v).ok().map(|n| (k.clone(), n))).collect();
                    app.ui.rslab.lee.out.push(Out::Bars(title, bars));
                    Ok(V::None)
                }
                (V::Dict(d), "items") => Ok(V::List(d.into_iter().map(|(k, v)| V::List(vec![V::Str(k), v])).collect())),
                (V::Dict(d), "keys") => Ok(V::List(d.into_iter().map(|(k, _)| V::Str(k)).collect())),
                (V::Dict(d), "values") => Ok(V::List(d.into_iter().map(|(_, v)| v).collect())),
                (V::Dict(d), "get") => {
                    let k = text(&arg(0))?;
                    Ok(d.into_iter().find(|(x, _)| *x == k).map_or(arg(1), |(_, v)| v))
                }
                (V::Str(s), "upper") => Ok(V::Str(s.to_uppercase())),
                (V::Str(s), "lower") => Ok(V::Str(s.to_lowercase())),
                (V::Str(s), "format") => {
                    let mut out = s.clone();
                    for a in &args {
                        out = out.replacen("{}", &show(a), 1);
                    }
                    Ok(V::Str(out))
                }
                (V::Coll(c), m) => collection(app, &c, m, &args, &kw),
                (V::Img(i), m) => image(app, &i, m, &args, &kw),
                (o, m) => err(format!("{} has no method '{m}'", show(&o))),
            }
        }
        other => err(format!("{} isn't a function", show(&other))),
    }
}

fn builtin(app: &mut RustGisApp, b: &str, args: &[V], kw: &[(String, V)]) -> R<V> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(V::None);
    match b {
        "print" => {
            // Values that need images made are made first; the line is printed once.
            let line = args.iter().map(show).collect::<Vec<_>>().join(" ");
            if !effect_done_before(app) {
                app.ui.rslab.lee.out.push(Out::Text(line));
            }
            Ok(V::None)
        }
        "round" => {
            let n = num(&arg(0))?;
            let d = match args.get(1) {
                Some(v) => num(v)? as i32,
                None => 0,
            };
            let f = 10f64.powi(d);
            Ok(V::Num((n * f).round() / f))
        }
        "len" => Ok(V::Num(match arg(0) {
            V::List(l) => l.len(),
            V::Dict(d) => d.len(),
            V::Str(s) => s.chars().count(),
            _ => return err("len takes a list, a dict or text"),
        } as f64)),
        "str" => Ok(V::Str(show(&arg(0)))),
        "int" => Ok(V::Num(num(&arg(0))?.trunc())),
        "float" => Ok(V::Num(num(&arg(0))?)),
        "abs" => Ok(V::Num(num(&arg(0))?.abs())),
        "min" | "max" => {
            let items = if args.len() == 1 { as_list(&arg(0))? } else { args.to_vec() };
            let nums: Vec<f64> = items.iter().map(num).collect::<R<_>>()?;
            let v = if b == "min" {
                nums.iter().copied().fold(f64::INFINITY, f64::min)
            } else {
                nums.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            };
            Ok(V::Num(v))
        }
        "sum" => Ok(V::Num(as_list(&arg(0))?.iter().map(num).sum::<R<f64>>()?)),
        "range" => {
            let (a, z) = if args.len() >= 2 { (num(&arg(0))?, num(&arg(1))?) } else { (0.0, num(&arg(0))?) };
            Ok(V::List((a as i64..(z as i64).min(a as i64 + 10_000)).map(|i| V::Num(i as f64)).collect()))
        }
        "sorted" => {
            let mut l = as_list(&arg(0))?;
            l.sort_by(|x, y| match (x, y) {
                (V::Num(a), V::Num(b)) => a.total_cmp(b),
                _ => show(x).cmp(&show(y)),
            });
            if kwarg(kw, "reverse").is_some_and(truthy) {
                l.reverse();
            }
            Ok(V::List(l))
        }
        "grid.scale" => Ok(V::Num(cell(app))),
        "lee.Image" => Ok(V::Img(Rc::new(Img::Layer(text(&arg(0))?)))),
        o => err(format!("'{o}' isn't a function")),
    }
}

fn cell(app: &RustGisApp) -> f64 {
    let s = app.ui.rslab.lee.scale;
    if s > 0.0 { s } else { app.ui.rslab.cell }
}

fn collection(app: &mut RustGisApp, c: &Rc<Coll>, m: &str, args: &[V], kw: &[(String, V)]) -> R<V> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(V::None);
    let with = |f: &dyn Fn(&mut Coll)| -> V {
        let mut n = (**c).clone();
        f(&mut n);
        V::Coll(Rc::new(n))
    };
    match m {
        "filterDate" => {
            let (a, b) = (text(&arg(0))?, text(&arg(1))?);
            Ok(with(&|n| {
                n.start.clone_from(&a);
                n.end.clone_from(&b);
            }))
        }
        "filterBounds" => Ok(V::Coll(c.clone())),
        "filterMetadata" => {
            let (prop, rel, v) = (text(&arg(0))?, text(&arg(1))?, num(&arg(2))?);
            if !prop.contains("cloud") || !matches!(rel.as_str(), "lt" | "less_than" | "lte") {
                return err("filterMetadata takes \"eo:cloud_cover\", \"lt\", a number here");
            }
            Ok(with(&|n| n.cloud = v))
        }
        "limit" => {
            let k = num(&arg(0))?.clamp(1.0, 12.0) as usize;
            Ok(with(&|n| n.limit = k))
        }
        "select" => {
            let b = texts(&arg(0))?;
            Ok(with(&|n| n.select.clone_from(&b)))
        }
        "size" => Ok(V::Num(scenes_of(app, c)?.len() as f64)),
        "median" | "mean" | "mosaic" | "first" => {
            let reducer: &'static str = match m {
                "median" => "median",
                "mean" => "mean",
                _ => "first",
            };
            let comp = Rc::new(Img::Composite((**c).clone(), reducer));
            if c.select.is_empty() { Ok(V::Img(comp)) } else { Ok(V::Img(Rc::new(Img::Select(comp, c.select.clone())))) }
        }
        "map" | "timeSeries" | "monthly" | "qualityMosaic" => err(format!("{m} isn't supported in KHGIS's LEE yet")),
        o => {
            let _ = kw;
            err(format!("ImageCollection has no method '{o}'"))
        }
    }
}

fn image(app: &mut RustGisApp, i: &Rc<Img>, m: &str, args: &[V], kw: &[(String, V)]) -> R<V> {
    let arg = |k: usize| args.get(k).cloned().unwrap_or(V::None);
    let img = |x: Img| Ok(V::Img(Rc::new(x)));
    let cmp = |sym: &str| -> R<V> {
        match arg(0) {
            V::Img(j) => img(Img::Math(i.clone(), Some(j), format!("a1 {sym} b1"))),
            v => img(Img::Math(i.clone(), None, format!("a1 {sym} ({})", num(&v)?))),
        }
    };
    match m {
        "normalizedDifference" => {
            let b = texts(&arg(0))?;
            let (x, y) = match b.as_slice() {
                [x, y] => (x.clone(), y.clone()),
                [x] => (x.clone(), text(&arg(1))?),
                _ => return err("normalizedDifference takes two band names"),
            };
            img(Img::NormDiff(i.clone(), x, y))
        }
        "expression" => img(Img::Expression(i.clone(), text(&arg(0))?)),
        "select" => img(Img::Select(i.clone(), texts(&arg(0))?)),
        "rename" => img(Img::Rename(i.clone(), texts(&arg(0))?)),
        "add" => binop("+", V::Img(i.clone()), arg(0)),
        "subtract" => binop("-", V::Img(i.clone()), arg(0)),
        "multiply" => binop("*", V::Img(i.clone()), arg(0)),
        "divide" => binop("/", V::Img(i.clone()), arg(0)),
        "gt" => cmp(">"),
        "lt" => cmp("<"),
        "gte" => cmp(">="),
        "lte" => cmp("<="),
        "eq" => cmp("=="),
        "neq" => cmp("!="),
        "And" | "And_" => cmp("&"),
        "Or" => cmp("|"),
        "Not" => img(Img::Math(i.clone(), None, "a1 == 0".into())),
        // Where the mask is 0 the image has no value (b1 / b1 is no value there).
        "updateMask" => match arg(0) {
            V::Img(j) => img(Img::Math(i.clone(), Some(j), "a1 * b1 / b1".into())),
            _ => err("updateMask takes an image"),
        },
        // The images are already the study area's: clip keeps them as they are.
        "clip" => Ok(V::Img(i.clone())),
        "slope" => img(Img::Terrain(i.clone(), "slope", 0.0, 0.0)),
        "aspect" => img(Img::Terrain(i.clone(), "aspect", 0.0, 0.0)),
        "hillshade" => {
            let az = args.first().map(num).transpose()?.unwrap_or(315.0);
            let alt = args.get(1).map(num).transpose()?.unwrap_or(45.0);
            img(Img::Terrain(i.clone(), "hillshade", az, alt))
        }
        "cluster" => {
            let k = kwarg(kw, "k").or(args.first()).map(num).transpose()?.unwrap_or(5.0).clamp(2.0, 30.0) as usize;
            let bands = kwarg(kw, "bands").map(texts).transpose()?.unwrap_or_default();
            img(Img::Cluster(i.clone(), k, bands))
        }
        "otsu" => {
            let layer = materialize(app, i)?;
            Ok(V::Num(otsu(app, &layer)?))
        }
        "reduceRegion" => {
            let reducer = match kwarg(kw, "reducer").or(args.first()) {
                Some(v) => text(v)?,
                None => "mean".into(),
            };
            let layer = materialize(app, i)?;
            reduce(app, &layer, &reducer, i.renamed())
        }
        "classAreas" => {
            let layer = materialize(app, i)?;
            match app.run("raster.classAreas", json!({"layer": layer})) {
                Ok(v) => Ok(V::Dict(
                    v["classes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| Some((c["class"].as_str()?.to_string(), V::Num((c["hectares"].as_f64()? * 10.0).round() / 10.0))))
                        .collect(),
                )),
                Err(e) => err(e),
            }
        }
        "bandNames" => {
            let layer = materialize(app, i)?;
            Ok(V::List(crate::rslab::layer_band_names(app, &layer).into_iter().map(V::Str).collect()))
        }
        o => err(format!("Image has no method '{o}'")),
    }
}

/// The dates a collection's search finds, searching first when it hasn't.
fn scenes_of(app: &mut RustGisApp, c: &Coll) -> R<Vec<crate::rslab::Scene>> {
    let bbox = crate::rslab::area_bbox(app).map_err(Stop::Err)?;
    let url = crate::rslab::search_url(c.dataset, bbox, &c.start, &c.end, c.cloud);
    if let Some(s) = app.ui.rslab.lee.searches.get(&url) {
        return Ok(s.clone());
    }
    if app.ui.rslab.lee.waiting.is_none() {
        app.ui.rslab.lee.out.push(Out::Text(crate::i18n::fmt("Searching {d}…", &[("d", &format!("{:?}", c.dataset))])));
        let slot = crate::rslab::fetch(&url);
        app.ui.rslab.lee.waiting = Some(Waiting::Search(url, slot, c.dataset));
    }
    Err(Stop::Wait)
}

/// A layer's band, by name (case and the satellites' own names alike), as 1-based.
fn band_of(app: &RustGisApp, layer: &str, name: &str) -> R<usize> {
    let names = crate::rslab::layer_band_names(app, layer);
    let alias = |n: &str| -> String {
        match n.to_ascii_lowercase().as_str() {
            "b2" | "sr_b2" | "b02" => "blue".into(),
            "b3" | "sr_b3" | "b03" => "green".into(),
            "b4" | "sr_b4" | "b04" => "red".into(),
            "b8" | "sr_b5" | "b08" | "nir08" => "nir".into(),
            "b11" | "sr_b6" | "swir16" => "swir1".into(),
            "b12" | "sr_b7" | "swir22" => "swir2".into(),
            "vv" | "vv db" => "vv db".into(),
            "vh" | "vh db" => "vh db".into(),
            o => o.into(),
        }
    };
    let want = alias(name);
    if let Some(i) = names.iter().position(|n| alias(n) == want) {
        return Ok(i + 1);
    }
    // A one-band image answers to any name (its own was set by rename).
    if names.len() == 1 {
        return Ok(1);
    }
    err(format!("{layer} has no band '{name}' (it has {})", names.join(", ")))
}

/// The layer an image recipe is (making it, or waiting while it's made).
fn materialize(app: &mut RustGisApp, img: &Rc<Img>) -> R<String> {
    let key = img.key();
    if let Some(l) = app.ui.rslab.lee.made.get(&key).cloned() {
        if crate::rslab::layer_ready(app, &l) {
            return Ok(l);
        }
        if crate::rslab::busy(app) {
            return Err(Stop::Wait);
        }
        return err(format!("{l} wasn't made: see the messages above"));
    }
    if crate::rslab::busy(app) || app.ui.rslab.lee.waiting.is_some() {
        return Err(Stop::Wait);
    }
    let mk = |cmd: &str, p: Json, out: &str| crate::rslab::makes(cmd, p, out);
    let (out, steps): (String, Vec<crate::rslab::Step>) = match &**img {
        Img::Layer(name) => {
            if app.session.map().layer_by_name(name).and_then(|l| l.raster.as_ref()).is_none() {
                return err(format!("no raster layer '{name}'"));
            }
            return Ok(name.clone());
        }
        Img::Composite(c, reducer) => {
            let found = scenes_of(app, c)?;
            if found.is_empty() {
                return err(crate::i18n::fmt("No {d} images found for those dates and clouds.", &[("d", &format!("{:?}", c.dataset))]));
            }
            let chosen: Vec<crate::rslab::Scene> =
                if *reducer == "first" { found.into_iter().take(1).collect() } else { found.into_iter().take(c.limit).collect() };
            crate::rslab::composite_steps(app, c.dataset, &chosen, cell(app), *reducer != "first").map_err(Stop::Err)?
        }
        Img::Load(ds, year) => {
            let c = Coll { dataset: *ds, start: String::new(), end: String::new(), cloud: 100.0, limit: 1, select: Vec::new() };
            let found = scenes_of(app, &c)?;
            let pick = match year {
                Some(y) => found.iter().find(|s| s.date == *y).cloned(),
                None => found.first().cloned(),
            };
            let Some(scene) = pick else {
                return err(format!("no {ds:?} for {}", year.clone().unwrap_or_default()));
            };
            crate::rslab::composite_steps(app, *ds, &[scene], cell(app), false).map_err(Stop::Err)?
        }
        Img::Math(a, b, expr) => {
            let la = materialize(app, a)?;
            let lb = match b {
                Some(b) => Some(materialize(app, b)?),
                None => None,
            };
            let out = crate::rslab::fresh(app, "lee_math");
            // Two images: stacked (the second put on the first's grid), then `b…` are the second's.
            let na = crate::rslab::layer_band_names(app, &la).len().max(1);
            let rewrite = |e: &str, shift: usize| -> String {
                let mut s = String::new();
                let c: Vec<char> = e.chars().collect();
                let mut k = 0;
                while k < c.len() {
                    if (c[k] == 'a' || c[k] == 'b') && c.get(k + 1).is_some_and(char::is_ascii_digit) {
                        let mut j = k + 1;
                        while j < c.len() && c[j].is_ascii_digit() {
                            j += 1;
                        }
                        let n: usize = c[k + 1..j].iter().collect::<String>().parse().unwrap_or(1);
                        s.push_str(&format!("b{}", if c[k] == 'a' { n } else { n + shift }));
                        k = j;
                    } else {
                        s.push(c[k]);
                        k += 1;
                    }
                }
                s
            };
            match lb {
                Some(lb) if lb != la => {
                    let stack = crate::rslab::fresh(app, "lee_stack");
                    let expr = rewrite(expr, na);
                    (
                        out.clone(),
                        vec![
                            mk("raster.process", json!({"layer": la, "operation": "stack", "layers": [lb], "name": stack}), &stack),
                            mk("raster.function", json!({"layer": stack, "function": "bandMath", "expression": expr, "name": out}), &out),
                            crate::rslab::step("layer.remove", json!({"layer": stack})),
                        ],
                    )
                }
                _ => {
                    let expr = rewrite(expr, 0);
                    (
                        out.clone(),
                        vec![mk("raster.function", json!({"layer": la, "function": "bandMath", "expression": expr, "name": out}), &out)],
                    )
                }
            }
        }
        Img::NormDiff(a, x, y) => {
            let la = materialize(app, a)?;
            let (bx, by) = (band_of(app, &la, x)?, band_of(app, &la, y)?);
            let out = crate::rslab::fresh(app, "lee_nd");
            // Kept to −1…1: where both bands are near 0 (or one below it, as reflectance can be) the
            // ratio runs off.
            let e = format!("max(-1, min(1, (b{bx} - b{by}) / (b{bx} + b{by})))");
            (out.clone(), vec![mk("raster.function", json!({"layer": la, "function": "bandMath", "expression": e, "name": out}), &out)])
        }
        Img::Expression(a, text) => {
            let la = materialize(app, a)?;
            // Band names (longest first) become b1, b2…
            let mut names: Vec<(String, usize)> =
                crate::rslab::layer_band_names(app, &la).into_iter().enumerate().map(|(i, n)| (n.to_ascii_lowercase(), i + 1)).collect();
            names.sort_by_key(|n| std::cmp::Reverse(n.0.len()));
            let mut e = String::new();
            let c: Vec<char> = text.chars().collect();
            let mut k = 0;
            while k < c.len() {
                if c[k].is_alphabetic() {
                    let mut j = k;
                    while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_') {
                        j += 1;
                    }
                    let w: String = c[k..j].iter().collect();
                    let lw = w.to_ascii_lowercase();
                    match names.iter().find(|(n, _)| *n == lw) {
                        Some((_, b)) => e.push_str(&format!("b{b}")),
                        None => match band_of(app, &la, &w) {
                            Ok(b) if !["sqrt", "abs", "log", "log10", "exp", "min", "max"].contains(&lw.as_str()) => {
                                e.push_str(&format!("b{b}"))
                            }
                            _ => e.push_str(&w),
                        },
                    }
                    k = j;
                } else {
                    e.push(c[k]);
                    k += 1;
                }
            }
            let out = crate::rslab::fresh(app, "lee_expr");
            (out.clone(), vec![mk("raster.function", json!({"layer": la, "function": "bandMath", "expression": e, "name": out}), &out)])
        }
        Img::Select(a, names) => {
            let la = materialize(app, a)?;
            let bands: Vec<usize> = names.iter().map(|n| band_of(app, &la, n)).collect::<R<_>>()?;
            let out = crate::rslab::fresh(app, "lee_select");
            (out.clone(), vec![mk("raster.process", json!({"layer": la, "operation": "select", "bands": bands, "name": out}), &out)])
        }
        // A rename is the same layer; its names are the recipe's.
        Img::Rename(a, _) => {
            let la = materialize(app, a)?;
            app.ui.rslab.lee.made.insert(key, la.clone());
            return Ok(la);
        }
        Img::Terrain(a, f, az, alt) => {
            let la = materialize(app, a)?;
            let out = crate::rslab::fresh(app, &format!("lee_{f}"));
            let mut p = json!({"layer": la, "function": f, "name": out});
            if *f == "hillshade" {
                p["azimuth"] = json!(az);
                p["altitude"] = json!(alt);
            }
            (out.clone(), vec![mk("raster.function", p, &out)])
        }
        Img::Cluster(a, k, names) => {
            let la = materialize(app, a)?;
            let bands: Vec<usize> = if names.is_empty() {
                (1..=crate::rslab::layer_band_names(app, &la).len()).collect()
            } else {
                names.iter().map(|n| band_of(app, &la, n)).collect::<R<_>>()?
            };
            let out = crate::rslab::fresh(app, "lee_clusters");
            (out.clone(), vec![mk("raster.classify", json!({"layer": la, "classes": k, "bands": bands, "name": out}), &out)])
        }
    };
    app.ui.rslab.lee.made.insert(key, out.clone());
    crate::rslab::start_chain(app, tl!("LEE script").to_string(), steps, String::new());
    Err(Stop::Wait)
}

/// A layer drawn as a script asks: its name, bands, range and palette.
fn draw(app: &mut RustGisApp, layer: &str, name: &str, vis: Option<&V>) {
    let shown = if name == layer { name.to_string() } else { crate::rslab::fresh(app, name) };
    // Drawn the first time, the layer itself takes the name (the recipes that made it follow);
    // drawn again (other bands, another palette), a copy of it is.
    let ours = app.ui.rslab.lee.made.values().any(|v| v == layer);
    let target = if app.ui.rslab.lee.drawn.contains(layer) || (!ours && shown != layer) {
        let n = crate::rslab::layer_band_names(app, layer).len().max(1);
        match app.run("raster.process", json!({"layer": layer, "operation": "select", "bands": (1..=n).collect::<Vec<_>>(), "name": shown}))
        {
            Ok(_) => shown,
            Err(_) => layer.to_string(),
        }
    } else if shown != layer && app.run("layer.rename", json!({"layer": layer, "name": shown})).is_ok() {
        for v in app.ui.rslab.lee.made.values_mut() {
            if v == layer {
                v.clone_from(&shown);
            }
        }
        shown
    } else {
        layer.to_string()
    };
    app.ui.rslab.lee.drawn.insert(target.clone());
    let vis = match vis {
        Some(V::Dict(d)) => d.clone(),
        _ => Vec::new(),
    };
    let get = |k: &str| vis.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    if let Some(b) = get("bands").and_then(|b| texts(&b).ok())
        && b.len() == 3
    {
        let idx: Vec<usize> = b.iter().filter_map(|n| band_of(app, &target, n).ok()).collect();
        if idx.len() == 3 {
            let _ = app.run("raster.bands", json!({"layer": target, "red": idx[0], "green": idx[1], "blue": idx[2]}));
        }
    } else if let Some(p) = get("palette").and_then(|p| text(&p).ok()) {
        let (ramp, invert) = ramp_for(&p);
        let _ = app.run("raster.symbology", json!({"layer": target, "type": "stretch", "ramp": ramp, "invert": invert}));
    }
    match (get("min").and_then(|v| num(&v).ok()), get("max").and_then(|v| num(&v).ok())) {
        (Some(lo), Some(hi)) => {
            let _ = app.run("raster.stretch", json!({"layer": target, "type": "custom", "min": lo, "max": hi}));
        }
        _ if get("bands").is_some() => {
            let _ = app.run("raster.stretch", json!({"layer": target, "type": "percentClip", "low": 2, "high": 2}));
        }
        _ => {}
    }
    let _ = app.run("layer.visibility", json!({"layer": target, "visible": true}));
    app.map.invalidate();
}

/// KHGIS's colour ramp for an Earth Engine / ColorBrewer palette name, and whether reversed.
fn ramp_for(p: &str) -> (&'static str, bool) {
    let (name, rev) = match p.strip_suffix("_r") {
        Some(n) => (n, true),
        None => (p, false),
    };
    let ramp = match name.to_ascii_lowercase().as_str() {
        "rdylgn" | "ylgn" | "greens" | "ylgnbu" => "Forest",
        "blues" | "pubu" | "gnbu" => "Lagoon",
        "ylorrd" | "reds" | "orrd" | "ylorbr" | "oranges" | "hot" => "Ember",
        "spectral" | "jet" | "rainbow" | "turbo" => "Turbo",
        "brbg" | "rdbu" | "rdylbu" | "piyg" | "coolwarm" => "Diverging",
        "greys" | "gray" | "grey" => "Greys",
        "viridis" => "Viridis",
        "magma" => "Magma",
        "inferno" => "Inferno",
        "plasma" => "Plasma",
        "terrain" | "gist_earth" => "Terrain",
        _ => "Viridis",
    };
    // ColorBrewer's red-to-green and red-to-blue run the other way to KHGIS's.
    let flip = matches!(name.to_ascii_lowercase().as_str(), "rdbu" | "rdylbu");
    (ramp, rev ^ flip)
}

/// A statistic of each band of a layer over the area: {band: value}.
fn reduce(app: &RustGisApp, layer: &str, reducer: &str, renamed: Option<Vec<String>>) -> R<V> {
    let Some(r) = app.session.map().layer_by_name(layer).and_then(|l| l.raster.as_ref()?.data.clone()) else {
        return err(format!("no raster layer '{layer}'"));
    };
    let mut out = Vec::new();
    for (i, b) in r.bands.iter().enumerate() {
        let mut v: Vec<f64> = b.data.iter().copied().filter(|x| r.is_valid(*x) && x.is_finite() && *x > -1e30).map(f64::from).collect();
        let name = renamed
            .as_ref()
            .and_then(|n| n.get(i).cloned())
            .unwrap_or_else(|| if b.name.is_empty() { format!("b{}", i + 1) } else { b.name.clone() });
        let x = match reducer {
            "mean" => v.iter().sum::<f64>() / v.len().max(1) as f64,
            "sum" => v.iter().sum(),
            "min" => v.iter().copied().fold(f64::INFINITY, f64::min),
            "max" => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "count" => v.len() as f64,
            "median" => {
                v.sort_by(f64::total_cmp);
                v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
            }
            "stdDev" | "std" => {
                let m = v.iter().sum::<f64>() / v.len().max(1) as f64;
                (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len().max(1) as f64).sqrt()
            }
            o => return err(format!("unknown reducer '{o}': mean, median, min, max, sum, count, stdDev")),
        };
        out.push((name, V::Num(x)));
    }
    Ok(V::Dict(out))
}

/// Otsu's threshold of a layer's first band (256 bins between its range).
fn otsu(app: &RustGisApp, layer: &str) -> R<f64> {
    let Some(r) = app.session.map().layer_by_name(layer).and_then(|l| l.raster.as_ref()?.data.clone()) else {
        return err(format!("no raster layer '{layer}'"));
    };
    let v: Vec<f64> = r
        .bands
        .first()
        .map(|b| b.data.iter().copied().filter(|x| r.is_valid(*x) && x.is_finite() && *x > -1e30).map(f64::from).collect())
        .unwrap_or_default();
    Ok(otsu_of(&v))
}

pub(crate) fn otsu_of(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let (lo, hi) = (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    if hi <= lo {
        return lo;
    }
    let mut hist = [0f64; 256];
    for x in v {
        hist[(((x - lo) / (hi - lo)) * 255.0).round().clamp(0.0, 255.0) as usize] += 1.0;
    }
    let total = v.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, h)| i as f64 * h).sum();
    // The middle of the bins as good as the best (an empty gap between two groups is flat).
    let (mut wb, mut sb, mut best, mut at, mut last) = (0.0, 0.0, -1.0, 0usize, 0usize);
    for (i, h) in hist.iter().enumerate() {
        wb += h;
        if wb == 0.0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0.0 {
            break;
        }
        sb += i as f64 * h;
        let (mb, mf) = (sb / wb, (sum - sb) / wf);
        let between = wb * wf * (mb - mf).powi(2);
        if between > best * (1.0 + 1e-9) {
            best = between;
            at = i;
            last = i;
        } else if between >= best * (1.0 - 1e-9) {
            last = i;
        }
    }
    lo + ((at + last) as f64 / 2.0 + 0.5) / 255.0 * (hi - lo)
}

/// The LEE tab: examples, the editor, Run / Stop, and what the script printed.
pub fn pane(app: &mut RustGisApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(tl!("Earth-Engine-style scripts (Python), run on your computer: the study area of tab 1 is `aoi`.")).weak(),
    );
    if app.ui.rslab.lee.code.is_empty() {
        app.ui.rslab.lee.code = EXAMPLES[0].1.to_string();
    }
    let lee = &mut app.ui.rslab.lee;
    let mut load = None;
    ui.horizontal(|ui| {
        ui.label(tl!("Example"));
        egui::ComboBox::from_id_salt("lee_example")
            .selected_text(tl!(EXAMPLES[lee.example.min(EXAMPLES.len() - 1)].0))
            .width((ui.available_width() - 8.0).max(120.0))
            .truncate()
            .show_ui(ui, |ui| {
                for (i, (title, _)) in EXAMPLES.iter().enumerate() {
                    if ui.selectable_label(lee.example == i, tl!(title)).clicked() {
                        load = Some(i);
                    }
                }
            });
    });
    if let Some(i) = load {
        lee.example = i;
        lee.code = EXAMPLES[i].1.to_string();
    }
    egui::ScrollArea::vertical().id_salt("lee_code").max_height(300.0).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut lee.code)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(14)
                .font(egui::TextStyle::Monospace),
        );
    });
    let running = lee.running;
    let mut go = false;
    let mut halt = false;
    ui.horizontal(|ui| {
        if running {
            ui.spinner();
            halt = ui.button(tl!("Stop")).clicked();
        } else {
            go = ui.button(egui::RichText::new(tl!("▶ Run script")).strong()).clicked();
        }
        if ui.small_button(tl!("Clear output")).clicked() {
            app.ui.rslab.lee.out.clear();
        }
        if ui.small_button(tl!("Forget made images")).on_hover_text(tl!("Run everything again from the downloads")).clicked() {
            app.ui.rslab.lee.made.clear();
            app.ui.rslab.lee.searches.clear();
        }
    });
    ui.horizontal(|ui| {
        if ui.small_button(tl!("Save…")).on_hover_text(tl!("Save the script as a .py file")).clicked() {
            crate::rsclasses::save_script(app);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if ui.small_button(tl!("Open…")).on_hover_text(tl!("Open a .py script")).clicked() {
            crate::rsclasses::open_script(app);
        }
    });
    egui::CollapsingHeader::new(tl!("API: what scripts can use")).id_salt("lee_api").show(ui, |ui| {
        for (group, items) in API {
            ui.label(egui::RichText::new(*group).strong());
            ui.label(egui::RichText::new(*items).monospace().small());
        }
    });
    if go {
        start(app);
    }
    if halt {
        stop(app);
    }
    let out = app.ui.rslab.lee.out.clone();
    if out.is_empty() {
        return;
    }
    ui.separator();
    for o in &out {
        match o {
            Out::Text(s) => {
                ui.label(egui::RichText::new(s).monospace());
            }
            Out::Error(e) => {
                ui.label(egui::RichText::new(format!("✗ {e}")).monospace().color(t.red));
            }
            Out::Bars(title, bars) => bars_chart(ui, title, bars),
        }
    }
}

fn bars_chart(ui: &mut egui::Ui, title: &str, bars: &[(String, f64)]) {
    let t = crate::theme::Tokens::get(ui.ctx());
    if !title.is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }
    let most = bars.iter().map(|b| b.1.abs()).fold(0.0, f64::max).max(1e-12);
    for (i, (name, v)) in bars.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized([120.0, 14.0], egui::Label::new(egui::RichText::new(name).small()).truncate());
            let w = 160.0 * (v.abs() / most) as f32;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(w.max(1.0), 12.0)), 2.0, crate::rslab::class_color(i));
            ui.label(egui::RichText::new(show(&V::Num((v * 1000.0).round() / 1000.0))).small().color(t.text_dim));
        });
    }
}

/// What scripts can use, by object: (object, its functions).
pub const API: &[(&str, &str)] = &[
    ("aoi", "the study area of tab 1"),
    (
        "lee.ImageCollection(name)",
        "\"sentinel-2\", \"landsat-9\", \"landsat-8\", \"landsat-7\", \"landsat-5\", \"sentinel-1\"\n.filterDate(start, end) .filterBounds(aoi) .filterMetadata(\"eo:cloud_cover\", \"lt\", n)\n.limit(n) .select(bands) .size() .median() .mean() .mosaic() .first()",
    ),
    ("lee.Image", "lee.Image.load(\"dem\" | \"worldcover\" | \"lulc-annual\" | \"surface-water\", year)\nlee.Image(\"a raster layer of the project\")"),
    (
        "image",
        ".normalizedDifference(a, b) .expression(\"(nir-red)/(nir+red)\") .select(b) .rename(n)\n+ - * / ** .add .subtract .multiply .divide\n.gt .lt .gte .lte .eq .neq .And .Or .Not .updateMask(m) .clip(aoi)\n.slope() .aspect() .hillshade(az, alt) .cluster(k=, bands=) .otsu()\n.reduceRegion(\"mean\"|\"median\"|\"min\"|\"max\"|\"sum\"|\"count\"|\"stdDev\")\n.classAreas() .bandNames() .grid.scale",
    ),
    ("Map", "Map.addLayer(image, {\"bands\": [r, g, b], \"min\": , \"max\": , \"palette\": \"RdYlGn\"}, name)\nMap.centerObject(aoi)"),
    ("Chart, lee", "Chart.bars(dict, title)   lee.config(scale=30)"),
    ("Python", "= for … in …: print round len str int float abs min max sum sorted range, \"%s\" % x, \"{}\".format(x), d[k] = v, dict.items() keys() values() get()"),
];

/// Ready-to-run scripts: (title, code).
pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "True colour + NDVI (dry-season median)",
        r#"# Cloud-masked median of the dry season, then NDVI — like GEE, but on your computer
s2 = (lee.ImageCollection("sentinel-2")
        .filterDate("2025-11-01", "2026-03-31")
        .filterBounds(aoi)
        .filterMetadata("eo:cloud_cover", "lt", 30)
        .limit(6))                       # the 6 clearest dates
print("dates found:", s2.size())
img = s2.median()
Map.addLayer(img, {"bands": ["red", "green", "blue"], "min": 0, "max": 0.3}, "True colour")
Map.addLayer(img, {"bands": ["nir", "red", "green"], "min": 0, "max": 0.4}, "False colour")
ndvi = img.normalizedDifference("nir", "red").rename("NDVI")
Map.addLayer(ndvi, {"min": 0, "max": 0.9, "palette": "RdYlGn"}, "NDVI")
print("mean NDVI in the AOI:", ndvi.reduceRegion("mean"))
"#,
    ),
    (
        "Water mapping (MNDWI + Otsu)",
        r#"img = (lee.ImageCollection("sentinel-2").filterDate("2025-09-01", "2025-11-30")
        .filterMetadata("eo:cloud_cover", "lt", 40).limit(4).median())
mndwi = img.normalizedDifference("green", "swir1").rename("MNDWI")
t = mndwi.otsu()                                  # automatic threshold from the histogram
water = mndwi.gt(t).rename("water")
print("Otsu threshold:", round(t, 3))
print("water area (ha):", round(water.reduceRegion("sum")["water"] * img.grid.scale ** 2 / 1e4, 1))
Map.addLayer(mndwi, {"min": -0.6, "max": 0.6, "palette": "Blues"}, "MNDWI")
Map.addLayer(water.updateMask(water), {"palette": "Blues"}, "Water")
"#,
    ),
    (
        "Terrain: slope, aspect, hillshade (Copernicus DEM)",
        r#"lee.config(scale=30)
dem = lee.Image.load("dem")          # Copernicus GLO-30, tiles mosaicked and clipped to the AOI
print("elevation:", dem.reduceRegion("min"), dem.reduceRegion("max"))
Map.addLayer(dem, {"palette": "terrain"}, "Elevation (m)")
Map.addLayer(dem.hillshade(315, 45), {}, "Hillshade")
Map.addLayer(dem.slope(), {"min": 0, "max": 30, "palette": "YlOrRd"}, "Slope (°)")
Map.addLayer(dem.aspect(), {"min": 0, "max": 360, "palette": "Spectral"}, "Aspect (°)")
"#,
    ),
    (
        "Land-cover statistics (ESA WorldCover)",
        r#"wc = lee.Image.load("worldcover", 2021)
Map.addLayer(wc, {}, "ESA WorldCover 2021")
areas = wc.classAreas()
for name, ha in areas.items():
    print("%-28s %10.1f ha" % (name, ha))
Chart.bars(areas, "Land cover (ha)", "ha")
"#,
    ),
    (
        "Land-cover change 2017 → 2023 (annual LULC)",
        r#"a = lee.Image.load("lulc-annual", 2017)
b = lee.Image.load("lulc-annual", 2023)
Map.addLayer(a, {}, "Land cover 2017")
Map.addLayer(b, {}, "Land cover 2023")
changed = a.neq(b).rename("changed")
print("changed area (ha):", round(changed.reduceRegion("sum")["changed"] * a.grid.scale ** 2 / 1e4, 1))
Map.addLayer(changed.updateMask(changed), {"palette": "Reds"}, "Changed 2017–2023")
"#,
    ),
    (
        "Burn severity dNBR (Landsat before / after)",
        r#"pre = lee.ImageCollection("landsat-9").filterDate("2025-01-01", "2025-02-15").limit(3).median()
post = lee.ImageCollection("landsat-9").filterDate("2025-04-01", "2025-05-15").limit(3).median()
dnbr = (pre.normalizedDifference("nir", "swir2") - post.normalizedDifference("nir", "swir2")).rename("dNBR")
Map.addLayer(dnbr, {"min": -0.25, "max": 0.66, "palette": "YlOrRd"}, "dNBR")
burnt = dnbr.gt(0.27)                          # moderate-to-high severity (USGS classes)
print("burnt area > 0.27 (ha):", round(burnt.reduceRegion("sum")["dNBR"] * pre.grid.scale ** 2 / 1e4, 1))
"#,
    ),
    (
        "Flood mapping with Sentinel-1 (before / after)",
        r#"# Radar sees through clouds: water is dark (low VV backscatter)
before = lee.ImageCollection("sentinel-1").filterDate("2025-04-01", "2025-04-30").select("VV").limit(3).median()
after = lee.ImageCollection("sentinel-1").filterDate("2025-09-15", "2025-10-15").select("VV").limit(3).median()
diff = (after - before).rename("dVV")
flood = after.lt(-18).And(diff.lt(-3)).rename("flood")
print("flooded area (ha):", round(flood.reduceRegion("sum")["flood"] * after.grid.scale ** 2 / 1e4, 1))
Map.addLayer(after, {"min": -25, "max": 0}, "VV after (dB)")
Map.addLayer(flood.updateMask(flood), {"palette": "Blues"}, "Flood")
"#,
    ),
    (
        "Unsupervised clustering (K-means)",
        r#"img = lee.ImageCollection("sentinel-2").filterDate("2025-12-01", "2026-02-28").limit(4).median()
cl = img.cluster(k=6, bands=["blue", "green", "red", "nir", "swir1", "swir2"])
Map.addLayer(cl, {"min": 1, "max": 6, "palette": "Spectral"}, "K-means 6 clusters")
print(cl.classAreas())
"#,
    ),
    (
        "Surface-water occurrence 1984–2021 (JRC)",
        r#"occ = lee.Image.load("surface-water")
Map.addLayer(occ, {"min": 0, "max": 100, "palette": "Blues"}, "Water occurrence (%)")
perm = occ.gt(90).rename("permanent")
print("permanent water > 90% (ha):", round(perm.reduceRegion("sum")["permanent"] * occ.grid.scale ** 2 / 1e4, 1))
"#,
    ),
    (
        "Vegetation & soil indices pack (EVI, SAVI, NDMI, NDBI, BSI)",
        r#"# Several indices from one dry-season composite, with expression() for the formulas
img = (lee.ImageCollection("sentinel-2").filterDate("2025-12-01", "2026-03-31")
        .filterMetadata("eo:cloud_cover", "lt", 30).limit(4).median())
idx = {
    "EVI":  "2.5*(nir-red)/(nir+6*red-7.5*blue+1)",
    "SAVI": "1.5*(nir-red)/(nir+red+0.5)",
    "NDMI": "(nir-swir1)/(nir+swir1)",
    "NDBI": "(swir1-nir)/(swir1+nir)",
    "BSI":  "((swir1+red)-(nir+blue))/((swir1+red)+(nir+blue))",
}
ramps = {"EVI": "RdYlGn", "SAVI": "RdYlGn", "NDMI": "BrBG", "NDBI": "Reds", "BSI": "YlOrBr"}
means = {}
for name, f in idx.items():
    im = img.expression(f).rename(name)
    Map.addLayer(im, {"min": -0.5, "max": 0.8, "palette": ramps[name]}, name)
    means[name] = round(im.reduceRegion("mean")[name], 3)
print("AOI means:", means)
Chart.bars(means, "Mean index value in the AOI", "index")
"#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_example_parses() {
        for (title, code) in EXAMPLES {
            let p = parse(code).unwrap_or_else(|e| panic!("{title}: {e}"));
            assert!(!p.is_empty(), "{title}");
        }
    }

    #[test]
    fn scripts_parse_loops_dicts_and_errors() {
        let p = parse("a = 1\nfor k, v in {\"x\": 1}.items():\n    print(k, v)\nb = [1, 2]\n").unwrap();
        assert_eq!(p.len(), 3);
        assert!(matches!(&p[1], S::For(v, _, body) if v.len() == 2 && body.len() == 1));
        assert!(parse("x = (1,\n 2").is_err());
        assert!(parse("  x = 1\ny = 2").is_err());
        assert!(parse("f = lambda i: i").unwrap_err().contains("lambda"));
    }

    #[test]
    fn values_print_and_format_as_python() {
        assert_eq!(percent_format("%-6s %6.1f ha", &V::List(vec![V::Str("Crop".into()), V::Num(12.345)])), "Crop     12.3 ha");
        assert_eq!(show(&V::Dict(vec![("NDVI".into(), V::Num(0.5))])), "{'NDVI': 0.5}");
        assert_eq!(show(&V::Num(3.0)), "3");
        assert!(matches!(binop("+", V::Num(1.0), V::Num(2.0)), Ok(V::Num(n)) if n == 3.0));
        assert!(matches!(binop(">", V::Num(1.0), V::Num(2.0)), Ok(V::Bool(false))));
    }

    #[test]
    fn otsu_splits_two_groups() {
        let mut v: Vec<f64> = (0..100).map(|i| 0.1 + (i % 5) as f64 * 0.01).collect();
        v.extend((0..100).map(|i| 0.8 + (i % 5) as f64 * 0.01));
        let t = otsu_of(&v);
        assert!(t > 0.15 && t < 0.8, "{t}");
    }

    /// A script over a raster of the project: an index, a statistic, a threshold and a layer
    /// drawn, run as the app runs it (steps in the background, the script again when they land).
    #[test]
    fn a_script_runs_over_a_project_raster() {
        use rustgis_engine::{Project, Session};
        let dir = std::env::temp_dir().join(format!("khgis-lee-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = RustGisApp::new(Session::new(Project::default()));
        let mut names = Vec::new();
        for (file, low, high) in [("red.asc", 10, 20), ("nir.asc", 12, 80)] {
            let rows: String =
                (0..10).map(|_| (0..10).map(|c| if c < 5 { low } else { high }.to_string()).collect::<Vec<_>>().join(" ") + "\n").collect();
            let path = dir.join(file);
            std::fs::write(&path, format!("ncols 10\nnrows 10\nxllcorner 104\nyllcorner 11\ncellsize 0.001\n{rows}")).unwrap();
            let id = app.run("layer.addData", json!({"path": path})).unwrap()["layer"].as_u64().unwrap();
            names.push(app.session.map().layer(id as u32).unwrap().name.clone());
        }
        app.run(
            "raster.process",
            json!({"layer": names[0], "operation": "stack", "layers": [names[1]], "bandNames": ["red", "nir"], "name": "img"}),
        )
        .unwrap();
        app.ui.rslab.lee.code = r#"img = lee.Image("img")
nd = img.normalizedDifference("nir", "red").rename("ND")
print("mean", round(nd.reduceRegion("mean")["ND"], 3))
veg = nd.gt(0.3).rename("veg")
print("veg pixels", veg.reduceRegion("sum"))
Map.addLayer(nd, {"min": 0, "max": 1, "palette": "RdYlGn"}, "ND")
for k, v in {"a": 1, "b": 2}.items():
    print("%s=%d" % (k, v))
m = {}
m["x"] = 2
print(m)
"#
        .into();
        start(&mut app);
        let ctx = egui::Context::default();
        let t0 = std::time::Instant::now();
        while app.ui.rslab.lee.running() && t0.elapsed().as_secs() < 60 {
            crate::analysispane::land_job(&mut app);
            crate::rslab::poll(&mut app, &ctx);
            poll(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let out: Vec<String> = app.ui.rslab.lee.out.iter().map(|o| format!("{o:?}")).collect();
        let text = out.join("\n");
        assert!(!app.ui.rslab.lee.running(), "{text}");
        // (80-20)/(80+20) = 0.6 on half, (12-10)/22 ≈ 0.091 on the other.
        assert!(text.contains("mean 0.345"), "{text}");
        assert!(text.contains("veg pixels {'veg': 50}"), "{text}");
        assert!(text.contains("{'x': 2}"), "{text}");
        assert!(text.contains("a=1") && text.contains("b=2") && text.contains("Done"), "{text}");
        assert_eq!(text.matches("mean").count(), 1, "printed once: {text}");
        assert!(app.session.map().layer_by_name("ND").is_some(), "{text}");
    }

    #[test]
    fn palettes_map_to_ramps() {
        assert_eq!(ramp_for("RdYlGn"), ("Forest", false));
        assert_eq!(ramp_for("Spectral_r"), ("Turbo", true));
    }
}
