fn main() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let target = std::env::var("TARGET").expect("TARGET is set by Cargo");
    let extension = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let sidecar = manifest_dir
        .join("binaries")
        .join(format!("astesia-mcp-{target}{extension}"));

    // Direct Cargo commands should not require a prebuilt sidecar. Tauri's
    // beforeDev/beforeBuild hooks stage it before an actual app bundle.
    if !sidecar.is_file() && std::env::var_os("TAURI_CONFIG").is_none() {
        std::env::set_var("TAURI_CONFIG", r#"{"bundle":{"externalBin":[]}}"#);
        println!(
            "cargo:warning=MCP sidecar is not staged; skipping externalBin validation for this Cargo build"
        );
    }

    tauri_build::build()
}
