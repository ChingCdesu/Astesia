# Sidebar scroll layout — 2026-09-06

The release sidebar was CPU-bound in nested tree layout. A 15-second macOS Time Profiler
recording during user-driven scrolling attributed 98.1% of main-thread CPU sample weight to
Taffy layout stacks, with recursive flex layout as the main hotspot.

The connection, database, schema, table, and detail containers that only stack descendants now
use block flow. Horizontal rows retain Flex, and the catalog's spacing, controls, callbacks,
selection state, disclosure state, and scroll ownership remain in place. The change is confined
to `src/ui/connections/view.rs` and `src/ui/connections/catalog_tree.rs`.

## Native comparison

Both captures attached to `target/release/astesia` for 15 seconds. The user restored the same
expanded sidebar state and scrolled continuously. Background Clippy compilation was suspended
during the second capture and resumed afterwards.

| CPU sample weight | Before (ms) | After (ms) | Reduction |
| --- | ---: | ---: | ---: |
| Main thread | 15,065 | 5,708 | 62.1% |
| Inclusive Taffy layout | 14,786 | 3,771 | 74.5% |
| Inclusive window draw | 14,998 | 5,365 | 64.2% |

These are statistical CPU sample weights, not frame durations or measured FPS. Inclusive
categories overlap. Manual gestures were not replayed deterministically, so the percentages
are an observed comparison rather than a controlled benchmark. The user reported noticeably
smoother scrolling and no observed anomalies after the change. This verifies that macOS
scenario; other platforms, every engine, theme, density, and keyboard path were not exercised.

At this first stage the sidebar still laid out all expanded nodes. The follow-up below replaces
that remaining eager layout path.

## Evidence

- Before trace: `/tmp/astesia-sidebar-scroll-6453.trace`.
- After trace: `/tmp/astesia-sidebar-scroll-8242.trace`.
- Exported sample tables: `/tmp/astesia-sidebar-profile.xml` and
  `/tmp/astesia-sidebar-profile-after.xml`.
- Ignored local artifacts: `.scratch/sidebar-scroll/`, including source snapshots, the original
  executable, the profile summarizer, aggregate JSON, and build/check logs.
- Original executable SHA-256:
  `0514cf2717c249504f109f0b98ec8b19e566faab23f1ebed0ebf61e4fe4dcacb`.
- Measured updated executable SHA-256:
  `ffde1c886e8e1b67361449c4f0c41d8efa4d6a095dd7114bf82335f85575df0d`.

The release build passed. Release tests passed (394 library tests and one MCP test; 12 ignored).
Release Clippy across all targets, formatting, and whitespace checks passed. Existing warnings remain. The native before/after
capture is the regression check for this interaction; the regular tests do not measure FPS.


## Virtualized tree follow-up

The sidebar now flattens expanded profiles, databases, schemas, tables, details, and secondary
catalog objects into cached row descriptions. GPUI ListState constructs only visible rows and
a small overdraw region. The variable-height list keeps wrapped errors, retry controls, and
Redis search in the same scroll area without imposing the tree row height on them. Tree rows
retain the pinned Zed Project Panel ListItem composition.

ListState notifies its owning entity during scrolling. Invalidating on every entity notification
therefore rebuilt the entire flattened model on each wheel event. The regression test caught
this: the model build count increased from one to two after one wheel event. Row invalidation
now belongs to panel data, expansion, session, and settings changes through notify_sidebar;
scroll notifications reuse the cached model. Selection and context-menu callbacks read current
panel state when visible rows render.

The list retains the first visible row by identity when rebuilding. If that row disappears,
it anchors to the nearest preceding surviving row, including a collapsed parent.

The GPUI test expanded_sidebar_only_renders_visible_rows uses the real sidebar and a disposable,
in-memory-backed catalog of 10,000 PostgreSQL tables. It verifies:

- The first viewport creates fewer than 200 rows instead of all 10,000.
- A dispatched wheel event moves the list without rebuilding the flattened model.
- Scrolling to row 5,000 creates fewer than 200 rows and reaches the requested range.
- Collapsing the schema removes its descendants and keeps the scroll offset in bounds.

Run with cargo +1.97.1 test --locked expanded_sidebar_only_renders_visible_rows.
The test is a rendering-work regression, not an FPS benchmark. No new physical-window scrolling
capture was collected for this follow-up; the earlier 74.5% measurement applies only to the
first block-layout change. Cross-platform rendering, actual menu gestures, themes, and density
combinations remain unverified for the virtualized version.

Follow-up validation: 395 library tests and one MCP test passed; 12 tests remain ignored.
Clippy across all targets, rustfmt, and git diff --check passed with existing compiler/lint
warnings. The macOS release binary built successfully with Rust 1.97.1.

## First-expansion measurement retention

A follow-up GPUI test exposed that rebuilding the row model called ListState::reset even when
only descendants had changed. Before the next paint, an existing database row's bounds changed
from x=0, y=80, width=271, height=24 to unmeasured. This is evidence of discarded layout state;
it is not a recording of the user's visible shake.

Row reconciliation now preserves the matching prefix and suffix and splices only the changed
interval. A model refresh with unchanged row identities leaves the list measurements intact.
Loading/error rows include their content state in their identity so changes in wrapped message
height still get measured. Settings changes explicitly remeasure while retaining scroll position.

The first_catalog_load_preserves_parent_measurement GPUI test fails on the reset implementation
and passes on the incremental implementation. It checks both the pre-paint measurement and the
post-layout database position on first expansion and after each catalog section completes.
The 10,000-table wheel/virtualization regression also passes. Physical-window confirmation of
first expansion remains outstanding.

Validation for measurement retention: 396 library tests and one MCP test passed; 12 tests
remain ignored. Clippy across all targets, formatting, whitespace checks, and the macOS release
build completed successfully with Rust 1.97.1; existing warnings remain.
