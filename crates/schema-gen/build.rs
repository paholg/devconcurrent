use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("schema-gen lives at <workspace>/crates/schema-gen");

    let devconcurrent_src = workspace_root.join("crates/devconcurrent/src");
    let cargo_lock = workspace_root.join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", devconcurrent_src.display());
    println!("cargo:rerun-if-changed={}", cargo_lock.display());
    println!("cargo:rerun-if-changed=build.rs");

    let toml_spec_version = toml_spec_version(&cargo_lock);

    let mut schema = devconcurrent::schema();
    schema.ensure_object().insert(
        "x-tombi-toml-version".into(),
        format!("v{toml_spec_version}").into(),
    );
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes");

    let out_path = workspace_root.join("schemas/devconcurrent.schema.json");
    std::fs::write(&out_path, format!("{json}\n"))
        .unwrap_or_else(|e| panic!("write {}: {e}", out_path.display()));
}

/// Extract the TOML spec version from the `toml` crate's build metadata in
/// Cargo.lock. The crate is versioned as e.g. `1.1.2+spec-1.1.0`, where the
/// part after `+spec-` is the TOML specification version it conforms to.
fn toml_spec_version(cargo_lock: &Path) -> String {
    let lock = std::fs::read_to_string(cargo_lock)
        .unwrap_or_else(|e| panic!("read {}: {e}", cargo_lock.display()));
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line != r#"name = "toml""# {
            continue;
        }
        let Some(version_line) = lines.next() else {
            continue;
        };
        let version = version_line
            .strip_prefix(r#"version = ""#)
            .and_then(|s| s.strip_suffix('"'));
        let Some(version) = version else { continue };
        if let Some((_, spec)) = version.split_once("+spec-") {
            return spec.to_string();
        }
    }
    panic!("no `toml` package with `+spec-` build metadata in Cargo.lock");
}
