// Reads the parent (safemic) crate's version from the workspace root
// Cargo.toml and exposes it as the env var SAFEMIC_VERSION at compile
// time. The sidecar's `#[path = "../../../src/about.rs"]` include of
// about.rs uses SAFEMIC_VERSION via `option_env!()` so the rendered
// "v0.5.1" string in the headless preview matches the live app.
//
// Without this, the sidecar's `env!("CARGO_PKG_VERSION")` would resolve
// to the sidecar's own version (0.0.0) and the preview would render the
// wrong version string. about.rs has a hardcoded "v0.5.1" fallback that
// this script makes redundant.

use std::fs;
use std::path::PathBuf;

fn main() {
    let workspace_cargo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("Cargo.toml"))
        .expect("about-preview must live two levels under the workspace root");

    println!("cargo:rerun-if-changed={}", workspace_cargo.display());

    let contents = match fs::read_to_string(&workspace_cargo) {
        Ok(s) => s,
        Err(_) => {
            // Parent Cargo.toml unreadable; leave SAFEMIC_VERSION unset so
            // about.rs falls back to its hardcoded literal.
            return;
        }
    };

    // Naive single-line `version = "X.Y.Z"` extraction from [package] section.
    // The workspace root Cargo.toml is the safemic crate manifest.
    let mut in_package = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
        }
        if in_package && trimmed.starts_with("version") {
            // version = "0.5.1"  →  0.5.1
            if let Some(eq) = trimmed.find('=') {
                let value = trimmed[eq + 1..].trim().trim_matches('"');
                println!("cargo:rustc-env=SAFEMIC_VERSION={value}");
                return;
            }
        }
    }
}
