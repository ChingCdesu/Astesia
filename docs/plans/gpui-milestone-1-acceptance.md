# GPUI Milestone 1 acceptance

Status: Complete on 2026-09-01 on macOS.

## Delivered

- Rust 1.97.1 is fixed in `rust-toolchain.toml`, the Rust package declaration, and the release
  workflow. All Zed and GPUI crates use commit
  `399258feeaf90ad8a3a208c99221ee87b6452f38`.
- The application starts with GPUI, initializes the GPUI-compatible Tokio runtime, and creates a
  standalone local Zed `Editor` without initializing Zed `Client`, `Project`, or `Workspace`
  services.
- Zed's data root is redirected to Astesia's `zed-runtime` directory before editor initialization.
  GPUI uses `BlockedHttpClient`, and the native process opened no network sockets or user Zed data
  files during startup QA.
- The native window starts at 1280×800 with a 960×600 minimum size. The editor supports focus,
  typing, selection, marked-text composition, undo/redo, and live window resizing.
- GPUI uses last-window quit mode. On macOS, Astesia installs the corresponding AppKit delegate
  policy because the pinned GPUI delegate does not expose it; closing the final native window now
  exits the process cleanly.

## Verification

- `RUSTUP_TOOLCHAIN=1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml -q`: 267
  passed and 2 ignored; the MCP sidecar test also passed. The library suite includes a regression
  that drives the editor's macOS marked-text contract through `ni`, `nihao`, and committed `你好`,
  then verifies grouped undo and redo.
- `RUSTUP_TOOLCHAIN=1.97.1 cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm build`: passed. Vite reported only the existing large-chunk warning.
- Native UI QA covered focus, typing, selection, undo, redo, and window zoom/resize. A final-window
  close removed the exact process PID, including while native repository probing was active.
- `lsof` reported no network socket and no file under the user's Zed profile for the running native
  process.

## Existing frontend lint baseline

`pnpm lint` still reports 85 errors and one warning in the legacy React tree. None of the reported
paths are part of the GPUI runtime or this milestone's platform lifecycle changes. That frontend is
removed in the later React/Tauri retirement milestone.
