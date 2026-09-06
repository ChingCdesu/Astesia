# GPUI Milestone 5 acceptance

Status: Complete on 2026-09-02 on macOS.

## Delivered

- The native catalog lazily exposes only engine-supported databases, schemas, tables,
  collections, keys, views, functions, procedures, triggers, and users. Catalog loads preserve
  prior data on failure and reject results from stale Database Sessions.
- Native structure tabs show authoritative columns, indexes, constraints, and foreign keys.
  Qualified view, function, and procedure definitions retain their catalog identity and report
  unavailable or stale definitions explicitly.
- A typed `ObjectService` owns create, rename, and drop SQL for supported database objects. Forms
  validate before execution, destructive confirmations identify the complete target, and the UI
  refreshes only after success. Mutation notices never expose generated DDL or credentials.
- `GridSession` and `GridService` own filtering, typed sorting, pagination, row counts, selection,
  staged inserts, updates and deletes, undo, discard, and deterministic transactional batch saves.
  Any failed statement rolls back the whole batch and leaves the staged changes editable.
- The native data grid supports typed cell editors, catalog-backed enum values, SQL NULL and
  database defaults, long text and JSON editing, rectangular CSV/TSV paste, deterministic copy,
  column resizing, dirty-state navigation guards, and visible loading, empty, error, and save
  states.
- ClickHouse uses the same browse, filter, sort, page, selection, copy, and CSV export stack while
  both the model and UI keep every mutation path disabled.

## Workflow evidence

| ID | Acceptance evidence |
| --- | --- |
| D01 | Capability-gated catalog snapshots and native sections distinguish ready, empty, loading, error, and unsupported states. The seven-engine smoke checks each section against the engine capability matrix. |
| D02 | Structure services and tabs load columns, indexes, constraints, and foreign keys by capability. The SQL5 smoke loads every supported section from a created table. |
| D03 | Definition items preserve qualified identities, session generations, read-only SQL, and explicit missing/error states. The live smoke creates, catalogs, and reads supported view definitions. |
| D04 | Typed creation specifications render engine-specific DDL for databases, schemas, tables, views, functions, procedures, triggers, and users. Validation and execution failures stay in the form; success refreshes the owning catalog. |
| D05 | Engine quoting and qualified drop targets are covered by renderer tests and live create/rename/drop cycles. Confirmations include MySQL user hosts and PostgreSQL trigger parents and disclose PostgreSQL schema cascade scope. |
| D06 | Grid load plans own count, WHERE filtering, typed ORDER BY, pagination, and refresh invalidation. The SQL5 smoke verifies filtered paging and typed sorting. |
| D07 | Model tests cover primary-key editability, staged updates/inserts/deletes, undo, and discard. The four writable engines save all three mutation kinds in the live smoke. |
| D08 | Save plans preserve original row identities and deterministic statement order. Unit tests verify that failed saves roll back every statement and remain retryable; the live smoke verifies committed rows through a fresh load. |
| D09 | Typed editors cover boolean, numeric, date/time, enum, JSON, text, SQL NULL, and long values without silently changing type intent. |
| D10 | Grid actions cover row and rectangular-cell selection, bounded CSV/TSV paste, stable copy order, one-step paste undo, column resize, focus ownership, and a reachable Save action at the minimum layout width. |
| D11 | The ClickHouse smoke filters, sorts, pages, selects, copies, and writes a CSV while asserting `ReadOnlyEngine` for editability and staged insertion. Native edit controls are absent. |

## Verification

- `rustup run 1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml --quiet`:
  329 tests passed, three environment-dependent tests were ignored, and the MCP sidecar test
  passed.
- `ASTESIA_ENGINE_SMOKE_CONFIG_PATH="$PWD/.scratch/engine-smoke.json" rustup run 1.97.1 cargo
  test --locked --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture`: all three ignored
  tests passed against disposable MySQL 8.4, PostgreSQL 17, SQLite, SQL Server, MongoDB 8.2,
  Redis 7, and ClickHouse 25.8 instances.
- The Milestone 5 smoke verifies the capability-gated catalog on all seven engines. On SQL5 it
  creates and renames a table; loads structure; filters, sorts, pages, selects, copies, and exports
  rows; saves supported mutations; forces a mid-batch primary-key failure and verifies complete
  rollback on MySQL, PostgreSQL, SQLite, and SQL Server; checks ClickHouse's read-only boundary;
  exercises supported object creation, discovery, definition loading, and deletion; then verifies
  catalog removal.
- Live testing exposed binary catalog metadata and prepared-protocol DDL incompatibilities in
  MySQL 8.4, plus SQL Server batch requirements and principal/trigger semantic mismatches. The
  corrected driver paths passed targeted engine loops before the combined run.
- The debug native application launched as an isolated macOS bundle and rendered the complete
  loaded GPUI workspace. Source and finish reviews confirmed compact controls, internal grid
  scrolling, visible focus and operation states, and minimum-width action reachability.
- `rustup run 1.97.1 cargo check --locked --manifest-path src-tauri/Cargo.toml`: passed.
- `rustup run 1.97.1 cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets`:
  passed with the migration branch's existing warnings.
- `rustup run 1.97.1 cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm build`: passed. Vite reported only the existing large-chunk warning.
- `git diff --check`: passed.

## Existing frontend lint baseline

`pnpm lint` still reports 85 errors and one warning in the Legacy Shell. None of the reported paths
are part of the native GPUI implementation. Removing the Legacy Shell and its lint debt remains the
later React/Tauri retirement milestone.

## Closure boundary

The native catalog, object-management, and relational-grid slices satisfy D01-D11 and the
Milestone 5 exit condition on macOS. The normal suite keeps three seven-engine tests ignored
because they require disposable services and a local, untracked configuration; all three were run
explicitly for this acceptance. MongoDB and Redis data editing, long-running exports and transfers,
the MCP Sidecar, visualization, diagnostics, cross-platform validation, and packaged-application
cutover remain Milestones 6-8 scope.
