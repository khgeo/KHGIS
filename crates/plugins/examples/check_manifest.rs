//! Checks plugin.json files the way KHGIS does when it installs a plugin.
//! cargo run -p rustgis-plugins --example check_manifest -- path/to/plugin.json …
fn main() {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).expect("readable");
        match rustgis_plugins::manifest::Manifest::parse(&text).and_then(|m| m.validate().map(|_| m)) {
            Ok(m) => println!("ok   {path}: {} {} ({} commands, {} tools, i18n {:?})", m.id, m.version, m.commands.len(), m.tools.len(), m.i18n.keys().collect::<Vec<_>>()),
            Err(e) => {
                failed = true;
                println!("FAIL {path}: {e}");
            }
        }
    }
    std::process::exit(failed as i32);
}
