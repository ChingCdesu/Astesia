# Figma implementation coverage

Source: [Astesia — Zed Workspace UI](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=185-2703).

Inventory read from the live document on 2026-09-05 using the Plugin API. The metadata page-list endpoint returned only Cover; the full document contains the following pages.

| Page | Node | Contents |
| --- | --- | --- |
| Cover | 0:1 | Design cover, not an application route |
| Foundations | 7:3 | Tokens and typography |
| --- | 7:4 | Separator |
| Components | 7:5 | Shell and reusable components |
| --- | 7:6 | Separator |
| Utilities | 7:7 | Empty |
| Screens | 7:8 | Application screens and state variants |
| Context Menu | 79:2 | Profile menus, interaction states and delete confirmations |

## Final acceptance summary

All 8 document pages are inventoried, including the two separators and empty Utilities page. All 38 Screens variants are implemented and have native rendered evidence. Cover, Foundations and Components are supporting design artifacts, not invented application routes. The separate Light Command Palette children are covered together.

| Contract | Owning implementation | Evidence |
| --- | --- | --- |
| Empty startup and Zed shell/tabs/settings | `src/ui/workspace/`, `src/ui/tabs.rs`, `src/ui/mod.rs` | Passes 7–12, 26; both themes, compact startup, closing the last tab, dirty close confirmation |
| Profile/catalog/menu states | `src/ui/connections/` | Passes 9–12, 26, 30–39; lazy catalog, connected/disconnected/busy/MCP, live recovery, disabled AX state, keyboard menu and cancellation |
| Connection form and every state variant | `src/ui/connection_profile_form/`, `src/application/profile_editor.rs` | Passes 7–9, 21–25, 28–29, 37, 41; engine transitions, typed errors, real test/save outcomes, input retention, no auto-connect, scrolling, Tab/Shift-Tab, real Pinyin composition |
| Query workspace and command palette | `src/ui/query_item*`, `src/ui/command_palette.rs` | Passes 10, 26, 40–42; both appearances, search/empty search, query errors/retry, selected/current/full execution, retained earlier results, file-dialog cancellation |
| Relational grid and transactions | `src/ui/data_grid_item/`, `src/application/grid_transaction*`, `src/db/transaction.rs` | Passes 9, 19–20, 43; both appearances, supported isolation menu, real commit/rollback and transaction-consistent export; SQLite plus explicit PostgreSQL/MySQL/SQL Server tests |
| ER diagram and empty/failure/recovery | `src/ui/er_diagram_item.rs`, `src/application/er_diagram.rs` | Passes 10, 13–16, 27, 40; both appearances, PK/FK geometry, fit/drag/minimap, disabled empty controls, real locked-database failure and recovery |
| Supporting components and icons | `src/ui/assets.rs`, `src/ui/button.rs`, `icons/` | Passes 9–10, 15, 26, 30, 33, 37; pinned Zed primitives with documented local adapters for code fonts, SVG assets and accessibility |

Validation: Rust 1.97.1 build; 394 library tests and 1 MCP test passed, 12 environment-dependent tests ignored in the default suite; all-target Clippy passed with existing warnings; formatting and diff whitespace checks passed. The three explicit transaction engine tests also passed against task-owned PostgreSQL, MySQL and SQL Server containers (pass 19). Native evidence is macOS; this does not claim Windows/Linux visual verification.

The root Cargo package, root `src/` and `icons/` remain intact. Existing performance/streaming changes in the dirty worktree were preserved. No commit, push, PR or tracker closure was requested or performed. Detailed evidence and the corrections to intermediate observations are retained below; historical pending notes are not outstanding acceptance items.

## Screen coverage

All 38 screen/state variants have an implementation. Statuses below identify concrete native evidence; the contract matrix above provides the corresponding interaction acceptance. Light and dark variants share behavior but both require rendered verification. Design fixtures must not become application defaults.

| Screen | Figma node | Status |
| --- | --- | --- |
| Dark / Connected Query Workspace | [19:3](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=19-3) | Rendered; primary execution plus range/file-cancel workflows verified (passes 10, 42) |
| Light / Command Palette | [19:5](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=19-5) | Rendered; search, execution and Escape verified (pass 26) |
| Dark / Disconnected Profile Context Menu | [33:108](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=33-108) | Rendered; delete confirmation canceled (pass 12) |
| Dark / Connected Profile Context Menu | [80:365](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=80-365) | Rendered; connected actions verified (pass 26) |
| Dark / Settings Menu | [89:324](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=89-324) | Rendered; accessible settings names verified (pass 12) |
| Dark / Table Data Browser | [111:444](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=111-444) | Rendered; transaction workflow verified (passes 9, 20) |
| Dark / Table Data Browser / Tx Mode | [132:491](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=132-491) | Rendered; supported isolation and Manual controls verified (pass 9) |
| Dark / Command Palette | [139:556](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=139-556) | Rendered; New Query execution verified (pass 10) |
| Light / Connected Query Workspace | [141:655](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=141-655) | Rendered; query results verified (pass 10) |
| Light / Table Data Browser | [141:997](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=141-997) | Rendered; typography, transaction export and rollback verified (pass 20) |
| Light / Disconnected Profile Context Menu | [141:1264](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=141-1264) | Rendered; disconnected actions verified (pass 11) |
| Light / Connected Profile Context Menu | [141:1515](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=141-1515) | Rendered; connected actions verified (pass 11) |
| Light / Settings Menu | [141:1767](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=141-1767) | Rendered; language/theme changes and accessible names verified (pass 12) |
| Light / Table Data Browser / Tx Mode | [141:2019](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=141-2019) | Rendered; Auto/Manual and supported isolation verified (pass 43) |
| Dark / New Connection | [146:1093](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=146-1093) | Rendered; compact overflow and Tab navigation verified (passes 28–29); Pinyin input verified (pass 41) |
| Light / New Connection | [151:1303](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=151-1303) | Rendered; compact overflow and Tab navigation verified (passes 28–29); Pinyin input verified (pass 41) |
| Dark / New Connection / SQLite | [154:1416](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=154-1416) | Rendered; dynamic path field verified |
| Light / New Connection / SQLite | [155:1531](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=155-1531) | Rendered; dynamic path field verified |
| Dark / New Connection / Testing | [156:1521](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=156-1521) | Rendered; busy input and Escape protection verified (pass 23) |
| Dark / New Connection / Test Success | [156:1602](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=156-1602) | Rendered; test success without saving verified |
| Dark / New Connection / Validation Error | [157:1597](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=157-1597) | Rendered; required-name validation verified (pass 25) |
| Dark / New Connection / Saving | [157:1667](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=157-1667) | Rendered; real repository lock progress verified (pass 22) |
| Dark / New Connection / Save Failed | [157:1748](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=157-1748) | Rendered; English diagnostic and retained input verified (pass 29) |
| Dark / New Connection / Test Failed | [158:1712](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-1712) | Rendered; retained-input failure and cancel verified (pass 23) |
| Light / New Connection / Testing | [158:1782](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-1782) | Rendered; connection attempt progress verified (pass 24) |
| Light / New Connection / Test Success | [158:1951](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-1951) | Rendered; test success without saving verified (pass 25) |
| Light / New Connection / Validation Error | [158:2023](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-2023) | Rendered; required-field validation verified |
| Light / New Connection / Saving | [158:2095](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-2095) | Rendered; real repository lock progress verified (pass 21) |
| Light / New Connection / Save Failed | [158:2167](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-2167) | Rendered; retained input and successful retry verified (pass 21) |
| Light / New Connection / Test Failed | [158:2239](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=158-2239) | Rendered; timeout and cancel verified (pass 24) |
| Dark / ER Diagram | [173:2012](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=173-2012) | Rendered; fit, drag and relationship geometry verified |
| Light / ER Diagram | [178:2088](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=178-2088) | Rendered; fit and relationship geometry verified |
| Dark / ER Diagram / Empty | [179:2164](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=179-2164) | Rendered; empty state and disabled controls verified (passes 13, 15) |
| Dark / ER Diagram / Load Failed | [179:2384](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=179-2384) | Rendered; real lock failure and refresh recovery verified (pass 27) |
| Light / ER Diagram / Empty | [179:2604](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=179-2604) | Rendered; empty state verified (pass 13) |
| Light / ER Diagram / Load Failed | [179:2834](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=179-2834) | Rendered; failure and refresh recovery verified (passes 16, 40) |
| Dark / Startup / Empty Workspace | [185:2402](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=185-2402) | Rendered; zero-tab startup verified |
| Light / Startup / Empty Workspace | [185:2703](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=185-2703) | Rendered; zero-tab startup and 960×600 layout verified |

## Supporting design coverage

### Foundations

- Foundations / Astesia (9:2): inspected; pinned Zed primitives own production geometry and typography (pass 5).
### Components

- Components / Astesia Shell (11:2): inspected; implementation uses the pinned semantic components (pass 5).
- Icon/Chart (130:36): design context inspected in pass 26; mapped to the existing native implementation.
- Connection Form / Field (148:32): design context inspected in pass 26; mapped to the existing native implementation.
- Icon/Close (162:34): design context inspected in pass 26; mapped to the existing native implementation.
- ER Diagram / Table Node (174:32): design context inspected in pass 26; mapped to the existing native implementation.
- Icon/FitWindow (180:2392): design context inspected in pass 26; mapped to the existing native implementation.
### Utilities

### Context Menu

- Context Menu Item (79:241): inspected in pass 30; state headings describe component specimens, not application routes.
- Connection Profile Context Menu (80:16): inspected in pass 26, including Connected/Disconnected/Connecting/Disconnecting/MCP variants and blocked-action explanations.
- Context Menu / Dark · 中文 (80:17): inspected in pass 33, including keyboard and pointer contracts.
- Context Menu / Light · English (80:74): inspected in pass 33, including keyboard and pointer contracts.
- Delete confirmation / zh (80:630): inspected in pass 30; state headings describe component specimens, not application routes.
- Delete confirmation / en (80:638): inspected in pass 30; state headings describe component specimens, not application routes.
- State heading / Default (82:86): inspected in pass 30; state headings describe component specimens, not application routes.
- State heading / Hover (82:87): inspected in pass 30; state headings describe component specimens, not application routes.
- State heading / Keyboard focus (82:88): inspected in pass 30; state headings describe component specimens, not application routes.
- State heading / Disabled (82:89): inspected in pass 30; state headings describe component specimens, not application routes.
- Confirmation and recovery contract (82:90): inspected in pass 30; state headings describe component specimens, not application routes.

## Acceptance contracts

- Startup: no initial tab; centered quiet placeholder on editor background; opening work creates a tab; closing the last tab restores the placeholder. Saved profiles and actual connection status remain available. Startup errors remain distinct.
- Profile interactions: single click selects, double click connects, context menus reflect session state; delete still requires confirmation. Status remains accessible through names and tooltips.
- Connection form: example name/group/path are not defaults; engine defaults and field-reset rules follow node 158:1711. Test does not save; edits invalidate old test results; busy operations prevent duplicate submissions and closing; errors preserve inputs; save does not connect automatically.

## Verification

Initial verification (superseded by the per-frame matrix and later passes): zero-tab model tests passed (2 tests), including last-tab closure, empty navigation and reopening. `cargo test --locked` passed: 387 library tests and 1 MCP binary test, 9 environment-dependent tests ignored. `cargo clippy --locked --all-targets`, `cargo fmt -- --check` and `git diff --check` passed; Clippy reports warnings. Existing uncommitted performance changes predate this work and must be preserved.


## Handoff cross-check

The supplied Figma handoff confirms startup behavior was accepted before application implementation. Its earlier organization task does not change this implementation scope. Light Command Palette includes separate page children 19:5, 23:43, 23:110, 23:164 and 23:165; inspect their combined composition. Do not treat its outer frame alone as the complete screen.

## Initial implementation backlog (historical)

- Finish shared shell fidelity: compact sidebar rows and header, catalog hierarchy, Zed tabs and menu affordances.
- Inspect design context for every remaining screen and state before modifying its implementation.
- Finish query workspace, table data and transaction mode, command palette, profile context menus, connection forms and all state variants, and ER diagrams and recovery states.
- Verify supporting Foundations/Components/Context Menu contracts against their owning Zed primitives.
- Perform native light/dark, Chinese/English, compact/wide, keyboard and failure-path verification; attach actual evidence before closing any full-screen row.

## Implementation pass 2

- Connection Profiles now use compact Zed ListItem rows with semantic session indicators, accessible state text and tooltips. Single click selects; double click connects. Groups collapse without deleting profiles. Right-click menus keep operations bound to the clicked profile and retain existing busy/MCP/confirmation guards. Menu dismissal restores prior focus when the menu still owns it.
- Command Palette now uses Zed ListItem and ModalSurface depth. Keyboard selection scrolls into view. Profile operations appear only when enabled by the selected profile state.
- Workspace tabs use pinned Zed Tab/TabBar, preserve unsaved-change confirmation, scroll the active tab into view and allow the last tab to close.
- Connection forms put engine choice before name, put Test Connection in the footer start slot, bound the dialog to the viewport and display busy and unsaved-test-success feedback. Engine reset and test invalidation behavior have a GPUI regression test using a disposable local repository.
- Additional design contexts inspected: 33:108, 80:365, 139:556, 146:1093, 154:1416, 156:1521, 156:1602, 157:1597, 157:1667, 157:1748, 158:1712.
- Validation: 388 library tests plus 1 MCP test passed, 9 ignored. Clippy all targets, formatting and whitespace checks passed, with warnings. Native visual validation remains outstanding.

### Next fidelity gaps

- Catalog remains the incumbent hierarchy/layout; align database/schema/table/column/constraint/index disclosure and type icons with Figma without losing engine-specific actions.
- Finish connection-form dynamic endpoint labeling, code-value typography and exact native layout/feedback sizing; inspect light variants.
- Finish query and table-data toolbars/results, transaction mode, ER diagram and their failure states.
- Inspect the complete light Command Palette composition and remaining supporting design pages and variants.

## Implementation pass 3

- Inspected query workspace 19:3 and table-browser states 111:444 and 132:491.
- Query toolbar uses the specified pinned Zed glyphs with accessible names and shortcut tooltips. Explain is absent for unsupported engines. The editor and results use semantic editor surfaces; result rows have structural gridlines without alternating card backgrounds.
- Table-browser actions move into the top icon toolbar; paging moves below the grid. Change/selection controls remain reachable when relevant instead of occupying two permanent rows. The chart asset is the exact exported Figma SVG at `icons/chart.svg` (source node 130:36, instance 130:516).
- ORDER BY is editable alongside WHERE. The Application Core parses a list of column identifiers and ASC/DESC, rejects trailing SQL/expressions/unsupported ordering options, and the existing grid state validates actual column membership and duplicates. Header sorting synchronizes the text input.
- Full tests: 390 library tests and 1 MCP test passed, 9 ignored. Clippy all targets passed with warnings; formatting and whitespace checks passed.

### Transaction work remains required

The existing database adapter only exposes `execute_mutation_batch`, which commits each saved batch. It has no pinned per-grid transaction lifetime or isolation option. Implement the real manual transaction lifecycle and engine capabilities before exposing the Figma Auto/Manual and isolation controls. This is still part of the requested scope, not a completed or omitted state. Query/table result sizing and native screenshot verification also remain outstanding.

## Implementation pass 4 — Transactions

- Added owned database transactions for PostgreSQL, MySQL, SQLite and SQL Server. Owned connections never return unfinished transactions to shared pools. Isolation choices come from the engine: SQLite exposes Database Default/Serializable; the other three additionally expose Read Committed/Repeatable Read. Unsupported engines expose no transaction menu.
- GridTransaction serializes reads, mutation batches and finalization through one connection. Each batch/query uses a savepoint; a recovered statement failure preserves earlier pending batches. Driver retirement cancels and drops the owned transaction. Successful batches remain available as recovery SQL until confirmed commit/rollback.
- GridService loads and saves through the transaction and rejects a mismatched database/session target. Automatic UI saves use an owned transaction at the selected isolation; uncertain commit outcomes retain recovery SQL and block retry until the user discards/reloads.
- The table toolbar now exposes Auto/Manual, supported isolation levels, Commit/Roll Back and recovery copying. Manual Save applies a batch without committing; local edits still block navigation, while applied pending batches remain readable through the same transaction. Closing a tab with pending work retains the unsaved-change confirmation.
- Actual SQLite tests verified outside-connection invisibility, commit, rollback, failed-batch savepoint recovery, query-error recovery, retirement cleanup, recovery preservation and GridService load/save consistency. Full suite: 393 library tests and 1 MCP test passed, 9 ignored. Other transaction engines are compile-checked only; their real engine runs and native UI validation remain required.

## Implementation pass 5 — ER and design variants

- ER nodes now show qualified table names, PK/FK markers and separate column/type cells. Orthogonal links have FK-to-PK arrowheads and choose the nearer node side after dragging. Layout geometry and anchor calculations share the same header/row heights; zoom scales the text with nodes.
- ER toolbar uses the design order (refresh, zoom out, scale, zoom in, fit), the exact Figma fit SVG, runtime table/relationship counts and active target. Empty/error states disable diagram-only controls; an initial failure has centered remediation while retaining its actual error detail.
- Read all remaining light screen/state design contexts, plus the independent light Command Palette node 23:165. Some older connected-menu specimens still contain stale theme/status labels; production continues to show actual runtime state.
- Read Cover 8:2, Foundations 9:2, shell Components 11:2, and both delete confirmations 80:630/80:638. These explicitly make geometry representative and retain pinned Zed theme/density/typography ownership. The delete prompt now also states that database data remains.
- Validation before the final prompt-copy adjustment: 393 library tests and 1 MCP test passed, 9 ignored; Clippy all targets passed with warnings. Native ER interaction/screenshots, catalog hierarchy and remaining runtime verification are outstanding.

## Implementation pass 6 — Catalog and runtime context

- SQL catalog now follows database → schema → table → Columns/Constraints/Indexes, with exported Figma type icons. Table detail loads are lazy, keyed by database/session/table, protected against superseded responses and pruned when sessions disappear. Foreign keys are included in Constraints.
- Database, schema and table menus retain capability-gated creation, structure, copy, backup, restore, rename and destructive confirmation flows. Database rename/drop continue to use a separate control database. Empty auxiliary categories are hidden, with their creation commands retained in database menus. Drag-copy remains available.
- Databases expand independently. Selecting another profile no longer hides connected catalogs. Switching tabs synchronizes their database/table context without retargeting other query documents; the status bar retains the actual query context when another profile is selected.
- Pinned TreeViewItem was inspected: it has no custom type-icon slot and describes two levels. The deeper catalog therefore uses GPUI rows with pinned Label/Icon, exported type assets, semantic hover/focus, accessible tree roles and explicit keyboard activation.
- Found mise overriding ordinary cargo invocations to Rust 1.98. Re-ran with `rustup run 1.97.1`: 394 library tests and 1 MCP test passed, 12 ignored. Clippy all targets passed with warnings. Explicitly ran the 3 ignored transaction engine tests against isolated local PostgreSQL 17, MySQL 8.4 and SQL Server 2022 containers: all passed across supported isolation choices, savepoint failure, commit and rollback.
- Test containers remain running for native verification: astesia-figma-pg (127.0.0.1:55432), astesia-figma-mysql (127.0.0.1:53306), astesia-figma-mssql (127.0.0.1:51433). They were created by this task with --rm and must be stopped when verification is finished. No pre-existing containers were modified.

## Implementation pass 7 — Form values and native validation setup

- Database endpoint, port, username, database and color fields now render the same Zed editor controller with the configured buffer font. Other fields and password visibility retain the pinned InputField renderer. The adapter preserves inline validation, read-only behavior and tab order.
- SQLite now uses Database File Path and a file-path placeholder; network engines use Host and a host-address placeholder. Both update on engine changes in Chinese and English.
- Debug builds accept an absolute `ASTESIA_DEBUG_DATA_DIR` for the profile database, preferences and Zed cache. Release builds always use the platform application directory. This does not replace the native credential vault; native fixtures must use disposable accounts. Example: `ASTESIA_DEBUG_DATA_DIR=/tmp/astesia-figma-native target/debug/astesia`.
- Final pass checks on Rust 1.97.1: native build passed; 394 library tests and 1 MCP test passed (12 ignored); Clippy all targets passed with warnings; formatting and diff whitespace checks passed. Logs: `/tmp/astesia-figma-pass7-tests.log`, `/tmp/astesia-figma-pass7-clippy.log`, `/tmp/astesia-figma-native-build.log`.
- Began the first batched native pass on macOS using `/tmp/astesia-figma-native.UOMFIx`. Inspected light Chinese startup at 1280×800, opened New Connection, switched MySQL → SQLite, and invoked Cmd-Enter with empty fields. The file-path label updates, network fields disappear, both inline errors render and the save notice remains visible. Inspected that validation state at 960×600; header/footer and errors remain visible.
- Window screenshots are in the isolated directory: `startup-light-zh.png`, `form-light-zh.png`, `sqlite-light-zh.png`, `validation-light-zh.png`, `validation-compact-light-zh.png`. These are partial native evidence; theme/language counterparts, populated profiles, keyboard traversal, scrolling and the other screen states still require inspection. Accessibility inspection currently exposes the native window but no detailed GPUI controls; investigate before claiming accessibility acceptance.

### Remaining acceptance

Finish checks for the form adapter, inspect native layouts and keyboard paths in both themes/languages, validate all screen-state rows with native evidence, and remove the task-owned fixture containers. The per-frame matrix above tracks the subsequent rendered verification.

## Implementation pass 8 — Native findings

- The first native pass exposed network-form footer overflow at a 600px content height. Pinned ModalLayer places its child at `top_20`; the form now subtracts that offset and a bottom margin from `viewport_size()` before bounding its height. SQLite's shorter form already fit. The rebuilt fix still needs native confirmation.
- Profile validation now returns typed reasons instead of Chinese strings. The form translates required-name/path/host, invalid-port/color/tag and tag-count errors in Chinese or English. Connection-test and save failures now have localized headings and retain their original diagnostic details.
- Inspected actual settings menus in Chinese/light, English/light and English/dark, including changing language and theme at runtime. Captures: `settings-actual-light-zh.png`, `settings-light-en.png`, `settings-dark-en.png` in `/tmp/astesia-figma-native.UOMFIx`. Startup copy switches with the language and dark semantic surfaces render correctly at a 600px content height.
- Native automation must run the Swift source through `swift`, not the separately compiled temporary helpers: the latter did not expose/control the same accessibility state reliably. `swift /tmp/astesia-native-inspect.swift 59152` exposes named GPUI controls; `/tmp/astesia-native-press.swift` performs a targeted AXPress by observed title. The earlier empty accessibility snapshot is not evidence that the application lacks a tree. Custom settings rows still need accessibility review.
- Runtime PID 59152 still uses the pass-7 binary; pass-8 was built successfully but has not been relaunched. The first pass continues for the remaining surfaces; do not count intermediate automation captures named `palette-*` as command-palette evidence (they show the connection form). A process sample found the main thread waiting normally in the AppKit event loop.
- Final pass-8 verification: 394 library tests and 1 MCP test passed, 12 ignored; Clippy all targets passed with warnings; formatting and whitespace checks passed. Logs: `/tmp/astesia-figma-pass8-final-tests.log`, `/tmp/astesia-figma-pass8-final-clippy.log`. Native confirmation of this pass and the broader screen-state matrix remain outstanding.

## Implementation pass 9 — Connection and table runtime

- Fixed two native findings: code-value inputs now explicitly enable `tab_stop(true)`, and custom SVGs are bundled in a composite AssetSource and loaded by embedded paths. Pinned `Icon::from_external_svg` accepts a file path, not SVG markup. Zed's existing fonts/themes/icons continue through the delegated source.
- Confirmed the new build at 960×600: Tab advances Name → Host → Port → Username → Password → Database; entered the local MySQL fixture endpoint entirely through that route and Test Connection succeeded. Header/footer remain visible with the success notice. Overflow-body scrolling still needs a conclusive native check. Captures: `pass9-form-keyboard-compact.png`, `mysql-test-compact-dark-en.png`.
- SQLite fixture `/tmp/astesia-figma-native.UOMFIx/fixture.sqlite3` contains customers and orders with an FK. Native Test succeeded without saving; an unavailable parent path produced a failed test and retained input; restoring the path and Save created `SQLite Native QA` in a disconnected state. Double-click connected it; opening customers produced a data tab and expanding orders loaded Columns/Constraints/Indexes. Captures: `sqlite-test-success-dark-en.png`, `sqlite-test-failed-dark-en.png`, `table-dark-en.png`.
- Native manual transaction: changed customer 1 from Ada to Ada Pending, pressed Save Changes, observed the not-committed notice and verified an independent sqlite3 connection still saw Ada. Pressing Commit made Ada Pending visible independently. Fixture retains that committed value. `manual-save-result.png` records the applied-but-uncommitted state; `tx-menu-dark-en.png` records SQLite's supported isolation menu. Native rollback remains unverified.
- Confirmed bundled database/table/chart icons render in `icons-table-compact-dark-en.png`. Other custom glyphs and ER fit still need inspection. Current native PID is 63087, window 40387, using the final pass-9 build with dark English preferences and the customers table open. Only task-owned temporary data was changed.
- Rust 1.97.1 build, 394 library tests, 1 MCP test, Clippy all targets, formatting and whitespace checks passed (12 tests ignored; Clippy warnings remain). Logs: `/tmp/astesia-figma-pass9-tests.log`, `/tmp/astesia-figma-pass9-clippy.log`. Full Figma state coverage remains incomplete.

## Native pass 10 — ER and query

- On the current build at 960×600, opened the ER diagram from the database menu. It rendered the two SQLite tables, PK/FK columns and an orthogonal FK→PK arrow. Fit to Window changed 100% to 90% and brought both tables fully into view. Dragging orders moved its node, updated the connector and updated the minimap. Verified dark English and light Chinese appearances. Captures: `er-dark-en.png`, `er-fit-dark-en.png`, `er-fit-light-zh.png`, `er-drag-light-zh.png`.
- Opened Commands, accepted New Query with Enter, entered `SELECT id, name, email FROM customers ORDER BY id;`, and executed Run. The result contained both fixture rows and the previously committed Ada Pending value. Inspected dark English and light Chinese layouts and target status. Captures: `command-palette-dark-en.png`, `query-dark-en.png`, `query-light-zh.png`.
- These are primary-path native checks, not complete acceptance of empty/error ER, query cancellation/error, palette search/navigation, all densities, or every context-menu state. No Rust source changed in this pass, so the pass-9 checks remain the latest code verification.
- Current PID 63087/window 40387 remains running with light Chinese preferences. There are three tabs: customers data, ER and an unsaved query. ER is active after dragging orders. Temporary captures are under `/tmp/astesia-figma-native.UOMFIx`.

## Native pass 11 — Profile menus and closing tabs

- Inspected connected and disconnected profile context menus in light Chinese. Disconnect removed the catalog and invalidated the old ER session with a visible explanation. Disconnected menu contains Edit/Delete; connected menu additionally contains Disconnect. Captures: `profile-connected-light-zh.png`, `profile-disconnected-light-zh.png`.
- Opened Edit from the disconnected menu, verified the saved SQLite name/path were populated, then canceled. Found and removed the irrelevant saved-password description for an ordinary SQLite edit; conversion that removes a stored credential still retains its explanation. This copy/layout correction passed Rust 1.97.1 cargo check; native confirmation remains pending.
- Closed the data and ER tabs through their accessible close controls. Closing the unsaved query prompted for confirmation; Cancel retained the query, and a subsequent confirmed discard returned to the zero-tab workspace while preserving the profile. Captures: `unsaved-query-prompt-light-zh.png`, `last-tab-closed-light-zh.png`.
- Custom red Delete menu entries currently expose an unnamed AXMenuItem. This is a remaining accessibility finding, alongside custom settings-row naming; pointer rendering does not establish accessibility acceptance.
- Current PID 63087/window 40387 has no open tabs, with the SQLite fixture profile disconnected. It still runs pass 9; only the latest SQLite-description correction is not in that running binary. No fixture data was deleted. Dark menu counterparts and deletion confirmation coverage remain outstanding.

## Implementation pass 12 — Accessible custom menu labels

- Added identified Label-role text/value nodes to the custom red Delete entry and settings rows. AccessKit derives the owning MenuItem name from these descendants, preserving the existing visual layout and handler. Verified native AXMenuItem titles now include 删除连接…, 语言: 简体中文, 主题: 浅色, Language: English and Theme: Light.
- Invoked Delete through its accessible name, inspected the native Chinese confirmation and canceled without removing the fixture. Repeated in dark English, inspecting both the disconnected menu and confirmation; the prompt explicitly distinguishes deleting the profile from retaining the database/data. Captures: `profile-disconnected-dark-en.png`, `delete-confirm-dark-en.png`.
- Final Rust 1.97.1 build, 394 library tests, 1 MCP test and all-target Clippy passed, with 12 ignored tests and existing warnings. Logs: `/tmp/astesia-figma-pass12-build.log`, `/tmp/astesia-figma-pass12-tests.log`, `/tmp/astesia-figma-pass12-clippy.log`.
- Current native PID 64778/window 40431 uses pass 12, dark English, 1280×800, no tabs and the disconnected SQLite fixture. No deletion was confirmed. ER empty/load-failure, busy/saving/save-failure, rollback and remaining state/theme coverage are still outstanding.

## Native pass 13 — ER empty state

- Created a separate task-owned empty SQLite database at `/tmp/astesia-figma-native.UOMFIx/empty.sqlite3` and saved the profile SQLite Empty QA through the form. Connected it and opened ER from Database Actions. It shows zero tables/foreign keys and the centered no-tables explanation. Inspected dark English and light Chinese: `er-empty-dark-en.png`, `er-empty-light-zh.png`.
- Found a pinned-component accessibility discrepancy: disabled ER zoom/fit controls have their click handler filtered out and render muted, but AXEnabled remains 1. `ButtonLike::render` does not currently emit a disabled accessibility state. This remains unresolved; muted rendering is not sufficient to claim accessible disabled-state acceptance.
- Current PID 64778/window 40431 remains on pass 12, light Chinese, with the empty ER tab active. SQLite Empty QA is connected; SQLite Native QA is disconnected. Both fixture files are retained. This pass changed only fixture state and acceptance evidence, so the pass-12 code checks remain current.

## Implementation pass 14 — ER refresh failure

- Held a 45-second exclusive transaction on the task-owned empty SQLite fixture, refreshed ER, and observed the real database-locked error. The lock process exited normally and rolled back; refreshing again removed the error and recovered the empty result. Captures: `er-locked-light-zh.png`, `er-unlocked-recovered-light-zh.png`. No fixture data changed.
- Found that a failed refresh over a cached empty schema still showed the no-tables explanation. The renderer now prefers the load-failure explanation when the cached schema is empty. A nonempty prior diagram remains available with the error banner. This correction is source/test verified and still needs native confirmation; the running process remains pass 12.
- Rust 1.97.1: 394 library tests and 1 MCP test passed (12 ignored); all-target Clippy, formatting and diff whitespace checks passed. Logs: `/tmp/astesia-figma-pass14-tests.log`, `/tmp/astesia-figma-pass14-clippy.log`.

## Implementation pass 15 — Disabled ER controls

- Added a small Element adapter around the rendered pinned ButtonLike. It delegates layout, painting, identity and accessible metadata, adding AccessKit's disabled flag while keeping Zed's click filtering. Applied it to ER refresh/zoom/fit controls; external pinned source was not modified.
- Rebuilt and verified the empty ER page through native accessibility: Refresh reports AXEnabled=1; Zoom out, Zoom in and Fit to Window report AXEnabled=0. This closes the disabled-state discrepancy for the ER toolbar only; it is not a claim about all application buttons.
- Current native PID 67089 uses pass 15, including the prior failed-refresh presentation fix. An attempted repeat lock test could not target Refresh through the transient accessibility snapshot, so that attempt is not counted as failure-view confirmation. Its task-owned 25-second lock process exited normally and released the lock.
- Rust 1.97.1 build, 394 library tests, 1 MCP test, all-target Clippy, formatting and whitespace checks passed (12 ignored tests; existing warnings remain). Logs: `/tmp/astesia-figma-pass15-build.log`, `/tmp/astesia-figma-pass15-tests.log`, `/tmp/astesia-figma-pass15-clippy.log`. Broader Figma acceptance remains incomplete.

## Native pass 16 — Failed-refresh confirmation

- Revalidated the current process and window (PID 67089, window 41075), acquired an exclusive lock on the task-owned empty SQLite fixture, and invoked Refresh through AXPress. The current renderer now shows the centered Unable to load ER diagram explanation plus the database-locked diagnostic, instead of the cached no-tables message. Confirmed light Chinese in `er-failure-fixed-light-zh.png` under `/tmp/astesia-figma-native.UOMFIx`.
- The 35-second fixture lock process exited normally and released the lock. The subsequent accessibility snapshot exposed only the application node, so the attempted theme change is not counted as dark-state evidence. No source or database contents changed in this pass; pass-15 code checks remain current. Dark load-failure and final recovery confirmation remain pending.

## Implementation pass 17 — Table value typography

- Data-table column names, types, row numbers, displayed values and NULL values now explicitly use the configured Zed buffer font, matching query results and the code-value typography contract. Empty-state and ordinary control labels keep the UI font. This covers both normal and draft cell rendering.
- Rust 1.97.1 build, 394 library tests, 1 MCP test, all-target Clippy, formatting and whitespace checks passed (12 ignored tests; existing warnings remain). Logs: `/tmp/astesia-figma-pass17-build.log`, `/tmp/astesia-figma-pass17-tests.log`, `/tmp/astesia-figma-pass17-clippy.log`.
- Launched the new build as PID 68698. WindowServer lists its main window 41607, but it is not reported on screen and the accessibility snapshot exposes only the application. Do not restart based on that observation: the process remains alive. Typography native confirmation is pending. The application log is `/tmp/astesia-figma-native.UOMFIx/app-pass17.log`; fixture contents are unchanged.

## Acceptance audit 18 — Native access and export consistency

- Revalidated PID 68698: it remains alive. WindowServer reports zero visible windows for this process; AX visibility is intermittent even after requesting frontmost state. Asked the user whether the Mac is unlocked and the test window is visible. No native interaction result is inferred from this state.
- Source audit found an unresolved manual-transaction export inconsistency: `DataGridItem::start_export` exports Current from `GridSession::export_rows`, but All Matching Rows produces `ExportSource::Sql` and starts an independent export service query without the active GridTransaction. Applied-but-uncommitted changes can therefore differ across the two scopes. Resolve the transaction/read contract before claiming the table workflow fully accepted; no export behavior was changed in this audit.
- Pass-17 source checks remain current. Remaining work includes native typography confirmation, complete busy/saving/save-failure coverage, native rollback, export consistency, dark ER failure/recovery evidence and final fixture cleanup. This list supplements rather than replaces the full frame inventory and interaction acceptance requirements.

## Implementation pass 19 — Transaction-consistent streaming export

- All Matching Rows now passes the active GridTransaction to the export service. Its query runs on that owned connection rather than a new pooled connection. Current Page/Selection continues to export the captured page rows; exports without an active transaction retain the existing path.
- Added transaction streaming through the existing PostgreSQL/MySQL/SQLite/SQL Server row readers. A bounded 32-event channel relays rows to the existing export writer; stopping row acceptance still drains the database response. Export reads use a savepoint so a recoverable read failure preserves prior pending mutations. Retirement or an unusable transaction fails the export rather than switching to another connection.
- Real SQLite validation produced a CSV containing an uncommitted row while the independent connection still saw zero rows. Also verified limited row acceptance drains a 1,000-row query and a failing stream leaves the transaction usable. Explicit local PostgreSQL, MySQL and SQL Server fixture tests passed across their supported isolation settings, including uncommitted streaming and read-error recovery.
- Full library/MCP tests passed (394 + 1, 12 ignored); the 3 explicit engine tests passed. Logs: `/tmp/astesia-figma-transaction-export-tests.log`, `/tmp/astesia-figma-transaction-export-engines.log`, `/tmp/astesia-figma-pass19-tests.log`. Native export-flow confirmation is still required. The running UI remains pass 17/PID 68698 until the newer build is launched.
- Final native build, all-target Clippy, formatting and whitespace checks also passed. Logs: `/tmp/astesia-figma-pass19-build.log`, `/tmp/astesia-figma-pass19-clippy.log`. Existing warnings remain; full Figma acceptance is not yet complete.

## Native pass 20 — Transaction export and rollback

- Desktop access recovered. Launched pass 19 as PID 70934/window 42290, connected SQLite Native QA and opened customers. Confirmed the pass-17 buffer-font correction in `table-buffer-font-light-zh.png`.
- Selected Manual, edited customer 1 to Uncommitted Export QA, and pressed Save Changes. Used the native Export → CSV → All Matching Rows flow. The CSV contains the uncommitted value while an independent SQLite connection still reads Ada Pending. This confirms the UI uses the active transaction, not merely that a service test passes.
- The macOS save panel normalized an absolute path entered in the name field to a colon-containing filename in its prior directory. The task-created file was read and moved immediately into `/tmp/astesia-figma-native.UOMFIx/native-uncommitted.csv`; no artifact was left in that prior directory. The automation now excludes AXWindow titles when matching controls and searches child controls in reverse order to select a confirmation button instead of the background toolbar's identically named button.
- Invoked Roll Back, confirmed its native prompt, and verified both the grid and an independent SQLite connection returned to Ada Pending/Lin. Captures: `native-export-result.png`, `manual-rollback-light-zh.png`. The export remains as evidence of the transaction snapshot; database contents retain their prior committed values.
- Current process is pass 19/PID 70934, light Chinese, customers tab active, Manual mode with no pending changes, name descending sort. No code changed in this pass; pass-19 checks remain current. Full screen-state acceptance and fixture cleanup are still outstanding.

## Native pass 21 — Saving, failure and retry

- Opened New Connection and filled a SQLite profile named Save Failure QA. Acquired a 45-second write lock only on the task-owned profile repository (`/tmp/astesia-figma-native.UOMFIx/connections.sqlite3`), then saved through the native form.
- Captured the actual Saving state with progress notice, disabled controls and retained input (`form-saving-light-zh.png`). After the write timeout, captured the storage_busy failure with the original name/path retained and usable retry controls (`form-save-failure-light-zh.png`). Both were inspected in light Chinese at 1280×800.
- The lock process exited normally and released the lock. Retried Save, then verified the modal closed and the new profile appeared as disconnected. No database data changed. The Save Failure QA profile is retained in the isolated repository for remaining variant checks.
- Current PID/window remain 70934/42290 on pass 19. Source did not change; pass-19 checks remain current. Dark saving/failure and the rest of the full state matrix still require acceptance evidence.

## Native pass 22 — Dark saving and failure

- Repeated the task-owned repository write-lock test in dark English. Inspected the progress notice, disabled controls and retained fields in `form-saving-dark-en.png`, then the storage_busy failure in `form-save-failure-dark-en.png`. The 25-second lock process exited normally and released its lock. Canceled afterward; Dark Save QA was not saved.
- Found that storage_busy detail/remediation remained Chinese in the English form. Added a concise English explanation and retry instruction for that typed error code; other original diagnostics remain unchanged. The code passed Rust 1.97.1 cargo check, formatting and whitespace checks. This final copy correction still needs native confirmation. Log: `/tmp/astesia-figma-pass22-check.log`.
- Current PID/window remain 70934/42290 on pass 19, now dark English, with the customers tab active. The three saved task profiles remain; no database data changed. Full acceptance is still incomplete.

## Native pass 23 — Testing and busy-input protection

- Started a task-owned TCP listener bound only to 127.0.0.1 on an ephemeral port. It accepted the test connection, delayed for 15 seconds, then closed normally. Used a PostgreSQL form to target it; no credentials or profile were saved.
- Inspected dark English Testing in `form-testing-dark-en.png`. While it was still waiting, attempted to type into the focused port field and pressed Escape. The field retained its original port and the dialog stayed open (`form-testing-guard-dark-en.png`). The progress notice and unavailable controls remained visible.
- After the local listener closed, the form showed the connection-reset failure with all inputs retained (`form-delayed-test-failed-dark-en.png`). Cancel then dismissed it. The listener process is finished and its socket is closed. Current PID/window remain 70934/42290 with the customers tab active; no source changed in this pass.

## Native pass 24 — Light testing and timeout

- Inspected light Chinese Testing in `form-testing-light-zh.png`, followed by a pool-connection timeout in `form-test-result-light-zh.png`. The progress notice, retained fields and eventual failure explanation rendered inside the form. Cancel dismissed the failed test without saving a profile.
- The intended local listener timed out before accepting a connection because initial automation input was incomplete. It exited and closed its socket. Therefore this pass proves the visible connection-attempt/timeout path, not a delayed accepted-server handshake. The dark accepted-handshake path remains covered by pass 23.
- No source or saved profile changed. Current PID/window remain 70934/42290 on pass 19, now light Chinese, with the customers tab active. The pass-22 English storage-busy copy correction remains source-verified pending rebuild/native confirmation; broader acceptance and fixture cleanup remain outstanding.

## Native pass 25 — Success and validation counterparts

- Inspected light Chinese test success against the task-owned empty SQLite database (`form-test-success-light-zh.png`). It explicitly states that the connection profile has not been saved. Canceled the form, preserving the repository's existing three fixture profiles.
- Switched to dark English, opened a fresh network form and invoked Save with an empty name. Inspected the marked name field, English required-name error and fix-fields notice (`form-validation-dark-en.png`), then canceled. No connection attempt or profile save occurred for the invalid form.
- Initial automation text did not consistently land in the intended fields; corrected it against the visible form before counting results. These captures verify the stated success/validation states, not unobserved keyboard transitions. Current PID/window remain 70934/42290 on pass 19, dark English, customers tab active. No source changed in this pass.

## Native and coverage pass 26 — Command palette and evidence reconciliation

- Inspected the dark connected-profile menu and its accessible Disconnect, Edit Connection and Delete Connection actions (`profile-connected-dark-en.png`). No action was invoked in this menu.
- Used command search `theme light`; accessibility reported only Light / Theme and Return switched the theme. Reopened and inspected the light English palette (`command-palette-light-en.png`), then searched a deliberately unmatched string. The screenshot and accessibility value both report No matching commands (`command-palette-no-match-light-en.png`). Escape dismissed the palette and retained the active customers tab.
- Read the five supporting component contexts for chart, connection field, close, ER table and fit-window. Their compact icon, labelled code input and qualified PK/FK/name/type table contracts match the implementations already recorded. Reconciled all 38 screen rows with available evidence; none is now misleadingly labelled unimplemented. Full keyboard, overflow, remaining failure/recovery and final copy verification remain open.
- Rebuilt the latest source, including the English storage-busy correction, successfully: `/tmp/astesia-figma-pass26-build.log`. The running UI is still pass 19/PID 70934, light English; the new binary has not yet been launched. No fixture contents changed in this pass.

- Pass-26 full checks passed: 394 library tests plus 1 MCP test, 12 ignored; all-target Clippy, formatting and diff whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass26-tests.log`, `/tmp/astesia-figma-pass26-clippy.log`.

## Native pass 27 — Dark ER failure and recovery

- Connected SQLite Empty QA and opened its ER diagram. Held a 30-second exclusive lock on the task-owned empty SQLite fixture and refreshed through AXPress. Inspected the actual database-locked diagnostic and centered Unable to load ER diagram explanation in dark English (`er-failure-dark-en.png`).
- The lock process exited normally and released the lock. Pressed Refresh again; inspected the disappearance of the error banner and restored No tables to diagram explanation (`er-recovered-dark-en.png`). No fixture data changed. This closes the previously missing dark failure/recovery evidence.
- Resized the native window to 960×600 content and opened a fresh MySQL connection form. Empty-name Save produced an inline error and a lower notice clipped by the fixed footer. Synthetic wheel events over body margins/labels, through both global and process-targeted delivery, have not conclusively moved the viewport. The pinned Modal owns overflow_y_scroll and the scroll handle. Keep overflow acceptance open until event delivery versus actual scrolling behavior is distinguished; do not infer a fix from source alone.
- The application remains pass 19/PID 70934/window 42290, dark English, with the unsaved validation form open. No application source changed; pass-26 build/tests/Clippy checks remain current. The latest binary's English storage-busy copy is still pending native launch/confirmation.

## Implementation pass 28 — Scrollable connection form body

- Identified the overflow boundary: pinned Section renders size_full/flex_1, so its descendants can overflow without expanding the Modal scroll container's content extent. Replaced only this form's Section with an intrinsic-height, nonshrinking vertical body, retaining pinned Modal/header/footer and the same DynamicSpacing tokens.
- Built and launched the new binary in the isolated data directory (PID 75665, window 42969, session 88883). At 960×600 content, invoked empty-name validation and repeated the same negative wheel event. The body now scrolls: the engine row moves out of view and the full fix-fields notice becomes visible above the fixed footer (`form-scroll-fixed-dark-en.png`). Positive wheel returns the engine row to view. This confirms the former failure was a layout defect, not merely synthetic event delivery.
- A forward Tab then synthetic Shift-Tab attempt left focus on Host, as confirmed by subsequent disposable text appearing there (`form-reverse-tab-dark-en.png`). Reverse traversal remains unresolved; no test connection or save was attempted with that input. The form is still open.
- Rust 1.97.1 build, 394 library tests plus 1 MCP test, all-target Clippy, formatting and whitespace checks passed; 12 environment-dependent tests ignored, existing warnings remain. Logs: `/tmp/astesia-figma-pass28-build.log`, `/tmp/astesia-figma-pass28-tests.log`, `/tmp/astesia-figma-pass28-clippy.log`.
- The current app now includes the English storage-busy copy correction, which still needs its failure state exercised. The old pass-19 process was deliberately terminated after canceling its form; fixture profiles/data are retained, with all profiles disconnected after relaunch.

## Implementation pass 29 — Reverse field navigation and English save failure

- Native evidence showed Shift-Tab reaching the editor Backtab action and altering input instead of moving focus. GPUI resolves bound actions before raw key listeners. The form now captures editor Tab/Backtab actions at its owning boundary and moves focus with the existing tab order, stopping the indentation action. Raw-key interception was removed after it failed native verification.
- Rebuilt and launched the final change (PID 77386/window 43138/session 74095). Verified Name → Tab → Host → Shift-Tab → Name; subsequent text appeared only in Name and Host remained localhost (`form-reverse-tab-accepted.png`). Earlier `form-reverse-tab-fixed.png` is a failed intermediate capture and must not be used as acceptance evidence.
- Switched the unsaved form to SQLite, filled the task-owned empty database path and held a 25-second profile-repository write lock. Save failed with the now-English storage-busy reason, retry guidance and error code, preserving inputs (`form-storage-busy-english-fixed.png`). Canceled; the profile was not saved. The lock process exited normally and released the lock.
- Rust 1.97.1 final build, 394 library tests plus 1 MCP test, all-target Clippy, formatting and whitespace checks passed; 12 tests ignored, existing warnings remain. Logs: `/tmp/astesia-figma-pass29-build.log`, `/tmp/astesia-figma-pass29-tests.log`, `/tmp/astesia-figma-pass29-clippy.log`. Current app is dark English at 1280×800, no tabs, three disconnected fixture profiles.
- This closes the demonstrated reverse-navigation defect and pending English storage-busy confirmation. Remaining full acceptance includes supporting context-menu state contracts, broader keyboard/IME coverage, light recovery recheck and task fixture cleanup.

## Implementation pass 30 — Context-menu contract audit

- Inspected the Context Menu Item variants, four state headings, both confirmation specimens and the confirmation/recovery text. The contract requires transitional labels, unavailable actions during MCP usage, Cancel initial focus, Escape returning to the original row, and use/revision rechecks before deletion. Platform prompts are explicitly allowed.
- Added Connecting/Disconnecting menu labels while operations run. Previously a connecting disconnected snapshot omitted the operation row. Disabled state still comes from the shared action availability snapshot.
- MCP usage now also disables Disconnect in ProfileActions; the actual disconnect handler checks that same eligibility so alternate UI entry points cannot bypass the displayed restriction.
- Connected-profile deletion now explicitly discloses that its sessions will close, in both languages, while preserving the statement that database/data remain. The pinned macOS prompt code gives the existing two-button Delete/Cancel prompt initial keyboard focus on Cancel; no platform code was changed. Returning focus to the original profile and current usage/revision behavior still require final contract verification.
- Cargo check, full tests (394 library + 1 MCP, 12 ignored), all-target Clippy, formatting and diff whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass30-check.log`, `/tmp/astesia-figma-pass30-tests.log`, `/tmp/astesia-figma-pass30-clippy.log`. The running app remains pass 29/PID 77386 with no tabs or modal; these new menu changes await native validation.

- Pass-30 native build also completed successfully (`/tmp/astesia-figma-pass30-build.log`); launch/native menu checks remain pending.

## Implementation and native pass 31 — Delete disclosure and profile focus

- Launched pass 30 and connected SQLite Native QA. Opened Delete through the profile menu; both screenshot and AX text confirmed the new session-closure disclosure (`delete-connected-disclosure.png`). Escape canceled and retained the connected profile. No deletion was confirmed.
- Audited owning delete boundaries: ConnectionService holds the MCP lifecycle lock and checks managed ownership; the manager holds the runtime lifecycle lock, passes the expected revision to repository deletion, and closes sessions only after successful deletion. Existing managed-MCP and cross-repository usage-lease tests exercise refusal.
- Added a persistent focus handle for the selected profile row. Profile-menu dismissal now targets that row rather than the unrelated control that happened to be focused before opening the menu. Unselected rows retain their normal tab stops.
- Launched the new source as PID 79304/window 43165/session 41176, dark English at 1280×800, with three disconnected fixture profiles and no tabs. An Escape menu-dismissal attempt ended with AXFocusedUIElement reporting the window, so native row-focus acceptance is not yet established. Do not infer correct accessible focus from the new handle alone.
- Rust 1.97.1 build, 394 library tests + 1 MCP test (12 ignored), all-target Clippy, formatting and whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass31-check.log`, `/tmp/astesia-figma-pass31-build.log`, `/tmp/astesia-figma-pass31-tests.log`, `/tmp/astesia-figma-pass31-clippy.log`.

## Implementation and native pass 32 — Accessible profile focus container

- Rechecked menu opening before Escape. Releasing the synthetic right click at the menu origin sometimes invoked Edit immediately; moved its release point outside the menu and confirmed both AXMenuItem entries before dismissal. The unintended Edit form was canceled without changes.
- Added an explicit Group role and profile name to the focusable row container, which previously lacked accessible semantics despite having a focus handle. The child retains its detailed status/action accessibility label.
- Built and launched PID 80240/window 43294/session 41522. Confirmed the disconnected menu was open, then sent Escape. A later AXFocusedUIElement snapshot still reported the window; its origin had also changed from (640,320) to (1251,405) between observations. Native accessible focus return remains unproven under this changing desktop state; no further focus logic is inferred from this snapshot.
- Rust 1.97.1 build, 394 library + 1 MCP tests (12 ignored), all-target Clippy, formatting and diff whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass32-build.log`, `/tmp/astesia-figma-pass32-tests.log`, `/tmp/astesia-figma-pass32-clippy.log`. Current app uses pass 32, dark English, three disconnected fixture profiles; native position must be revalidated before coordinate actions.

## Implementation and native pass 33 — Keyboard profile menus

- Read both remaining full context-menu specimens. Their explicit keyboard contract includes Shift-F10 to open, arrows/Home/End to select, Enter to activate, and Escape to return focus. Identified that Shift-F10 had no application binding.
- Added OpenProfileMenu bound to Shift-F10 on ConnectionProfileRow. Each rendered row captures its current bounds for positioning; the keyboard menu opens below that row and uses the existing anchored container. Clicking a row now explicitly focuses the selected-row handle.
- Built and launched PID 81063/window 43304/session 90915. Clicked SQLite Native QA once, pressed Shift-F10 and confirmed the two disconnected menu items in AX. Escaped, pressed Shift-F10 again with no intervening click and confirmed the same menu reopened. Opened Delete, confirmed the prompt names SQLite Native QA, pressed Escape, then Shift-F10 again; the same menu reopened. This initially appeared to prove keyboard return, but pass 34 found missing generic menu key bindings; that earlier inference is superseded by pass-34 close/readback/reopen verification. No deletion was confirmed.
- Inspected `profile-keyboard-menu.png`; the menu is below the selected row and inside the window. Closed it with Escape. Current app is dark English, 1280×800, no tabs, three disconnected fixture profiles. Accessible AXFocusedUIElement naming remains a separate observation limitation; keyboard focus return is now established through its actual action path.
- Rust 1.97.1 native build, 394 library + 1 MCP tests (12 ignored), all-target Clippy, formatting and whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass33-build.log`, `/tmp/astesia-figma-pass33-tests.log`, `/tmp/astesia-figma-pass33-clippy.log`. All listed supporting design contexts have now been inspected. Arrow/Home/End menu navigation and remaining transient/MCP visual checks still need native evidence.

## Implementation and native pass 34 — Generic menu key bindings

- Found no bindings for the pinned ContextMenu's `menu` key context. Added Up/Down, Home/End, Left/Right submenu navigation, Enter and Escape at editor-runtime initialization. Component action handlers already existed; the standalone app did not load Zed's keymap.
- Corrected the pass-33 inference: seeing the menu again after Escape/Shift-F10 did not establish it had closed. On the new build, read AXMenu before Escape, then read the full relevant tree after Escape and confirmed no AXMenu; Shift-F10 subsequently reopened it.
- Native final build PID 81961/window 43316/session 33682: End/Enter opened Delete; Escape removed its prompt and retained SQLite Native QA. Shift-F10/Home/Enter opened Edit, verified in `profile-home-enter-edit.png`, then canceled. Also verified Home/Down/Enter opens Delete and End/Up/Enter opens Edit, canceling each. No profile was deleted or saved.
- The attempt to reopen directly after canceling Edit did not activate; selected the row explicitly before the arrow-navigation test. The menu/deletion Escape return path is established; edit-modal focus restoration is not claimed. Current app is dark English at 1280×800, no tabs or modal, three disconnected fixtures.
- Rust 1.97.1 build, 394 library + 1 MCP tests (12 ignored), all-target Clippy, formatting and whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass34-build.log`, `/tmp/astesia-figma-pass34-tests.log`, `/tmp/astesia-figma-pass34-clippy.log`. Busy/MCP native menu state coverage and the remaining full-frame acceptance audit remain open.

## Native pass 35 — Busy profile menu findings

- Added a task-owned synthetic PostgreSQL profile directly to the isolated repository, targeting a loopback-only ephemeral listener. Corrected its initial engine token from postgres to the repository's postgresql before connecting. This fixture setup is not evidence of the form-save path.
- The listener accepted the application's connection and delayed 25 seconds before closing. During Connecting, Shift-F10 exposed Connecting, Edit and Delete, all visually muted (`profile-menu-connecting.png`). AXPress on Edit did not open a form; the menu remained.
- Two concrete gaps remain: all disabled menu items report AXEnabled=1; after the listener closed and the profile changed to Connection failed, the already-open menu retained its stale Connecting label and disabled actions. Closing and reopening restored Edit/Delete. Source inspection confirms ContextMenu entries currently capture availability at construction.
- The listener finished and closed its socket. Closed the menu, removed only synthetic profile id figma-busy-menu-qa from the task repository, bumped repository revision and refreshed. The original three SQLite fixtures remain; no application source changed, so pass-34 validation remains current. Current PID/window remain 81961/43316.
- Resolve live menu-state refresh and accessible disabled semantics before claiming the busy/MCP menu contract complete. These are newly evidenced implementation gaps, not a desktop-access blocker.

## Implementation and native pass 36 — Live profile-menu state

- Added a profile-menu state snapshot of target identity, connection status, current operation, shared action eligibility and language. Rendering compares current state and rebuilds the open profile menu at its existing position only when these values change. Selecting a different target/removing it closes the old profile menu; opening database/catalog menus clears the profile-specific snapshot.
- Built and launched PID 83649/window 43332/session 72550. Created a synthetic loopback PostgreSQL fixture; the local listener accepted, delayed 15 seconds, then closed. Confirmed Connecting and unavailable actions, kept the menu open, and observed it change directly to Edit/Delete after failure. `profile-menu-live-recovered.png` shows the updated menu and failed profile; no intervening menu close/reopen occurred.
- The listener finished and closed. Closed the menu, removed only figma-busy-menu-qa, incremented repository revision and refreshed. Original three SQLite fixtures remain. Current app uses pass 36, dark English at 1280×800 with no tabs or modal.
- Rust 1.97.1 check/build, 394 library + 1 MCP tests (12 ignored), all-target Clippy, formatting and whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass36-check.log`, `/tmp/astesia-figma-pass36-build.log`, `/tmp/astesia-figma-pass36-tests.log`, `/tmp/astesia-figma-pass36-clippy.log`.
- The stale-open-menu defect is resolved. Disabled menu accessibility still reports enabled and remains open. The screenshot also shows the original connection-failure prefix in Chinese under English UI; its owning outcome presentation needs localization audit.

## Implementation and native pass 37 — Disabled menu semantics

- Profile actions that are unavailable now use nonselectable custom ContextMenu entries. Their child exposes MenuItem, an explicit name and AccessKit disabled state through the existing Element adapter. Pinned menu layout and selection behavior remain; enabled entries still use ContextMenuEntry. No dependency source was edited.
- Removed the Chinese driver-error prefix from the application connection outcome and localized Connection failed at the profile UI boundary. Form test failures retain their own existing localized heading and original diagnostic.
- Built and launched PID 84881/window 43342/session 3375. A task-owned loopback listener accepted a synthetic PostgreSQL connection and delayed 20 seconds. During Connecting, all three menu items reported AXEnabled=0; AXPress on Delete opened no confirmation. Inspected the retained layout (`profile-menu-disabled-accessible.png`). After listener closure, inspected the English Connection failed heading and original reset diagnostic (`profile-failure-english.png`).
- Listener completed and closed. Removed only figma-busy-menu-qa from the isolated repository, incremented revision and refreshed; original three SQLite profiles remain. Current app is pass 37, dark English, 1280×800, no tabs or modal.
- Rust 1.97.1 check/build, 394 library + 1 MCP tests (12 ignored), all-target Clippy, formatting and whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass37-check.log`, `/tmp/astesia-figma-pass37-build.log`, `/tmp/astesia-figma-pass37-tests.log`, `/tmp/astesia-figma-pass37-clippy.log`.
- The demonstrated disabled-menu accessibility defect and driver failure heading are resolved. MCP-specific native coverage, other remaining full-frame checks and final task-container cleanup remain open.

## Native pass 38 — Managed MCP test setup

- Opened MCP Service for an end-to-end menu fixture. The first launch selected an existing release sidecar, which does not honor debug data-directory isolation. Stopped it before issuing client requests and confirmed its process exited. Built the matching debug astesia-mcp binary, then relaunched the app so sidecar discovery selected target/debug/astesia-mcp.
- Current app PID 86087/window 43468/session 4596 uses pass-37 source. Confirmed the running service binary is the debug sidecar in `mcp-debug-setup.png`. Temporarily enabled MCP only for the task-owned SQLite Native QA profile.
- Created an authenticated local MCP session using the UI's copied configuration without printing its token; restored prior clipboard text. Initial copying needed explicit foreground activation before AXPress. Client tools/list succeeded; connect_connection for the fixture returned connected=true and app_synced=true. No database query or mutation tool was invoked.
- The native sidebar still reported the fixture disconnected without an MCP usage marker, including after explicit Refresh Connections. Connection success alone does not prove the In use by MCP menu state. Diagnose ownership/activity propagation before counting this contract as accepted.
- Called disconnect_connection and confirmed connected=false, closed_now=true and app_synced=true. Stopped the service and verified no astesia-mcp process remained; restored the fixture's mcp_enabled=0 and removed the token-bearing temporary session file. The debug client scripts retain no embedded token. The MCP Server tab remains open and stopped; Save Failure QA is locally connected from this native setup, other fixtures disconnected. No application source changed; pass-37 validation remains current. Debug sidecar build log: `/tmp/astesia-figma-pass38-sidecar-build.log`.

## Implementation and native pass 39 — MCP usage and recovery

- The row's accessible name omitted the existing MCP badge, so the prior text-only inspection could not establish absence of synchronization. Added localized In use by MCP plus the session badge to the row accessibility name and tooltip; local session status remains distinct.
- Built and launched PID 87130/window 43646/session 50592. Started the matching debug MCP sidecar and temporarily enabled only SQLite Native QA. Authenticated initialize/tools-list/connect succeeded. The row then reported Disconnected / In use by MCP (MCP 1); Edit and Delete both reported AXEnabled=0.
- Connected the same fixture locally. Its name then reported Connected / In use by MCP (MCP 1), and Disconnect, Edit, Delete all reported AXEnabled=0. Inspected `profile-menu-mcp-in-use.png`. Kept the menu open and called MCP disconnect; the badge disappeared and all three menu actions changed to AXEnabled=1 without reopening. This establishes actual MCP ownership rendering and recovery rather than inferring it from connect success.
- Used the restored Disconnect action to close the local fixture session, stopped the MCP service and confirmed no sidecar process remained. Restored mcp_enabled=0, removed the temporary token/session file and refreshed. No database contents changed; the MCP Server tab remains open and stopped.
- Rust 1.97.1 build, 394 library + 1 MCP tests (12 ignored), all-target Clippy, formatting and whitespace checks passed with existing warnings. Logs: `/tmp/astesia-figma-pass39-build.log`, `/tmp/astesia-figma-pass39-tests.log`, `/tmp/astesia-figma-pass39-clippy.log`. Remaining acceptance must now be reconciled against the complete per-frame matrix; MCP ownership propagation is no longer an unresolved finding.

## Native pass 40 — Light ER recovery and query retry

- On current pass-39 binary (PID 87130/window 43646), switched to light English, connected SQLite Empty QA and opened ER. Held a 20-second exclusive lock only on empty.sqlite3; Refresh produced the load-failure center and lock diagnostic (`er-failure-light-en-current.png`). The lock process completed and released; Refresh restored No tables to diagram and removed the error (`er-recovered-light-en-current.png`). This closes the pending light recovery check.
- Opened Query 3 and executed a missing-table SELECT, observing Failed plus the original diagnostic and retained SQL (`query-error-light-en.png`). The initial text was prepended to the default SELECT 1; the failed first statement is the result counted here. Replaced the editor content and retried SELECT with a Chinese string and 42; the grid displayed both correctly (`query-retry-unicode-light-en.png`). This verifies committed Unicode text, not IME composition.
- Executed a read-only recursive aggregate against the fixture. It completed normally in 33,117 ms and returned 5,000,000,050,000,000 (`query-long-read-light-en.png`). Current source replaces Run with a loading icon and disables execution while busy but exposes no Cancel command. No cancellation was performed or proven. Reconcile this earlier audit item with the actual design contract before treating it as required new functionality.
- No database contents or application source changed. Pass-39 build/tests/Clippy remain current. Current UI is light English with MCP Server (stopped), ER and an unsaved Query 3 tab; the query is complete and idle. SQLite Empty QA remains connected, other fixture profiles disconnected.

## Acceptance pass 41 — Scope reconciliation and IME

- Re-read the supplied handoff and current product-design/query-data/resilience contracts to distinguish explicit scope from accumulated audit suggestions. The handoff is historical Figma organization context; the active user goal is implementation. Query-data requires open/save cancellation preservation, selection/current/full execution, editor operations and IME; it does not specify a new query-cancel button. Do not add that adjacent feature merely because an earlier audit suggested testing it.
- Native Pinyin composition: opened an unsaved network form, selected the already-enabled macOS Simplified Pinyin input source, sent physical n/i/h/a/o keys and Space to commit. Inspected the resulting 你好 in Connection Name (`form-pinyin-commit.png`). Source selection and restoration both returned success. Canceled the form without saving or testing a connection. This supplies real IME composition evidence, distinct from pass-40 committed Unicode injection.
- No source or fixture data changed. Pass-39 code checks remain current. Current app PID 87130/window 43646 remains light English, Query 3 active/idle with the previous long-read text; MCP Server is stopped, ER tab remains, SQLite Empty QA connected.

### Pass-41 remaining acceptance (closed by passes 42–43)

- Verify query range selection/current-statement/full execution and file-dialog cancellation against the current editor surface; close any newly observed defect in scope.
- Reconcile supporting component and frame evidence once against the final source; retained historical pending notes below are chronological records, not a new implementation backlog.
- Inspect the final diff for unintended changes while preserving preexisting performance work; keep root Cargo/src packaging intact.
- Stop task-owned database fixture containers and the native QA process after the last needed runtime check, retaining evidence artifacts.
- Do not claim cross-platform native verification; the actual runtime evidence is macOS, with engine tests separately recorded.

## Native pass 42 — Query ranges and file cancellation

- Replaced Query 3 with three read-only statements: first=11, second=22, then a missing-table SELECT. The first synthetic Up attempt did not move the caret and executed the third statement; it is not second-statement evidence. Clicking the visible second line and Run Current Statement returned only second=22 (`query-second-statement.png`).
- Run without selection produced ordered result selectors #1, #2 and #3 Failed (`query-full-mixed-results.png`). Selecting #1 showed first=11 despite the later failure (`query-preserved-first-result.png`). Triple-click selected only the first SQL line; Run returned only first=11 with the selected range still visible (`query-selected-range.png`).
- Open on the dirty document displayed Discard and Open; proceeded to the actual native Open panel and canceled it. Opened the native Save panel and canceled it. The final screenshot preserves all three SQL statements, Query 3 identity, dirty indicator and prior results (`query-file-cancel-preserved.png`). No file was opened or written.
- No source or database contents changed; pass-39 code checks remain current. Current PID/window 87130/43646, light English, three tabs, Query 3 idle with first statement selected. SQLite Empty QA remains connected; MCP Server remains stopped. Query range and file-cancellation acceptance is now established. Remaining work is the final source/evidence reconciliation, diff audit and fixture cleanup.

## Final audit pass 43 — Transaction counterpart and scope closure

- Inspected the live light English Auto/Manual transaction menu for SQLite customers, including Database Default/Serializable and automatic-commit explanation (`tx-menu-light-en-final.png`). This resolves the last Pending frame row, 141:2019. Prior native commit/rollback/export evidence remains applicable to the shared workflow.
- Re-read current root/package diff, startup/tab model, workspace target mapping, asset source, form busy/dismiss guards, transaction actor and order parser. No TODO/unimplemented placeholders were found in the task-owned implementation modules. Confirmed Cargo/src/icons remain at root and preexisting performance work remains in the dirty worktree. This is implementation-scope reconciliation, not a claim of auditing unrelated performance changes.
- Reconciled all frame rows and supporting contexts with the actual sources and accumulated native/test evidence. The original implementation objective is covered; query cancellation was not added because it is not a specified action in the supplied design.
- Stopping the three named task database containers and the task-owned native process; final process/container readback is recorded below. Screenshots, logs and disposable fixture files remain under `/tmp/astesia-figma-native.UOMFIx` for inspection. No user database contents were modified.

- Cleanup readback confirmed no task native/MCP process and no astesia-figma containers remain. The existing ocuu-minio, ocuu-postgres, ocuu-rabbitmq and ocuu-redis containers remain running. Final formatting and git diff whitespace checks passed. The worktree remains uncommitted with the preexisting changes preserved.
