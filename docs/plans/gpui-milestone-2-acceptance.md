# GPUI Milestone 2 acceptance

Status: Complete on 2026-09-01.

## Delivered

- `Application` is the composition root for Connection Profiles and Database Sessions, catalog,
  query, mutation, export, performance, transfer, task, event, and optional MCP services. It owns no
  GPUI or Tauri type.
- Connection Profile persistence, native-state probing, system credential binding, repository
  revisions, Database Session generations, Usage Leases, and MCP snapshots remain shared contracts
  instead of UI-owned state.
- Each Database Session has a separately locked `DriverHandle`. Retiring a handle rejects queued
  work, reconnecting cannot make an in-flight operation switch drivers, and unrelated sessions do
  not share one I/O lock.
- Profile and session workflows are implemented by `ConnectionService`; catalog, query, mutation,
  export, performance, and transfer behavior lives in dedicated application services rather than
  desktop command handlers.
- `UiEventSink` and `UiEventBus` carry typed task and MCP events without a window handle. The task
  model owns progress, cancellation, panic conversion, terminal state, and exactly-once completion
  events.
- `SidecarHost` and `SidecarControl` isolate process discovery, spawn, observation, and termination.
  Failed termination retains retryable control, observation-channel closure terminates an
  unobserved process, and shutdown blocks new work.
- `EngineCapabilities` defines catalog shape, schema/database management, row mutation, read-only
  browsing, backup/restore, same-engine table copy, Explain, and performance support for all seven
  engines. Unsupported MongoDB and ClickHouse actions can be omitted before reaching a driver.

## Boundary proof

| Requirement | Evidence |
| --- | --- |
| Remove `tauri::AppHandle` from core state | `AppState`, `AppHandle`, and Tauri types are absent from the Rust source boundary; `Application` composes plain Rust services. |
| Move business logic out of command handlers | `src-tauri/src/commands/` and `src-tauri/src/state.rs` are absent; the application service modules own the workflows. |
| Preserve repository, revision, session, Usage Lease, task, and MCP contracts | The repository and connection-runtime types feed `ConnectionService`, `TaskManager`, and `McpSyncRegistry`; focused regression suites cover stale revisions, session generations, ownership, task outcomes, and sidecar lifecycle. |
| Replace Tauri event emission | `UiEventSink` is an object-safe boundary and `UiEventBus` provides subscriptions through Tokio broadcast channels. |
| Replace Tauri sidecar/shell calls | `SidecarHost` returns a process control and typed event stream; the MCP runtime depends only on that interface. |
| Gate engine-specific workflows | `DbType::capabilities()` returns one explicit policy for each of the seven engines and mutation services enforce it. |
| Make the GPUI entry independent of commands | `lib.rs` exports `ui::run`; `main.rs` calls `app_lib::run()` with no Tauri builder, command registration, or IPC state. |

No Rust dependency named `tauri` remains. The only remaining `tauri` source text is the schema URL
inside `src-tauri/tauri.conf.json`; the legacy frontend, configuration, capabilities, package hooks,
and directory name remain Milestone 8 cleanup and are not used by the native entry point.

## Milestone 0 coverage

This milestone establishes the UI-independent owning boundary for checklist rows C01-C06,
Q03-Q05, D01-D11, E02-E04, and O01-O08 in
`gpui-milestone-0-acceptance.md`. It does not by itself claim that their remaining GPUI views are
complete; user-visible parity stays with each row's destination milestone.

## Verification

- Boundary guards confirmed that `src-tauri/src/commands/` and `src-tauri/src/state.rs` are absent;
  no Tauri/AppState/command-registration reference exists in the Rust package; and Application
  Core, database, task, MCP, and connection-runtime modules do not depend on GPUI or `ui`.
- Focused locked test filters passed for `application::`, `connection_runtime::`, `tasks::tests`,
  `mcp_runtime::tests`, and `db::engine::tests`.
- `RUSTUP_TOOLCHAIN=1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml -q`: 267 tests
  passed, two environment-dependent smoke tests were ignored, and the MCP binary test passed.
- `RUSTUP_TOOLCHAIN=1.97.1 cargo check --locked --manifest-path src-tauri/Cargo.toml`: passed.
- `RUSTUP_TOOLCHAIN=1.97.1 cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm build`: passed with the existing large-chunk warning.
- `pnpm lint`: unchanged legacy React baseline of 85 errors and one warning.
- `git diff --check`: passed.

The Application Core compiles and tests without Tauri types, and the GPUI entry point does not need
the former Tauri command modules. Milestone 2's exit condition is satisfied.
