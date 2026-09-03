# GPUI UI Rebuild Plan

## Outcome

Replace the React/WebView/Tauri desktop shell with a native GPUI application while retaining Astesia's existing Rust database, repository, task, and MCP behavior. The first pass preserves workflows and shortcuts rather than redesigning the product or matching CSS pixels.

This is an internal rebuild. External licensing, public updater compatibility, signing, and automatic migration of WebView-only state are not current completion gates.

## Fixed decisions

- The final runtime is GPUI with no Tauri or WebView dependency.
- The existing `astesia` binary entry is replaced in place; there is no second or feature-gated Legacy Shell.
- The Legacy Shell remains available through Git history and prior artifacts only.
- The pinned Zed commit is `399258feeaf90ad8a3a208c99221ee87b6452f38`.
- Astesia uses Zed's complete editor stack internally, with every Zed crate pinned to the same commit.
- Rust 1.97.1 is the repository and CI toolchain baseline.
- Zed data is isolated below Astesia's data directory and Zed telemetry, AI, authentication, collaboration, and network connections stay disabled.
- Existing native Connection Profiles and credentials are retained when readable. WebView-only connections and preferences are not imported; MCP port and token may reset.
- macOS is the implementation and validation platform first. Windows and Linux follow after the main workflows work on macOS.
- Browser-only development and the unused React plugin scaffold are removed with the frontend.

## Target structure

Keep the existing Rust package location during the migration so path churn does not obscure behavior changes. Rename `src-tauri` only as a final mechanical cleanup if it remains worthwhile.

```text
src-tauri/src/
├── application/     UI-independent application services and workflow models
├── platform/        Files, dialogs, preferences, events, processes, and updates
├── ui/              GPUI application, window, entities, views, actions, and theme
├── db/              Existing database drivers
├── mcp/             Existing MCP protocol and service logic
├── tasks/           Existing long-running task models
└── main.rs           GPUI process entry
```

The dependency direction is strict:

- `ui` may depend on `application` and narrow `platform` interfaces.
- `application` may depend on `db`, `mcp`, `tasks`, and repository abstractions.
- `application`, `db`, `mcp`, and `tasks` must not depend on GPUI or Tauri.
- Platform implementations may depend on GPUI, the OS, and packaging libraries.

Use separate narrow interfaces such as `UiEventSink`, `SidecarHost`, `FileDialogs`, and `Preferences`; do not replace Tauri with one broad platform trait.

## Milestones

### 0. Freeze the behavioral baseline

Status: Complete on 2026-09-01. See [Milestone 0 acceptance](gpui-milestone-0-acceptance.md).

- Record the current Legacy Shell commit as the reference point.
- Preserve the existing 149-test Rust baseline.
- Turn the current workflow inventory into a parity checklist covering seven database engines, shortcuts, empty/loading/error states, and destructive confirmations.
- Keep visual comparison at the information-architecture level, not pixel level.

Exit condition: every existing workflow has an explicit destination milestone and acceptance check.

### 1. Prove the GPUI and Zed editor runtime

Status: Complete on 2026-09-01. See [Milestone 1 acceptance](gpui-milestone-1-acceptance.md).

- Pin Rust 1.97.1 with `rust-toolchain.toml` and update the package toolchain declaration.
- Add the Zed editor runtime, UI, settings, theme, language, and asset crates at the same pinned commit as `gpui` and `gpui_platform`.
- Replace `app_lib::run()` with a GPUI application bootstrap based on Zed's minimal component-preview initialization path.
- Set the custom Zed data directory before any Zed path or database access.
- Disable telemetry, AI, login, collaboration, extension downloads, and Zed server connections.
- Render one standalone local Zed `Editor` buffer per Astesia window without initializing Zed `Client`, `Project`, or `Workspace` services.
- Initialize Tokio through the GPUI-compatible runtime bridge.
- Match the current 1280×800 window and 960×600 minimum size.

Exit condition: the macOS app opens a native window containing the Zed editor; typing, selection, IME, undo/redo, focus, resize, and shutdown work without reading the user's Zed data or making Zed network requests.

### 2. Extract the Application Core

Status: Complete on 2026-09-01. See [Milestone 2 acceptance](gpui-milestone-2-acceptance.md).

- Remove `tauri::AppHandle` from the core `AppState` boundary.
- Move business logic out of command handlers into UI-independent application services.
- Preserve the existing connection repository, credential vault binding, revision checks, Usage Leases, database driver map, task state, and MCP snapshot contracts.
- Replace Tauri event emission with `UiEventSink` subscriptions.
- Replace Tauri sidecar and shell calls with `SidecarHost`.
- Add an explicit per-engine capability matrix so the GPUI menu cannot expose unsupported MongoDB or ClickHouse operations.

Exit condition: core workflows compile and test without Tauri types; Tauri command modules are no longer needed by the new entry point.

### 3. Build the native shell and connections workflow

Status: Complete on 2026-09-01. See [Milestone 3 acceptance](gpui-milestone-3-acceptance.md).

- Implement GPUI entities for application state, the workspace, tabs, Sidebar, Status Bar, overlays, notifications, theme, and language.
- Port the command palette and shortcut precedence, including IME composition and editable-focus handling.
- Implement Connection Profile create, edit, test, connect, disconnect, delete, grouping, tags, lazy schema loading, MCP badges, and Usage Lease restrictions.
- Read the existing native repository before initialization. On corruption or credential failure, show an error and leave the data untouched rather than silently creating an empty repository.
- Reset WebView-only theme, layout, update-skip, and MCP endpoint preferences to documented defaults.

Exit condition: all seven engines can be configured, tested, connected, disconnected, and browsed on macOS without React or Tauri IPC.

### 4. Port querying as the first complete vertical slice

Status: Complete on 2026-09-02. See [Milestone 4 acceptance](gpui-milestone-4-acceptance.md).

- Wrap Zed `Editor` in an Astesia-owned `QueryItem` containing connection context, result state, execution state, and file state.
- Bundle SQL grammar and highlighting locally; do not initialize the Zed extension marketplace.
- Port Astesia's dialect keywords, schema/table/column completion, identifier quoting, statement splitter, current-statement execution, selection execution, and sequential multi-statement errors.
- Implement open/save, find/replace, completion keyboard interaction, Explain, result tabs, timing, and errors.
- Build the result grid selection and clipboard contract before charts or exports.

Exit condition: the five SQL engines support open/edit/save, completion, selected/current/full execution, Explain, multiple results, errors, range selection, and TSV copy with existing shortcuts.

### 5. Port schema browsing and editable data grids

Status: Complete on 2026-09-02. See [Milestone 5 acceptance](gpui-milestone-5-acceptance.md).
The first slice adds actionable SQL table rows, a native table-structure workspace item for columns
and indexes, and an Application Core state model that
rejects stale loads when a connection session changes. The workspace tab model now hosts typed
query and table-structure items so later data-grid sessions can reuse the same navigation seam. The
second slice extracts a UI-independent `GridSession` for paging, typed sorting, filter state, row and
cell selection, staged edits/inserts/deletes, undo, discard, and deterministic save planning. It
keeps ClickHouse read-only, requires exactly one primary key for writes, preserves unsaved changes
when a session becomes unavailable, and orders primary-key edits after other edits to the same row.
The third slice adds a session-generation-checked `GridService` that loads filtered, typed-sorted,
paginated rows with authoritative column metadata and executes each save plan as one transaction on
one driver session. Successful saves invalidate the page for reload; any failed statement rolls the
whole batch back and leaves the staged changes available for correction, undo, discard, or retry.
The fourth slice exposes a native read-only `DataGridItem`: table rows open reusable data tabs,
while a dedicated tree action keeps table structure accessible. The grid virtualizes rows, supports
row and rectangular cell selection, cycles single-column typed sorting from keyboard or pointer,
and presents refresh, pagination, row totals, empty results, failures, and invalidated sessions.
The fifth slice connects editable single-primary-key grids to the staged mutation service. Operators
can open an inline cell editor from the keyboard or pointer, validate boolean, numeric, JSON, text,
and nullable values, then Undo, Discard, or Save the batch. Dirty cells and tabs remain visible,
navigation is locked until changes are resolved, closing a dirty grid requires confirmation, and
failed transactions preserve the complete staged change set for correction or retry. ClickHouse,
missing-primary-key tables, and composite-primary-key tables remain explicitly read-only.
The sixth slice completes the grid interaction contract: draft rows distinguish database defaults
from explicit SQL NULL, staged row deletion stays reversible, filtering can recover from query
errors, and rectangular CSV/TSV paste is one undoable batch. Copy uses `\\N` for SQL NULL, typed
editors validate date/time, integer, JSON, and catalog-backed enum values, long values use an
expanded editor, and columns can be resized without hiding the Save action at the minimum window
width. The seventh slice expands each database into capability-gated catalog sections, adds table
constraints and foreign keys to structure tabs, and opens view, function, and procedure definitions
as qualified read-only SQL snapshots with explicit missing-definition and stale-session states. The
final slice adds typed create, rename, and drop workflows with complete destructive identities,
credential-safe notices, and visible busy/error states; completes ClickHouse read-only CSV export;
and passes the catalog, object, and grid workflow against disposable instances of all seven engines.

- Move SQL and DDL construction out of views into typed application services.
- Implement schema trees, table structure, indexes, object definitions, and supported create/rename/drop actions from the engine capability matrix.
- Extract a `GridSession` model for pagination, filtering, sorting, selection, staged insert/update/delete operations, undo, discard, and batch save.
- Implement typed cell editors, paste parsing, column resize, long-text/JSON viewing, primary-key edit gating, and ClickHouse read-only behavior.

Exit condition: schema and editable-grid workflows match the Legacy Shell for every engine that supports them, including failure and destructive-confirmation paths.

### 6. Port engine-specific and long-running workflows

Status: Complete on 2026-09-03. See [Milestone 6 acceptance](gpui-milestone-6-acceptance.md).

- Port MongoDB collection/document/filter workflows.
- Port Redis key search, TTL, and string/hash/list/set/zset editing.
- Port backup, restore, table copy, export, task progress, cancellation, and completion notifications.
- Reconnect the MCP Sidecar using `SidecarHost`; generate a new local port/token configuration when the old WebView values are unavailable.

Exit condition: MongoDB, Redis, tasks, exports, and MCP work without Tauri process or event APIs.

### 7. Port visualization and diagnostics

- Implement table/query charts for bar, line, area, scatter, and pie data.
- Implement ER layout, dragging, pan, zoom, fit, and overview behavior.
- Port the seven-engine performance dashboard and refresh intervals.

Exit condition: charts, ER, and performance workflows meet behavioral parity on representative datasets.

### 8. Remove the old runtime and finish platform support

- Delete React, Zustand, Radix, Monaco, Vite, Tauri commands, plugins, capabilities, configuration, build hooks, and unused plugin interfaces once their replacement milestone passes.
- Replace file dialogs, filesystem access, clipboard, preferences, sidecar staging, application version, and relaunch with native implementations.
- Produce internal macOS packages first, then validate Windows x64 and Linux x64 windowing, fonts, input, file dialogs, sidecar placement, and graphics backends.
- Keep any future updater endpoint under the existing `ChingCdesu/Astesia` release authority.
- Optionally rename `src-tauri` after `rg` confirms no Tauri dependency or terminology remains.

Exit condition: the locked build contains no Tauri or frontend runtime dependency, and the parity checklist passes on every platform still used internally.

## Verification strategy

Add tests at the extracted model boundaries rather than trying to reproduce browser E2E infrastructure:

- Application model tests for Connection Profiles, Database Sessions, revision invalidation, Usage Leases, tabs, stale requests, and engine capabilities.
- SQL golden tests for splitting, quoting, generated DDL, Explain, filtering, and pagination.
- `GridSession` tests for selection, edit staging, insert/delete, undo, discard, paste, primary-key gating, and ClickHouse read-only behavior.
- Event and sidecar tests for task progress, cancellation, MCP snapshots, and shutdown.
- GPUI render/action tests for empty, loading, error, data, overlay, focus, and shortcut states.
- Manual macOS checks for Chinese IME, multi-cursor editing, clipboard, dialogs, large documents, large result sets, Retina rendering, and long-running tasks.
- Focused Windows/Linux smoke checks after macOS workflow parity.

Continue running:

```sh
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

## First implementation batch

The first batch should stop after retiring the highest technical risk:

1. Pin Rust 1.97.1.
2. Add the complete, same-SHA Zed editor dependency set.
3. Replace the desktop entry with an isolated, offline GPUI/Zed bootstrap.
4. Render a single local editor buffer in the current window dimensions.
5. Verify locked compilation, the existing Rust tests, native launch, IME, focus, undo/redo, and absence of Zed network access.

Do not begin component-by-component porting until this batch passes. A failure here changes the editor or dependency strategy before Application Core and UI work accumulate around it.

## Explicitly outside the internal rebuild

- Pixel-perfect reproduction of the React UI.
- A browser version.
- The unused React plugin scaffold.
- Automatic import of WebView localStorage.
- A parallel Legacy Shell binary.
- Public distribution licensing, notarization, installer signing, and external updater guarantees.
