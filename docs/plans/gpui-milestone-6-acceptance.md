# GPUI Milestone 6 acceptance

Status: Complete on 2026-09-03 on macOS.

## Delivered

- MongoDB collections open as native, session-bound document tabs with strict object filters,
  paging, refresh, stale-load rejection, retained data during refresh, collapsible documents,
  typed JSON fields, and distinct empty, unavailable, and error states.
- Redis uses cursor-based `SCAN` with de-duplication and native key tabs for type, TTL, and value.
  Typed mutations cover strings, hashes, lists, sets, and sorted sets; deletion requires explicit
  confirmation and refreshes both the selected viewer and the catalog tree.
- Redis query tabs parse quoted and escaped raw command arguments without passing through the SQL
  parser or enabling SQL completion.
- CSV, JSON, and XLSX exports run as observable tasks for the current page, rectangular selection,
  or all rows matching the current filter and sort, while preserving the selected columns.
- Backup, restore, and same-engine table copy use generation-checked Database Sessions. Native
  prompts expose selected/all-table scope, structure/data/both content, drop behavior, file paths,
  explicit copy targets and names, plus both drag/drop and copy/paste entry points.
- The native Task Center retains every task and displays pending, running, cancelling, completed,
  partial, failed, and cancelled states. Progress is monotonic, cancellation remains non-terminal
  until worker acknowledgement, and one terminal event produces one notification.
- A process-backed `SidecarHost` discovers the target-named MCP binary, passes secrets only through
  environment variables, pipes output, monitors exit, and terminates the child on stop or owner
  drop. The native MCP tab provides status, start, stop, restart, retry, and explicit configuration
  copy with a fresh loopback port and token for every start.
- Existing MCP Usage Lease, shared-revision, SQL classification, and elicitation approval contracts
  remain the authority for profile mutation and destructive operations.

## Workflow evidence

| ID | Acceptance evidence |
| --- | --- |
| Q08 | `QueryWorkspaceState` routes a Redis target to one typed `RedisCommand`; quoted and escaped arguments are covered by parser tests and a live quoted `GET` succeeds without SQL parsing. |
| E01 | `DocumentSession` tests cover strict filters, paging, retained refresh data, stale results, and unavailable sessions. The MongoDB smoke seeds nested typed documents, filters them, and loads both pages. |
| E02 | The Redis driver traverses `SCAN MATCH ... COUNT`, removes duplicates, sorts results, and loads type, TTL, and value with a stable `Missing` variant. The live smoke discovers and inspects all five supported value types. |
| E03 | Typed mutations map to the appropriate string, hash, list, set, and sorted-set commands. The live smoke creates and reads each type through `RedisService`. |
| E04 | Native deletion is behind a destructive prompt. Success reloads the key as `Missing`, emits `RedisKeyDeleted`, and invalidates the owning catalog section; cancellation has no mutation path. |
| O01 | The grid export workflow offers CSV, JSON, and XLSX plus current/selection or all-matching scope. The seven-engine smoke writes all three formats as tasks, verifies files, and checks the exact two-row completion count. |
| O02 | Backup prompts choose selected/all tables, structure/data/both, three drop modes, and a destination. Transfer tests cover cancellation and partial output; the SQL5 smoke backs up one table with structure and data and verifies durable output. |
| O03 | Restore binds the chosen file to one validated target session. Statement effects determine complete, partial, failed, or cancelled outcomes; the SQL5 smoke drops and restores the source table and verifies both rows. |
| O04 | The sidebar exposes drag/drop and copy/paste through one explicit source/target form with same-engine validation, destination profile/database/name, and structure/data/both options. The SQL5 smoke copies structure and data and verifies terminal completion. |
| O05 | Task tests cover pending through every terminal status, cooperative cancellation, worker panic conversion, and durable inspection. The global Task Center reads the same retained task records. |
| O06 | `TaskManager` clamps progress to the prior maximum and emits a single `TaskCompleted` event only on the first terminal transition. Workspace notifications subscribe to that terminal event rather than polling. |
| O07 | The process sidecar tests prove target-aware discovery and secret-free arguments. The live lifecycle test starts, restarts with a different port and token, and stops the staged MCP binary without a Tauri process API. |
| O08 | The normal suite covers coexisting Usage Leases, cross-repository mutation blocking, generation/ABA protection, abandoned-lease cleanup, revision rechecks, permission SQL rejection, and session-scoped update/destructive elicitation. |

## Verification

- `rustup run 1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml --quiet`:
  339 tests passed, five environment-dependent tests were ignored, and the MCP binary test passed.
- `ASTESIA_ENGINE_SMOKE_CONFIG_PATH="$PWD/.scratch/engine-smoke.json" rustup run 1.97.1 cargo
  test --locked --manifest-path src-tauri/Cargo.toml
  application::engine_smoke_tests::milestone_six::milestone_six_data_transfer_task_and_export_workflows
  -- --ignored --nocapture`: passed against disposable MySQL 8.4, PostgreSQL 17, SQLite,
  Azure SQL Edge, MongoDB 8.2, Redis 7, and ClickHouse 25.8 instances.
- The live SQL5 workflow creates and seeds a table, backs it up, copies it, drops it, restores it,
  verifies restored rows, and cleans up. The same run verifies MongoDB filtering/paging, every Redis
  value type and deletion state, and task-backed CSV/JSON/XLSX output for all seven targets.
- `pnpm mcp:prepare:debug` staged the target-named debug sidecar, then
  `milestone_six_native_mcp_sidecar_lifecycle` passed start, restart, endpoint change, token change,
  stop, and child-process termination.
- Live acceptance exposed and corrected MySQL auto-increment restoration and SQL Server DDL
  reconstruction for length, precision, scale, temporal precision, identity seed/increment,
  nullability, defaults, and primary keys before the combined seven-engine run passed.
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

## Code-quality closure

The final strict review moved export orchestration, engine-specific sidebar actions, long-running
workspace operations, and Milestone 6 engine acceptance into focused modules. No modified source
file crossed the 1,000-line boundary, and the Application Core remains the sole owner of session,
task, export, transfer, MongoDB, Redis, and sidecar behavior.

## Closure boundary

Q08, E01-E04, and O01-O08 satisfy the Milestone 6 exit condition on macOS without Tauri process or
event APIs. The normal suite keeps five environment-dependent tests ignored because they require
disposable services, a local untracked configuration, or a staged sidecar; the two Milestone 6
tests were run explicitly. Visualization, ER diagrams, performance dashboards, Legacy Shell
removal, cross-platform validation, and packaged-application cutover remain Milestones 7-8 scope.
