# GPUI Milestone 0 acceptance

Status: Complete on 2026-09-01.

## Reference baseline

The Legacy Shell behavioral reference is commit
`5004eb9b7d1e78bbdc02fae2d5a30169e7158725` (`feat: add ClickHouse database support`). It is the
last React/Tauri commit and the direct parent of the Application Core migration. Later GPUI work
must compare behavior with files at that commit, not with the React tree still present on the
migration branch.

The inventory was derived from the reference commit's `README.md`, `src/components/`,
`src/stores/`, `src/lib/commands/definitions.ts`, `src/i18n/`, and the Tauri command registration in
`src-tauri/src/lib.rs`. The baseline includes user-visible success paths, failure paths, keyboard
behavior, and capability gating. Incidental exposure of an unsupported operation is a Legacy Shell
bug, not a parity requirement.

The historical locked Rust suite passes on the repository toolchain as 148 library tests plus one
MCP binary test, which is the plan's 149-test baseline. The current branch passes 267 library tests
with two ignored environment-dependent smoke tests, plus the MCP binary test. No later milestone
may reduce the historical 149-test floor without replacing the removed coverage at the owning
Application Core boundary.

## Engine baseline

The checklist uses `SQL5` for MySQL, PostgreSQL, SQLite, SQL Server, and ClickHouse, and `All` for
all seven engines including MongoDB and Redis.

| Engine | Legacy data workflow | Write boundary | Additional baseline |
| --- | --- | --- | --- |
| MySQL | SQL editor and relational grid | Structured row editing | Database and object management, export, backup/restore, same-engine copy, charts, ER, performance |
| PostgreSQL | SQL editor, schema tree, and relational grid | Structured row editing | Schema/database/object management, export, backup/restore, same-engine copy, charts, ER, performance |
| SQLite | SQL editor and relational grid | Structured row editing | Tables, views, triggers, export, backup/restore, same-engine copy, charts, ER, performance |
| SQL Server | SQL editor and relational grid | Structured row editing | Database and object management, export, backup/restore, same-engine copy, charts, ER, performance |
| ClickHouse | SQL editor and relational grid | Grid is read-only | Database/table/view/function browsing, export, backup/restore, same-engine copy, charts, performance |
| MongoDB | Collection tree and paged JSON documents | Document viewer is read-only | Collection filtering, collapsible JSON, index catalog, and engine dashboard |
| Redis | Database/key tree, raw commands, and typed key viewer | String, hash, list, set, sorted-set, and TTL editing | Cursor-based key search and engine dashboard |

The capability matrix owned by `src-tauri/src/db/engine.rs` decides whether an action is available.
Milestone acceptance must test both supported behavior and the absence of unsupported actions. In
particular, generic relational actions exposed for MongoDB by Legacy Shell menu branching are not
workflows to preserve, and ClickHouse data editing remains disabled.

## Workflow parity checklist

### Shell, workspace, and Connection Profiles

| ID | Legacy workflow | Engines | Destination | Acceptance check |
| --- | --- | --- | --- | --- |
| S01 | Start the desktop workspace and load saved state | All | M3 | Native repository data is loaded before initialization; corruption or credential failure is visible and leaves the data untouched. |
| S02 | Sidebar, tabbed work area, status bar, overlays, and notifications | All | M3 | Each surface renders its empty, active, and error state and keeps active profile/database context synchronized. |
| S03 | Command palette and shortcut dispatch | All | M3 | Keyboard selection works; workspace commands take precedence without stealing normal editable or IME input. |
| S04 | Show/hide the sidebar and zoom in, out, or reset | All | M3 | Commands update the active window deterministically and remain repeatable where the Legacy Shell allowed repetition. |
| S05 | Create, select, cycle, close, and bulk-close tabs | All | M3 | Active-tab selection is stable and closing left, right, other, or all tabs never closes an unsaved query without confirmation. |
| S06 | Simplified Chinese/English and light/dark/system appearance | All | M3 | Language and theme switch the complete shell; missing WebView preferences use documented native defaults. |
| C01 | Create, view, edit, and test a Connection Profile | All | M3 | Engine-specific fields and defaults are correct; success, validation, credential, and connection failures are visible. |
| C02 | Connect, browse, refresh, and disconnect a Database Session | All | M3 | Every engine crosses configure, test, connect, lazy browse, refresh, and disconnect through Application Core. |
| C03 | Delete a Connection Profile and its saved credential | All | M3 | Deletion requires an explicit destructive confirmation and reports repository or credential failure without losing the profile. |
| C04 | Group, tag, filter, color, collapse, and select Connection Profiles | All | M3 | Grouped and ungrouped profiles remain discoverable and profile metadata survives restart. |
| C05 | Show connecting, connected, disconnecting, MCP-use, and failure state | All | M3 | State is readable without relying only on color; Usage Leases prevent unsafe edit, delete, or disconnect operations. |
| C06 | Reconcile repository revisions and live MCP usage | All | M3 | Stale profile revisions are rejected and UI state refreshes without overwriting a newer repository snapshot. |

### Querying and result interaction

| ID | Legacy workflow | Engines | Destination | Acceptance check |
| --- | --- | --- | --- | --- |
| Q01 | Create a query and open/save a SQL file | SQL5 | M4 | File dialogs preserve text and file identity; dirty state clears only after a successful save. |
| Q02 | Edit with local syntax highlighting and schema-aware completion | SQL5 | M4 | Dialect keywords, tables, columns, and engine-correct identifier quoting work without marketplace or network access. |
| Q03 | Execute the selection, current statement, or full editor | SQL5 | M4 | The correct range executes; multiple statements remain ordered and a failed statement does not hide prior results. |
| Q04 | Explain a statement | SQL5 | M4 | Each engine uses its declared Explain mode and unsupported contexts do not expose the action. |
| Q05 | Inspect multiple results, affected rows, timing, and errors | SQL5 | M4 | Success, empty results, mixed sequential results, and errors remain distinguishable and associated with their statement. |
| Q06 | Find/replace, focus, selection, undo/redo, and IME composition | SQL5 | M4 | Editor-native operations coexist with Astesia commands and preserve marked-text composition and grouped undo/redo. |
| Q07 | Select result rows or a rectangular cell range and copy TSV | SQL5 | M4 | Clipboard output includes the selected headers and values in stable row/column order with JSON values serialized. |
| Q08 | Run raw Redis commands from a query tab | Redis | M6 | Commands execute against the selected Redis database and show data, empty, and error results without SQL-only completion. |

### Catalog, objects, and data grids

| ID | Legacy workflow | Engines | Destination | Acceptance check |
| --- | --- | --- | --- | --- |
| D01 | Lazily browse databases, schemas, tables/collections/keys, views, functions, procedures, triggers, and users | All by capability | M5 | Expanding or refreshing a node loads only supported children and shows empty, loading, and error states in place. |
| D02 | Inspect table columns, indexes, constraints, and foreign keys | SQL5 by capability | M5 | Structure metadata matches the engine catalog and unsupported sections are absent rather than empty-looking actions. |
| D03 | Open view, function, and procedure definitions | SQL5 by capability | M5 | The selected object's qualified identity is preserved and missing definitions produce a visible empty or error state. |
| D04 | Create databases, schemas, tables, views, functions, procedures, triggers, and users | SQL5 by capability | M5 | Forms generate engine-correct DDL, refresh the owning catalog on success, and surface validation or execution failure. |
| D05 | Rename or drop database objects | SQL5 by capability | M5 | Names are quoted for the engine, the catalog refreshes after success, and destructive operations require confirmation. |
| D06 | Browse table data with pagination, row counts, WHERE filtering, ORDER BY sorting, and refresh | SQL5 | M5 | Paging and filtering are engine-correct; pending edits prevent navigation that would silently discard changes. |
| D07 | Stage cell updates, row inserts, and row deletions | MySQL, PostgreSQL, SQLite, SQL Server | M5 | Mutations require a usable primary key and remain local until Save; Undo and Discard restore the prior staged state. |
| D08 | Save a batch of staged grid changes | MySQL, PostgreSQL, SQLite, SQL Server | M5 | Updates, inserts, and deletes target the original row identity; partial or failed saves remain visible and reload does not claim success. |
| D09 | Use typed editors and a long-text/JSON value viewer | MySQL, PostgreSQL, SQLite, SQL Server | M5 | Boolean, date/time, enum, JSON, null, and long values round-trip without losing type intent. |
| D10 | Resize columns and select/copy/paste rows or cell ranges | MySQL, PostgreSQL, SQLite, SQL Server | M5 | Selection is stable across drag/shift/meta gestures and CSV/TSV paste respects headers, bounds, nulls, and staged undo. |
| D11 | Browse ClickHouse table data | ClickHouse | M5 | Filtering, sorting, paging, selection, copying, and export work while every mutation entry point stays disabled. |

### Engine-specific, long-running, and MCP workflows

| ID | Legacy workflow | Engines | Destination | Acceptance check |
| --- | --- | --- | --- | --- |
| E01 | Browse, filter, page, and refresh MongoDB collections | MongoDB | M6 | Documents render as collapsible typed JSON; empty collections and query errors remain distinguishable. |
| E02 | Search Redis keys and inspect key type, TTL, and value | Redis | M6 | Cursor traversal completes without `KEYS`, duplicate keys are removed, and missing/expired keys show a stable state. |
| E03 | Create/update strings and edit hash, list, set, and sorted-set members | Redis | M6 | Type-specific commands update the selected key and refresh its value without exposing relational mutations. |
| E04 | Delete a Redis key | Redis | M6 | Deletion requires destructive confirmation; cancellation changes nothing and success removes the key from tree and viewer state. |
| O01 | Export current, all, or ranged rows with selected columns | SQL5 | M6 | CSV, JSON, and XLSX options produce the selected scope and report the written row count or a visible failure. |
| O02 | Back up selected tables as structure, data, or both | SQL5 | M6 | File selection, drop options, progress, cancellation, partial failure, and completion reflect durable output accurately. |
| O03 | Restore from a selected backup file | SQL5 | M6 | The task targets the selected Database Session and reports progress, cancellation, partial application, and failure. |
| O04 | Copy a table between profiles/databases of the same engine | SQL5 | M6 | Drag/drop and copy/paste choose an explicit target/name; structure/data options and partial results are reported. |
| O05 | Inspect and cancel background tasks | SQL5 | M6 | Running, cancelling, completed, failed, partial, and cancelled states update once and remain inspectable. |
| O06 | Receive task progress/completion notifications | SQL5 | M6 | Progress is monotonic for one task and terminal notification is emitted exactly once. |
| O07 | Start, stop, restart, and inspect the MCP Sidecar; copy client configuration | All | M6 | Loopback endpoint/token state is accurate, shutdown terminates the process, and failure leaves retryable controls. |
| O08 | Use profiles through MCP with Usage Leases and destructive approval | All by capability | M6 | Desktop and MCP share revisions; updates, deletes, permissions, destructive DDL, and unknown SQL follow session-scoped approval rules. |

### Visualization, diagnostics, and cutover

| ID | Legacy workflow | Engines | Destination | Acceptance check |
| --- | --- | --- | --- | --- |
| V01 | Build bar, line, area, scatter, and pie charts from table/query results | SQL5 | M7 | Column mapping, aggregation, empty/non-numeric data, refresh, and chart switching work on representative results. |
| V02 | Explore an ER diagram with layout, drag, pan, zoom, fit, and overview | MySQL, PostgreSQL, SQLite, SQL Server | M7 | Tables and foreign-key edges use qualified identities and remain usable on empty, small, and large schemas. |
| V03 | View engine-specific performance metrics with manual/automatic refresh | All | M7 | All seven dashboards expose their Legacy Shell metrics, errors, and 5/10/30/60-second refresh intervals. |
| P01 | Use native open/save dialogs, filesystem access, clipboard, and preferences | All | M8 | Each replacement works without Tauri and reports cancellation separately from platform failure. |
| P02 | Check application version and relaunch after an internal update | All | M8 | Native version/relaunch plumbing is validated for internal packages; public updater compatibility remains outside the rebuild gate. |
| P03 | Run the final packaged application without React, WebView, or Tauri | All | M8 | Locked artifacts contain no frontend/Tauri runtime and the complete checklist passes on every supported internal platform. |

## Shortcut contract

`Mod` means Command on macOS and Control on Windows/Linux. When `Mod+S` or `Mod+R` has more than
one eligible handler, the active view owns the command.

| Shortcut | Legacy command | Destination | Acceptance check |
| --- | --- | --- | --- |
| `Mod+Shift+P` | Command palette | M3 | Opens from editable and non-editable focus without committing IME text. |
| `Mod+N` | New query | M3 | Opens a query for the active Database Session and does not replace the current tab. |
| `Mod+B` | Toggle sidebar | M3 | Hides/restores the same sidebar state. |
| `Mod+W` | Close active tab | M3 | Dirty queries require confirmation. |
| `Ctrl+Tab` | Next tab | M3 | Repeats and wraps through tabs. |
| `Ctrl+Shift+Tab` | Previous tab | M3 | Repeats and wraps through tabs. |
| `Mod+Enter` | Execute selection/full query | M4 | Executes the selection when present and the intended full range otherwise. |
| `Mod+Shift+Enter` | Execute current statement | M4 | Executes only the statement containing the caret. |
| `Mod+O` | Open SQL file | M4 | Loads the selected file without losing unsaved content. |
| `Mod+S` | Save SQL file or grid changes | M4/M5 | The active eligible view wins; disabled handlers do not consume the command. |
| `Mod+R` | Refresh active view | M4/M5/M7 | Refreshes the active surface only and is disabled during conflicting work. |
| `Mod+=`, `Mod+Shift+=` | Zoom in | M3 | Repeats without changing editor contents. |
| `Mod+-` | Zoom out | M3 | Repeats without changing editor contents. |
| `Mod+0` | Reset zoom | M3 | Returns to the documented default. |
| `Mod/Ctrl+C`, `Mod/Ctrl+V`, `Mod/Ctrl+Z` | Grid copy, paste, and staged undo | M5 | Grid handlers run only outside an active cell editor and never override normal text editing. |

## State contract

| Surface | Empty state | Loading state | Error state | Destination |
| --- | --- | --- | --- | --- |
| Startup | No saved profiles | Native State Probe | Repository/credential failure with remediation | M3 |
| Connections | No profiles or no filter matches | Connecting/disconnecting | Test, save, revision, credential, or session failure | M3 |
| Catalog tree | No databases/objects | Per-node lazy load | Per-node load failure without clearing prior data | M3/M5 |
| Query | Empty editor/result | Executing statement(s) | Per-statement error with prior results retained | M4 |
| Relational grid | Zero rows | Initial load/refresh/save | Query, count, metadata, or mutation failure | M5 |
| MongoDB | Empty collection/filter result | Document fetch | Filter/query failure | M6 |
| Redis | No keys or expired key | Key scan/value refresh | Command/type/connection failure | M6 |
| Transfers/tasks | No tasks | Running/cancelling with progress | Failed or partial terminal outcome | M6 |
| Charts/ER/performance | No usable data | Metric/schema/query refresh | Unsupported mapping or engine/query failure | M7 |
| MCP/update | Service stopped/no update | Starting/stopping/checking/installing | Sidecar, configuration, download, or relaunch failure | M6/M8 |

## Confirmation contract

| Operation | Required behavior | Destination |
| --- | --- | --- |
| Close one or more dirty query tabs | Confirm before discarding; cancellation preserves every tab and active selection. | M3/M4 |
| Delete a Connection Profile | Destructive confirmation names the profile and saved credential consequence. | M3 |
| Drop a database or schema | Destructive confirmation names the qualified target and cancellation issues no statement. | M5 |
| Drop a table, view, function, procedure, or trigger | Destructive confirmation names the qualified target and refreshes only after success. | M5 |
| Delete a Redis key | Destructive confirmation names the key and cancellation leaves tree/view state unchanged. | M6 |
| Save staged relational deletes | The grid shows the staged deletion count and offers Undo/Discard before Save commits the batch. | M5 |
| Execute destructive work through MCP | Explicit approval is required; only eligible updates may suppress prompts for the current session. | M6 |
| Restore/copy/backup with durable partial output | The terminal state must say Partial or Failed accurately; cancellation must not be reported as success. | M6 |

## Fixed replacements and exclusions

- Legacy WebView `localStorage` connection/preference import is replaced by the Milestone 3 Native
  State Probe and documented defaults. Automatic WebView-only state import is not a parity gate.
- Browser-only development and the unused React plugin scaffold are not product workflows. They
  are removed with the frontend in Milestone 8.
- Pixel-level React/CSS reproduction, public updater compatibility, signing, notarization, and
  external distribution remain outside the internal rebuild gate.

## Verification

- At detached Legacy Shell commit `5004eb9`, `RUSTUP_TOOLCHAIN=1.97.1 cargo test --locked
  --manifest-path src-tauri/Cargo.toml -q`: 148 library tests and one MCP binary test passed.
- On the current branch, `RUSTUP_TOOLCHAIN=1.97.1 cargo test --locked --manifest-path
  src-tauri/Cargo.toml -q`: 267 tests passed, two environment-dependent smoke tests were ignored,
  and the MCP binary test passed.
- `RUSTUP_TOOLCHAIN=1.97.1 cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm build`: passed with the existing large-chunk warning.
- `pnpm lint`: unchanged legacy React baseline of 85 errors and one warning.
- `git diff --check`: passed. The checklist audit found 49 unique workflow IDs and 15 shortcut
  rows.

Every user-visible Legacy Shell workflow now has a destination milestone and a concrete acceptance
check. Later milestone documents should cite the checklist IDs they satisfy and must not mark the
Cutover Gate complete while any applicable row remains unverified.
