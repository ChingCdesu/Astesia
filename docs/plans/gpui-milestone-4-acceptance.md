# GPUI Milestone 4 acceptance

Status: Complete on 2026-09-02 on macOS.

## Delivered

- Each native query tab owns a Zed editor, Database Session target, file state, execution state,
  ordered statement results, result selection, and errors without React or Tauri IPC.
- Native file dialogs support open and save, preserve file identity, track dirty state across
  in-flight writes, and require confirmation before unsaved text is replaced or closed.
- The bundled SQL grammar and highlight query work without the extension registry or network
  access. Completion combines five dialect vocabularies with session-scoped table, schema, and
  lazily loaded column metadata using engine-correct identifier quoting.
- Selection, current-statement, and full-document execution share the dialect-aware SQL splitter.
  Sequential results preserve statement order and keep earlier results visible when a later
  statement fails.
- Explain uses each engine's declared mode. SQL Server restores `SHOWPLAN_ALL` on the same session,
  including failure cleanup, and unsupported targets do not expose the workflow.
- Native find/replace, editor focus, completion, undo/redo, IME composition, result tabs, timing,
  errors, row/cell range selection, and deterministic TSV copy use scoped GPUI actions and existing
  shortcuts.
- MySQL switches databases through the text protocol before query execution because `USE` is not
  supported by its prepared-statement protocol.

## Workflow evidence

| ID | Acceptance evidence |
| --- | --- |
| Q01 | `QueryFileState` tests cover open identity, successful and failed saves, edits during in-flight writes, and dirty-state clearing. `QueryItem` owns the native dialogs and unsaved-change confirmation. |
| Q02 | The bundled Tree-sitter SQL test verifies local highlighting. Completion tests cover all five dialect vocabularies, schema/table/column metadata, catalog invalidation, and identifier quoting. |
| Q03 | `SqlScript` and `QueryWorkspaceState` tests cover dialect-aware splitting, selection precedence, current-statement lookup, full execution, ordered results, and first-failure selection. |
| Q04 | Engine capabilities declare the Explain mode. Workspace tests gate unsupported or multi-statement requests, SQL Server owns session-safe cleanup, and the live SQL5 smoke executes Explain through Application Core. |
| Q05 | Query workspace and native result rendering retain multiple statement tabs, timing, affected rows, empty results, prior successes, and the selected failure. |
| Q06 | The query editor uses a dedicated action context and Zed's buffer search. GPUI tests cover find/replace, focus return, and grouped undo; the standalone editor coverage preserves marked-text composition. |
| Q07 | Result-selection tests cover rectangular cells, replace/toggle/extend row selection, select-all, stable row/column order, optional headers, JSON serialization, and quoted TSV fields. `QueryItem` binds the clipboard and selection actions to the result grid context. |

## Verification

- `rustup run 1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml -q`:
  284 passed and 2 environment-dependent tests were ignored; the MCP sidecar test also passed.
- `ASTESIA_ENGINE_SMOKE_CONFIG_PATH="$PWD/.scratch/engine-smoke.json" rustup run 1.97.1
  cargo test --locked --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture`: both ignored
  tests passed against disposable PostgreSQL, Redis, MySQL, MongoDB, ClickHouse, SQLite, and SQL
  Server instances. All seven engines crossed their driver workflow; the Application Core workflow
  also configured, tested, connected, browsed, and disconnected every engine.
- The Application Core live smoke executed `SELECT 1` and Explain for MySQL, PostgreSQL, SQLite,
  SQL Server, and ClickHouse, asserting one query result, one column, one row, and a successful
  Explain result for every SQL engine.
- The live smoke initially reproduced MySQL error 1295 while preparing `USE`. The targeted
  `ASTESIA_ENGINE_SMOKE_TARGET=mysql` loop passed after database selection moved to the text
  protocol, and the complete seven-engine run then passed.
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

The query vertical slice satisfies Q01-Q07 and the Milestone 4 exit condition on macOS. The normal
suite keeps both seven-engine tests ignored because they require disposable services and a local,
untracked configuration; both were run explicitly for this acceptance. Cross-platform behavior and
the final packaged-application gate remain Milestone 8 scope.
