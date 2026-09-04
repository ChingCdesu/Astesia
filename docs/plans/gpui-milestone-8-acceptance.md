# GPUI Milestone 8 acceptance

Status: Complete on 2026-09-04.

## Delivered

- The React, Zustand, Radix, Monaco, Vite, and Tauri desktop shell has been removed. The repository
  no longer tracks frontend package metadata, WebView assets, Tauri capabilities or configuration,
  Tauri sidecar staging, or the former plugin scaffold.
- GPUI owns open/save prompts and clipboard access. Rust owns filesystem operations, atomic JSON
  preferences, process-sidecar discovery, the displayed application version, and in-place restart.
  Cancelling a path prompt remains distinct from a platform error.
- Native packaging scripts build `astesia` and `astesia-mcp` together with Rust 1.97.1. The macOS
  application bundle, Linux archive, and Windows archive layout all place the sidecar beside the
  desktop executable.
- The release workflow now builds macOS arm64, macOS x64, Linux x64, and Windows x64 packages with
  Cargo only. The version workflow updates `Cargo.toml` and `Cargo.lock` without Node, Vite, or
  Tauri tooling.
- The status bar reports the Cargo package version. The command palette exposes native restart and
  protects unsaved query tabs with a confirmation prompt.

## Workflow evidence

| ID | Acceptance evidence |
| --- | --- |
| P01 | The packaged Linux x64 application rendered through GPUI's X11/OpenGL path in an amd64 Debian environment. Chinese text and the embedded editor rendered correctly; `Ctrl+Shift+P` opened the command palette; `Ctrl+O` opened the XDG portal file chooser; and copying the editor returned `SELECT 1;` from the X11 clipboard. Preference tests cover defaults, atomic round-trip persistence, and corrupt-file preservation. |
| P02 | The packaged macOS application displays `Astesia v1.0.9`. `native_runtime_can_request_an_in_place_restart` verifies GPUI's restart request, while workspace behavior prompts before restarting with dirty tabs. Public updater, notarization, and external distribution remain outside the internal rebuild. |
| P03 | Both macOS application architectures were built and launched from signed bundles with a version-matched adjacent MCP binary. Linux produced and launched an x86_64 ELF package with all shared libraries resolved after installing its runtime packages. Windows produced x86_64 MSVC application and MCP PE files and linked the complete library test executable. Static scans find no tracked Legacy Shell files and no Tauri or WebView dependency below the Cargo root package. |

## Verification

- `rustup run 1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml --quiet`:
  359 tests passed, six environment-dependent tests were ignored, and the MCP binary test passed.
- `rustup run 1.97.1 cargo check --locked --manifest-path src-tauri/Cargo.toml`: passed.
- `rustup run 1.97.1 cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets`:
  passed with the migration branch's existing warnings.
- `rustup run 1.97.1 cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `scripts/package-macos.sh aarch64-apple-darwin` and
  `scripts/package-macos.sh x86_64-apple-darwin`: passed. Both bundles passed strict ad-hoc
  signature validation, contained the requested Mach-O architecture, reported version 1.0.9, and
  launched successfully. Both sidecars reported `astesia-mcp 1.0.9`.
- `scripts/package-linux.sh x86_64-unknown-linux-gnu`: passed in a Linux x86_64 container with Rust
  1.97.1. The package contained two x86_64 ELF executables, its sidecar reported version 1.0.9,
  and the rendered X11 smoke covered fonts, graphics, keyboard input, the command palette, a native
  portal file dialog, and clipboard output.
- `cargo-xwin test --locked --manifest-path src-tauri/Cargo.toml
  --target x86_64-pc-windows-msvc --no-run --lib`: passed with Rust 1.97.1 and linked the full
  library test executable. Separate debug builds produced x86_64 MSVC `astesia.exe` and
  `astesia-mcp.exe`; inspection confirmed the GPUI DirectX, text-input, and accessibility imports.
- The release workflow and version workflow parse as YAML; the Cargo-only version replacement was
  exercised on temporary copies and updated both version files to the same value.
- `git diff --check`: passed.

## Windows native-runner boundary

The Windows release build remains defined on `windows-latest`, where GPUI can use the Windows SDK's
`fxc.exe` for its optimized DirectX shaders. A macOS cross host can compile and link GPUI's Windows
debug path, including all library tests, but cannot execute those PE files or run that native shader
compiler. GitHub reports Actions enabled for this fork but currently exposes no registered workflow,
so a native hosted run could not be dispatched during this acceptance. The compiled Windows target
is retained as a release gate; Windows runtime behavior was not represented as locally exercised.

## Closure boundary

P01-P03 close the internal rebuild: the locked desktop runtime is Cargo-only, the Legacy Shell is
available only through Git history, native platform services replace the former Tauri boundaries,
and the complete parity checklist passes on the internally exercised macOS and Linux platforms.
Windows x64 remains in the package matrix with a successful compile-and-link gate; a native Windows
run is required before distributing that artifact outside the internal rebuild.
