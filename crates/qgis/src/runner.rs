//! The native QGIS bridge: the bridge script run with the Python that comes with QGIS.
//!
//! KHGIS finds QGIS's Python in the usual places (or the one the user picked), writes the
//! bridge script to its data folder, and runs it: `version` to check it, `manifests` once when
//! QGIS is connected (cached in `qgis/manifests.json`, so later starts don't start QGIS), and
//! `run` for each algorithm run, with the job on standard input and progress lines back. A
//! crash in QGIS or a plugin takes only that process down.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use rustgis_analysis::ToolResult;
use serde_json::{Value, json};

use crate::{BRIDGE_SCRIPT, BRIDGE_VERSION, Job, finish};

/// The environment variable naming QGIS's Python, ahead of every other place.
pub const ENV: &str = "KHGIS_QGIS_PYTHON";

/// How long checking a Python may take (QGIS starts in a few seconds; a cold start more).
const PROBE_LIMIT: Duration = Duration::from_secs(120);
/// How long listing the algorithms may take (every plugin is loaded).
const MANIFEST_LIMIT: Duration = Duration::from_secs(600);
/// The longest line read from the bridge (the manifests of QGIS with many plugins).
const MAX_LINE: usize = 64 * 1024 * 1024;
/// The most log lines kept from one run.
const MAX_LOGS: usize = 200;

/// A Python the user chose, ahead of the usual places.
static CHOSEN: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Uses `python` (QGIS's Python, or `python-qgis.bat`) ahead of the usual places; `None` goes
/// back to looking for one.
pub fn choose(python: Option<PathBuf>) {
    if let Ok(mut c) = CHOSEN.lock() {
        *c = python;
    }
}

/// The Python the user chose, if any.
pub fn chosen() -> Option<PathBuf> {
    CHOSEN.lock().ok().and_then(|c| c.clone())
}

fn data_dir() -> Result<PathBuf, String> {
    rustgis_data::dirs::app_data_dir().map(|d| d.join("qgis")).ok_or_else(|| "there is no folder for KHGIS's data".to_string())
}

fn cache_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("manifests.json"))
}

/// The bridge script on disk, written again when this KHGIS carries another version.
fn script() -> Result<PathBuf, String> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("khgis_qgis_bridge.py");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(BRIDGE_SCRIPT) {
        std::fs::write(&path, BRIDGE_SCRIPT).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(path)
}

/// Pythons QGIS installs put in the usual places, newest-looking first.
pub fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os(ENV).filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(p));
    }
    let mut glob = |dir: &Path, prefix: &str, tail: &[&str]| {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut found: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .map(|e| tail.iter().fold(e.path(), |p, t| p.join(t)))
            .filter(|p| p.exists())
            .collect();
        // `QGIS 3.40.2` after `QGIS 3.34.1`: newest first.
        found.sort();
        found.reverse();
        out.extend(found);
    };
    if cfg!(windows) {
        for root in [r"C:\Program Files", r"C:\Program Files (x86)"] {
            glob(Path::new(root), "QGIS", &["bin", "python-qgis-ltr.bat"]);
            glob(Path::new(root), "QGIS", &["bin", "python-qgis.bat"]);
        }
        for root in [r"C:\OSGeo4W", r"C:\OSGeo4W64"] {
            for bat in ["python-qgis-ltr.bat", "python-qgis.bat"] {
                let p = Path::new(root).join("bin").join(bat);
                if p.exists() {
                    out.push(p);
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for root in roots {
            glob(&root, "QGIS", &["Contents", "MacOS", "bin", "python3"]);
        }
    } else {
        for p in ["/usr/bin/python3", "/usr/local/bin/python3", "/opt/conda/bin/python3"] {
            if Path::new(p).exists() {
                out.push(PathBuf::from(p));
            }
        }
    }
    out.dedup();
    out
}

/// A command for the bridge under `python`, without a console window on Windows, with QGIS's
/// prefix set where the Python doesn't set it itself (the macOS app bundle).
fn command(python: &Path, script: &Path, args: &[&str]) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(python);
    c.arg(script).args(args);
    // …/QGIS.app/Contents/MacOS/bin/python3 → …/QGIS.app/Contents/MacOS
    if let Some(prefix) = python.parent().and_then(Path::parent).filter(|p| p.ends_with("Contents/MacOS")) {
        c.env("KHGIS_QGIS_PREFIX", prefix);
    }
    c.env("PYTHONIOENCODING", "utf-8").env("QT_QPA_PLATFORM", "offscreen");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// What the bridge printed, line by line, while it ran.
enum Line {
    Text(String),
    Closed,
}

/// Starts `c`, writes `input` (one line) to it, and reads its output lines as they come,
/// handing each to `on_line`, which returns the answer when the line is one. Stops the process
/// after `limit`, or when `stop` says so. The bridge's error output goes to the log.
fn talk(
    mut c: Command,
    input: Option<String>,
    limit: Option<Duration>,
    stop: &dyn Fn() -> bool,
    on_line: &mut dyn FnMut(&str) -> Option<Result<Value, String>>,
) -> Result<Value, String> {
    let mut child: Child = c
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't start QGIS's Python: {e}"))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        // Written on a thread: a large job mustn't block while the bridge is still starting.
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
            let _ = stdin.write_all(b"\n");
        });
    }
    let (tx, rx) = channel();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(out);
            loop {
                let mut buf = Vec::new();
                match (&mut reader).take(MAX_LINE as u64).read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(Line::Text(String::from_utf8_lossy(&buf).into_owned())).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx.send(Line::Closed);
        });
    }
    if let Some(err) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok).take(10_000) {
                log::info!("QGIS: {line}");
            }
        });
    }
    let start = Instant::now();
    let answer = loop {
        if stop() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(rustgis_analysis::control::CANCELLED.to_string());
        }
        if limit.is_some_and(|l| start.elapsed() > l) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("QGIS didn't answer in time".into());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Line::Text(line)) => {
                if let Some(a) = on_line(line.trim()) {
                    break a;
                }
            }
            Ok(Line::Closed) | Err(RecvTimeoutError::Disconnected) => {
                let status = child.wait().map(|s| s.to_string()).unwrap_or_default();
                break Err(format!("QGIS's Python ended without an answer ({status}); see the log for what it printed"));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    };
    // The answer is the last line: give the process a moment to exit, then stop it.
    let grace = Instant::now();
    while grace.elapsed() < Duration::from_secs(5) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return answer;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// The answer in a bridge line: `{"ok": …}` or `{"error": …}`; `None` for progress, logs and
/// anything else.
fn answer_of(line: &str) -> Option<Result<Value, String>> {
    let v: Value = serde_json::from_str(line).ok()?;
    if let Some(ok) = v.get("ok") {
        return Some(Ok(ok.clone()));
    }
    v.get("error").map(|e| Err(e.as_str().map_or_else(|| e.to_string(), str::to_string)))
}

fn simple(python: &Path, args: &[&str], limit: Duration) -> Result<Value, String> {
    let script = script()?;
    talk(command(python, &script, args), None, Some(limit), &|| false, &mut |l| answer_of(l))
}

/// Checks that `python` runs QGIS: its QGIS and Python versions.
pub fn probe(python: &Path) -> Result<Value, String> {
    let v = simple(python, &["version"], PROBE_LIMIT)?;
    if v.get("bridge").and_then(Value::as_u64) != Some(BRIDGE_VERSION) {
        return Err("the bridge script answered in another version".into());
    }
    Ok(v)
}

/// The Python to use: the chosen one, else the first of [`candidates`] that runs QGIS.
pub fn detect() -> Result<(PathBuf, Value), String> {
    if let Some(p) = chosen() {
        return probe(&p).map(|v| (p.clone(), v)).map_err(|e| format!("{}: {e}", p.display()));
    }
    let mut tried = Vec::new();
    for p in candidates() {
        match probe(&p) {
            Ok(v) => return Ok((p, v)),
            Err(e) => tried.push(format!("{}: {e}", p.display())),
        }
    }
    if tried.is_empty() {
        Err("QGIS wasn't found. Install QGIS (qgis.org), or pick its Python: python-qgis.bat on Windows, QGIS.app/Contents/MacOS/bin/python3 on macOS".into())
    } else {
        Err(format!("no QGIS Python worked:\n{}", tried.join("\n")))
    }
}

/// The Python the cached manifests came from.
fn cached_python() -> Option<PathBuf> {
    let text = std::fs::read_to_string(cache_path().ok()?).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("python").and_then(Value::as_str).map(PathBuf::from)
}

/// The manifests of the QGIS connected last, from the cache.
pub fn cached_manifests() -> Option<Value> {
    let text = std::fs::read_to_string(cache_path().ok()?).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    if v.get("bridge").and_then(Value::as_u64) != Some(BRIDGE_VERSION) {
        return None;
    }
    v.get("manifests").cloned()
}

/// Connects QGIS: finds its Python, lists every algorithm (QGIS's and the plugins'), caches the
/// list and puts the algorithms in the toolbox. Takes a while (every plugin is loaded): run it
/// off the UI thread. Returns [`crate::status`].
pub fn connect() -> Result<Value, String> {
    let (python, version) = detect()?;
    let manifests = simple(&python, &["manifests"], MANIFEST_LIMIT)?;
    let n = manifests.get("algorithms").and_then(Value::as_array).map_or(0, Vec::len);
    if n == 0 {
        return Err("QGIS listed no algorithms: is its Processing plugin installed?".into());
    }
    let cache = json!({"bridge": BRIDGE_VERSION, "python": python.display().to_string(), "version": version, "manifests": manifests});
    let path = cache_path()?;
    std::fs::write(&path, serde_json::to_vec(&cache).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    crate::set_manifests(&serde_json::to_string(&manifests).map_err(|e| e.to_string())?)?;
    let mut status = crate::status();
    if let Some(o) = status.as_object_mut() {
        o.insert("python".into(), json!(python.display().to_string()));
    }
    Ok(status)
}

/// Disconnects QGIS: its algorithms leave the toolbox and the cache is removed.
pub fn disconnect() {
    if let Ok(p) = cache_path() {
        let _ = std::fs::remove_file(p);
    }
    crate::clear_manifests();
}

/// The Python runs use: the chosen one, else the one the tools were listed with.
fn python() -> Result<PathBuf, String> {
    chosen().or_else(cached_python).ok_or_else(|| "QGIS isn't connected: connect it in Plugins › QGIS Plugins & Tools".to_string())
}

/// Installs a QGIS plugin from its .zip into the QGIS user profile, where QGIS and KHGIS both
/// find it. Connect again afterwards to list its algorithms.
pub fn install_zip(zip: &Path) -> Result<Value, String> {
    let python = python()?;
    let zip = zip.to_str().ok_or("the .zip's path isn't valid text")?;
    simple(&python, &["install", zip], PROBE_LIMIT)
}

/// A scratch folder for one run, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, String> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        for _ in 0..16 {
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("khgis-qgis-{}-{nanos:x}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Scratch(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("can't create a scratch folder: {e}")),
            }
        }
        Err("can't create a scratch folder".into())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs a prepared job through the bridge, with progress and Cancel.
pub(crate) fn run(job: &Job) -> Result<ToolResult, String> {
    let python = python()?;
    let script = script()?;
    let scratch = Scratch::new()?;
    for (name, bytes) in &job.files {
        let path = scratch.0.join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    }
    let request = serde_json::to_string(&job.request(&scratch.0.display().to_string())).map_err(|e| e.to_string())?;
    let mut logs = Vec::new();
    let result = talk(command(&python, &script, &["run"]), Some(request), None, &rustgis_analysis::control::stop_requested, &mut |line| {
        let v: Value = serde_json::from_str(line).ok()?;
        if let Some(p) = v.get("progress").and_then(Value::as_f64) {
            rustgis_analysis::control::report_progress(p.clamp(0.0, 100.0) as u64, 100);
            return None;
        }
        if let Some(l) = v.get("log").and_then(Value::as_str) {
            if logs.len() < MAX_LOGS {
                logs.push(l.to_string());
            }
            return None;
        }
        answer_of(line)
    })?;
    let files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&scratch.0)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    // Only the declared outputs: not the inputs, nor the bridge's GeoPackages.
                    if !job.outputs.iter().any(|(f, _)| *f == name) {
                        return None;
                    }
                    std::fs::read(e.path()).ok().map(|b| (name, b))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(finish(job, &files, &result, &logs))
}
