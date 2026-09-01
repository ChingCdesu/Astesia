# GPUI Milestone 3 acceptance

Status: Complete on 2026-09-01 on macOS.

## Delivered

- The native GPUI entry point owns the workspace, query tabs, connection sidebar, status bar,
  overlays, notifications, theme, and language without React or Tauri IPC.
- The command palette preserves editor focus for text input and IME composition, applies workspace
  shortcut precedence, and exposes keyboard selection through GPUI accessibility semantics.
- Connection Profiles support create, edit, test, connect, disconnect, delete, groups, tags, lazy
  database-object loading, MCP state, and Usage Lease restrictions.
- Query tabs reconcile exact target and connection-session invalidation independently, so one
  disconnected connection cannot leave stale tabs or clear valid tabs from another connection.
- Native repository probing fails closed before initialization. Native preferences use documented
  defaults and do not import WebView state.
- Redis object discovery uses cursor-based `SCAN`, deduplicates keys, and completes the cursor
  instead of blocking Redis with `KEYS` or silently truncating results.

## Verification

- `cargo test --locked --manifest-path src-tauri/Cargo.toml -q`: 266 passed, 2 ignored; the MCP
  sidecar test also passed.
- `ASTESIA_ENGINE_SMOKE_CONFIG_PATH=.scratch/engine-smoke.json cargo test --locked
  --manifest-path src-tauri/Cargo.toml
  application::engine_smoke_tests::all_engines_cross_the_application_connection_workflow --
  --ignored --nocapture`: passed against disposable PostgreSQL, Redis, MySQL, MongoDB, ClickHouse,
  SQLite, and SQL Server instances. Every engine crossed configure, test, connect, browse, and
  disconnect through Application Core.
- `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets`: passed with the
  migration branch's existing warnings.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm build`: passed. Vite reported only the existing large-chunk warning.
- Native UI QA covered startup, connection selection, command palette keyboard operation, language,
  sidebar, and light-theme startup. A finish review found no remaining blocker or important issue.

## Existing frontend lint baseline

`pnpm lint` still reports 85 errors and one warning in the legacy React tree. None of the reported
paths are part of the native GPUI implementation. Removing that frontend and its lint debt remains
the later React/Tauri retirement milestone, so it is recorded here rather than mixed into the
Milestone 3 native-shell change.
