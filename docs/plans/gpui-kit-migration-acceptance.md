# GPUI Kit migration acceptance

The approved scope is the complete desktop UI migration with existing product behavior
preserved. Rust is pinned to 1.98.0 at the user's request. The runtime is `gpui-pre 0.3.3`
with `gpui-kit 0.6.0`, including its SQL grammar feature. See
[ADR-0002](../adr/0002-use-gpui-kit-for-the-desktop-shell.md) for ownership boundaries.

## Implementation

The editor, single-line inputs, forms, dialogs, menus, theme, title bar, tab bar, and
list controls use GPUI Kit. Astesia retains its application workspace, tab lifecycle,
guarded dismissal, SQL completion service, and database workflows. Text-value observation
keeps programmatic and accessibility edits synchronized with dirty state and search matches.
The find/replace toolbar owns replacement text, focus, and shortcut precedence. Its
Cmd/Ctrl+Enter handler is scoped to the toolbar. Completion acceptance consumes Enter/Tab
before they can insert another character.

The catalog retains its variable-height GPUI virtual list and uses Kit ListItem controls.
Kit Tree uses uniform rows, which do not preserve existing inline error and Redis search
field layouts. This is an intentional composition boundary, not a remaining Zed dependency.

Compared with the workspace snapshot taken immediately before migration, non-UI Rust
sources are unchanged. Existing uncommitted application and database work was preserved.

## Dependency evidence

| Lockfile measure | Before | After |
| --- | ---: | ---: |
| Packages | 1257 | 1045 |
| Packages sourced from the Zed Git repository | 110 | 0 |

The application declares only `gpui-kit` and uses its API re-exports, including test support.
`gpui-pre` remains a transitive dependency. There is one GPUI runtime version. This establishes dependency removal, not a measured
compilation speedup. No equivalent cold-build comparison was performed.

## Verification

- Rust 1.98.0: `cargo test --locked`, `cargo clippy --locked --all-targets`,
  `cargo build --locked --bins`, formatting, and whitespace checks.
  All completed successfully: 399 library tests and one MCP binary test passed; 12
  environment-dependent tests were ignored.
- Tests cover editor composition and grouped undo, completion filtering/acceptance/dismissal,
  find/replace focus and query-shortcut isolation, programmatic search updates, guarded
  modal dismissal, and keyboard menu activation alongside existing application tests.
- macOS native QA used an isolated Astesia data directory and a disposable SQLite database:
  profile creation/test/save/connect, catalog navigation, query execution, two-row results
  including Chinese text, table data, dirty-query close cancellation, and settings menus.
- Light/English and dark/Chinese theme/locale combinations were exercised. Native search
  replacement was exercised through accessibility-set text and the replace-all button,
  then Cmd+Enter and a single Cmd+Z: both `name` occurrences became `id`, and one undo
  restored the complete original query. Plain Enter in the replacement field replaced only
  the current match. The connection form also fit a 960×640 window,
  keeping its footer visible.
- The product-design skill validates and its local Markdown references resolve.

Existing unused/dead-code lint warnings remain. The dependency `block 0.1.6` also emits a
future-compatibility warning. External database engine checks remain ignored unless their
environments are explicitly supplied. Windows/Linux runtime behavior and a complete OS
Chinese input-source session were not exercised; the composition test uses GPUI's input
handler. These checks do not establish runtime parity for every external engine.

## Native screenshots

- [Dark theme, Chinese, SQLite results](gpui-kit-migration/dark-chinese.png)
- [Light theme, English, SQLite results](gpui-kit-migration/light-english.png)
- [Connection form at 960×640](gpui-kit-migration/connection-form.png)
