# Astesia

Astesia is a native desktop database workspace built in Rust with GPUI and the embedded Zed
Editor. It connects to MySQL, PostgreSQL, SQLite, SQL Server, ClickHouse, MongoDB, and Redis.

The native-runtime migration has completed Milestones 0-8. The Legacy Shell and Tauri/WebView build
chain are gone, and the internal native package matrix is in place. See the
[Milestone 8 acceptance](docs/plans/gpui-milestone-8-acceptance.md) for platform evidence and the
[GPUI rebuild plan](docs/plans/gpui-ui-rebuild.md) for the complete behavioral checklist.

## Delivered native capabilities

- Native workspace, connection profiles, lazy connection lifecycle, notifications, command
  palette, shortcuts, localization, and light/dark/system appearance
- SQL query tabs for MySQL, PostgreSQL, SQLite, SQL Server, and ClickHouse using the embedded Zed
  Editor, local SQL highlighting and completion, multi-statement execution, Explain, result
  selection, and TSV copy
- Capability-gated catalog browsing for all seven engines
- SQL table structure, indexes, constraints, foreign keys, and qualified object definitions
- Supported database-object creation, rename, and destructive-confirmed deletion
- Paged relational data grids with filtering, typed sorting, selection, copying, CSV/TSV paste,
  typed and long-value editing, staged insert/update/delete, undo, discard, and deterministic saves
- Read-only ClickHouse grids with filtering, sorting, paging, selection, copy, and CSV export
- MongoDB document browsing, Redis key workflows, task-backed export and transfer operations,
  Task Center inspection, and native MCP Sidecar lifecycle
- Native table/query charts, qualified ER diagrams, and seven-engine performance dashboards

## Tech stack

| Layer | Current technology |
| --- | --- |
| Desktop UI | GPUI and Zed UI |
| Editor | Embedded Zed Editor with bundled Tree-sitter SQL |
| Application Core | Rust and Tokio |
| Database drivers | SQLx, Tiberius, MongoDB, Redis, and ClickHouse HTTP |
| Local state and credentials | SQLite repository and platform credential vault |

## Native development

The repository pins Rust 1.97.1 in `rust-toolchain.toml`.

```bash
cargo run --locked --bin astesia
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets
cargo fmt -- --check
```

Environment-dependent seven-engine tests are ignored by the normal suite. Their disposable-service
configuration and latest results are recorded in the milestone acceptance documents.

## MCP server

The standalone authenticated MCP server remains in `src/bin/astesia-mcp.rs`. The native
desktop owns its lifecycle through the MCP workspace tab. Build it directly for local clients:

```bash
cargo build --locked --bin astesia-mcp
```

See the [MCP server guide](docs/mcp.md) for stdio configuration, credential handling, available
tools, and destructive-operation safeguards.

## Project structure

```text
Cargo.toml              # Root Rust package manifest
src/
  application/            # UI-independent services and workflow state
  connection_runtime/     # Live connection and session ownership
  connection_repository/  # Durable profiles, revisions, and migration
  credential_vault/       # Platform-backed credential storage
  db/                     # Seven database-driver implementations
  platform/               # Native lifecycle, preferences, events, and sidecars
  ui/                     # GPUI shell, workspace, tabs, forms, catalogs, and grids
  mcp/                    # Standalone MCP tools and policy
  mcp_runtime/            # Native MCP lifecycle
  tasks/                  # Background task model
  bin/astesia-mcp.rs      # Standalone MCP entry point
  main.rs                 # Native desktop entry point

icons/                      # Desktop application icons
packaging/                  # Internal package metadata
scripts/                    # Native macOS, Linux, and Windows packaging entry points
```
