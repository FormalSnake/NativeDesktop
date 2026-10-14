use std::collections::BTreeMap;
use std::path::Path;

use nd_docs::captures::{Manifest, Os};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Links the app's Swift into one archive on macOS; a no-op on every other target.
    nd::build::app();

    let repo = nd_docs::find_repo(Path::new(env!("CARGO_MANIFEST_DIR"))).ok_or("docs/app is outside the repository")?;
    // Example sources feed the capture hashes, so any change there can turn a capture stale.
    for path in
        ["docs/content", "docs/examples.toml", "docs/app/assets/captures/manifest.toml", "examples", "Cargo.toml"]
    {
        println!("cargo:rerun-if-changed={}", repo.join(path).display());
    }

    let site = nd_docs::load(&repo).map_err(|e| format!("loading the docs: {e:#}"))?;
    let manifest = Manifest::load(&repo).map_err(|e| format!("loading the capture manifest: {e:#}"))?;
    let captures: BTreeMap<&str, BTreeMap<Os, _>> = site
        .examples
        .iter()
        .map(|e| (e.name.as_str(), Os::ALL.into_iter().map(|os| (os, manifest.state(e, os))).collect()))
        .collect();
    let json = serde_json::json!({ "site": site, "captures": captures });
    std::fs::write(Path::new(&std::env::var("OUT_DIR")?).join("content.json"), json.to_string())?;
    Ok(())
}
