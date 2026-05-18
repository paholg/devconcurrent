use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("schema-gen lives at <workspace>/crates/schema-gen");

    let devconcurrent_src = workspace_root.join("crates/devconcurrent/src");
    println!("cargo:rerun-if-changed={}", devconcurrent_src.display());
    println!("cargo:rerun-if-changed=build.rs");

    let schema = devconcurrent::schema();
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes");

    let out_path = workspace_root.join("schemas/devconcurrent.schema.json");
    std::fs::write(&out_path, format!("{json}\n"))
        .unwrap_or_else(|e| panic!("write {}: {e}", out_path.display()));
}
