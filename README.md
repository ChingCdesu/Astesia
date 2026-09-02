# Astesia

Astesia is a native desktop database workspace built in Rust with GPUI and the embedded Zed
Editor. It connects to MySQL, PostgreSQL, SQLite, SQL Server, ClickHouse, MongoDB, and Redis.

The native-runtime migration has completed Milestones 0-5 on macOS. See the
[GPUI rebuild plan](docs/plans/gpui-ui-rebuild.md) and
[Milestone 5 acceptance](docs/plans/gpui-milestone-5-acceptance.md) for the behavioral checklist
and current evidence.

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

## Remaining parity work

Milestones 6-8 cover MongoDB document and Redis key editing; long-running export, backup, restore,
and table-copy tasks; native MCP Sidecar lifecycle; charts, ER diagrams, and performance dashboards;
cross-platform validation; packaging; and final removal of the Legacy Shell. These capabilities may
still exist in the retained React/Tauri source, but they are not yet accepted in the native runtime.

## Tech stack

| Layer | Current technology |
| --- | --- |
| Desktop UI | GPUI and Zed UI |
| Editor | Embedded Zed Editor with bundled Tree-sitter SQL |
| Application Core | Rust and Tokio |
| Database drivers | SQLx, Tiberius, MongoDB, Redis, and ClickHouse HTTP |
| Local state and credentials | SQLite repository and platform credential vault |
| Legacy Shell, pending Milestone 8 removal | React 19, TypeScript, Tauri 2, Monaco, Vite, Tailwind, Radix UI, and Zustand |

## Native development

The repository pins Rust 1.97.1 in `rust-toolchain.toml`.

```bash
cargo run --locked --manifest-path src-tauri/Cargo.toml --bin astesia
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Environment-dependent seven-engine tests are ignored by the normal suite. Their disposable-service
configuration and latest results are recorded in the milestone acceptance documents.

## Legacy Shell migration commands

Node.js and pnpm are needed for the retained frontend, Tauri packaging paths, and MCP sidecar
staging helpers while the migration is in progress.

```bash
pnpm install
pnpm dev              # Legacy frontend only
pnpm tauri:dev        # Legacy Tauri CLI wrapper
pnpm build            # Legacy TypeScript and Vite build gate
pnpm lint             # Legacy frontend lint baseline
pnpm tauri:build      # Legacy packaging path
```

## MCP server

The standalone authenticated MCP server remains in `src-tauri/src/bin/astesia-mcp.rs`. The native
desktop lifecycle and status-bar integration are Milestone 6 work. Existing build helpers remain
available during migration:

```bash
pnpm mcp:prepare:debug
pnpm mcp:build
```

See the [MCP server guide](docs/mcp.md) for stdio configuration, credential handling, available
tools, and destructive-operation safeguards.

## Project structure

```text
src-tauri/                  # Native Rust application; directory name is retained until cutover
  src/
    application/            # UI-independent services and workflow state
    connection_runtime/     # Live connection and session ownership
    connection_repository/  # Durable profiles, revisions, and migration
    credential_vault/       # Platform-backed credential storage
    db/                     # Seven database-driver implementations
    platform/               # Native lifecycle, preferences, events, and sidecars
    ui/                     # GPUI shell, workspace, tabs, forms, catalogs, and grids
    mcp/                    # Standalone MCP tools and policy
    mcp_runtime/            # Native MCP lifecycle under migration
    tasks/                  # Background task model
    bin/astesia-mcp.rs      # Standalone MCP entry point
    main.rs                 # Native desktop entry point

src/                        # Temporary React/Tauri Legacy Shell; removed in Milestone 8
public/                     # Legacy frontend assets
```
