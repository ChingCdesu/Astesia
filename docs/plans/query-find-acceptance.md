# Query find acceptance — 2026-09-15

Implemented the approved editor and result find designs linked from DESIGN.md.
The result bar operates independently of the editor's existing find/replace session.

## Automated validation

- `cargo test --locked`: 421 library tests and 1 binary test passed; 15
  environment-dependent library tests remained ignored.
- `cargo clippy --locked --all-targets`: passed with existing repository warnings.
- `cargo fmt -- --check` and `git diff --check`: passed.
- Added result-search coverage for UTF-8 byte ranges, case sensitivity, literal
  matching, NULL/JSON display values, match ordering, focus routing, circular
  navigation, independent editor state, result switching, and Escape focus return.
- Existing editor replacement/grouped-undo and result memory-release tests passed.

## Native validation

macOS debug build, isolated `ASTESIA_DEBUG_DATA_DIR`, disposable SQLite database
with three rows containing `Mira`, `Samira`, and `你好mira`.

- Opened editor find via Cmd+F and entered SELECT; observed the input, count,
  navigation controls, and highlighted SQL in the dark theme.
- Executed a read-only SELECT against the fixture, opened result find, entered
  mira, and used Enter to reach match 2/3. Observed the current-cell outline and
  distinct text highlights while all three rows remained present.
- Entered missing and pressed Enter. Observed No matches and disabled navigation,
  with the result table retained.
- Inspected normal and maximized windows. Window resizing was needed to refresh
  computer-use screenshots; accessibility state reflected actions immediately.
- Closed the isolated test application after validation. The installed application
  and its profiles were not modified.

Light appearance was exercised by GPUI tests, but not visually inspected in a native
window. Windows/Linux, real-engine integration beyond SQLite, physical IME input,
and the 960px minimum window have not been manually verified in this change.

## Text selection contrast follow-up

The theme regression assertion first reproduced the low-visibility dark selection
(gray at alpha 0.3). After assigning a dedicated opaque blue selection and syncing
Kit/input tokens, all 421 library tests passed with 15 ignored. Clippy, formatting,
and diff checks passed. In the rebuilt macOS debug window, selected SQL and selected
username text both rendered with a visible blue background. Used only default form
values in the isolated test profile; no connection was saved or tested in this pass.
Light/dark theme switching is regression-tested; light-mode visual QA remains pending.

## Selected engine button follow-up

The engine selector uses Filled + selected, which resolves
`button_primary_active`, not `button_primary`. Explicitly configured primary hover
and active surfaces to prevent fallback to Kit's pale primary-active background.
Theme-switch regression assertions and all 421 library tests passed, with 15 ignored;
formatting and diff checks passed. The native test application was rebuilt, but
computer-use clicks did not open the connection dialog in this run, so the updated
engine selector has not been visually verified. Closed the isolated test process.
