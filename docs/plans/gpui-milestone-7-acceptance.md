# GPUI Milestone 7 acceptance

Status: Complete on 2026-09-04 on macOS.

## Delivered

- Table and query results switch in place between the grid and native bar, line, area, scatter,
  and pie charts. Column mapping preserves the selected X and Y columns across chart-type and data
  refreshes, categorical X values aggregate duplicate labels, and numeric X values retain every
  point.
- Table charts load every page matching the active filter and sort through the generation-checked
  grid service. Query charts remain bound to the active statement result. Empty results,
  non-numeric results, invalid scatter mappings, and loading all have explicit states. Refresh
  failure retains the last usable chart, while stale query sessions clear their visualization.
- MySQL, PostgreSQL, SQLite, and SQL Server database sessions open native ER tabs. Tables and
  relationships use qualified `TableRef` identities; deterministic layout handles disconnected
  tables and cycles; table nodes expose columns and primary keys; and the canvas supports node
  drag, background pan, wheel/button zoom, fit, and a viewport-aware overview.
- All seven engines expose their engine-specific performance metrics in a native workspace tab.
  MongoDB now uses `serverStatus`; manual refresh and 5/10/30/60-second automatic refresh retain
  the prior snapshot while loading and surface refresh failures without clearing it.
- `Mod+R` is owned by the active chart, ER diagram, performance dashboard, or data grid before the
  connection sidebar fallback. Every visualization is invalidated when its exact Database Session
  generation is replaced.

## Workflow evidence

| ID | Acceptance evidence |
| --- | --- |
| V01 | Pure model tests cover default mapping, categorical aggregation, numeric X values, all five chart switches, empty results, non-numeric results, and invalid scatter mapping. The seven-engine smoke builds all five chart types from representative query results on SQL5 and reloads a three-row table through the all-pages chart service on each engine. Native macOS rendering verified controls, two-series bars, legend, plot bounds, and first/last labels. |
| V02 | Layout tests cover empty, two-table, 80-table, qualified same-name, and cyclic schemas. The live smoke creates a real parent/child foreign key on MySQL, PostgreSQL, SQLite, and SQL Server, then verifies qualified tables, endpoints, and columns through `ErDiagramService`. Native macOS rendering verified readable qualified nodes, relationship paths, fit/zoom controls, and overview. Native handlers implement drag, pan, and bounded zoom; state tests cover retained refresh data and stale completion rejection. |
| V03 | Dashboard tests cover interval values, retained loading data, refresh errors, superseded completions, and session invalidation. Parser tests cover MongoDB native and extended numeric representations. The live smoke retrieves the matching performance snapshot variant from all seven engines. Native macOS rendering verified PostgreSQL sections, compact metric density, manual refresh, and the automatic-refresh control. |

## Verification

- `rustup run 1.97.1 cargo test --locked --manifest-path src-tauri/Cargo.toml --quiet`:
  357 tests passed, six environment-dependent tests were ignored, and the MCP binary test passed.
- `ASTESIA_ENGINE_SMOKE_CONFIG_PATH="$PWD/.scratch/engine-smoke.json" rustup run 1.97.1 cargo
  test --locked --manifest-path src-tauri/Cargo.toml
  application::engine_smoke_tests::milestone_seven::milestone_seven_visualization_and_diagnostics_workflows
  -- --ignored --nocapture`: passed against disposable MySQL 8.4, PostgreSQL 17, SQLite,
  SQL Server 2022, MongoDB 8.2, Redis 7, and ClickHouse 25.8 instances.
- The native debug application was rendered on macOS at the production 1280x800 window size.
  Bounded visual review covered representative chart, ER, and performance surfaces in the real
  GPUI renderers. The review strengthened ER relationship contrast and added the missing current
  viewport indicator to the overview.
- `rustup run 1.97.1 cargo check --locked --manifest-path src-tauri/Cargo.toml`: passed.
- `rustup run 1.97.1 cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets`:
  passed with the migration branch's existing warnings.
- `rustup run 1.97.1 cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `pnpm build`: passed. Vite reported only the existing large-chunk warning.
- `git diff --check`: passed.

## Existing frontend lint baseline

`pnpm lint` continues to report the retained Legacy Shell baseline of 85 errors and one warning.
The Milestone 7 implementation adds no TypeScript, React, or stylesheet source; removing that shell
and its lint debt remains Milestone 8 scope.

## Code-quality closure

The application layer owns chart shaping, all-pages loads, ER metadata/layout, performance
snapshots, session generations, and stale-completion rejection. GPUI views own only presentation,
focus, and pointer interaction. The requested simplification pass removed repeated pointer and
node-offset calculations and clarified the chart-page request boundary. The final specification
review also aligned nullable categorical series, cleared charts with their Database Session, and
made ER fit and overview bounds include dragged nodes. The Impeccable HTML/CSS detector does not
apply to this native Rust surface, so the final quality pass used rendered macOS screenshots and
the native craft floor instead.

## Closure boundary

V01-V03 satisfy the Milestone 7 exit condition on macOS. The normal suite keeps the seven-engine
test ignored because it requires disposable services and an untracked local configuration; it was
run explicitly for this acceptance. Legacy Shell removal, native packaging/update cutover, and
Windows/Linux validation remain Milestone 8 scope.
