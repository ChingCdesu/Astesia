---
name: Astesia
description: A native GPUI Kit workspace for database operations
colors:
  mysql: "#00758F"
  postgresql: "#336791"
  sqlite: "#003B57"
  sql-server: "#CC2927"
  mongodb: "#47A248"
  redis: "#DC382D"
  clickhouse: "#FFCC01"
---

# Astesia design system

Astesia is a compact, keyboard-driven database workspace. GPUI Kit owns visual primitives and
interaction behavior. Astesia owns the sidebar, tabs, work area, and persistent status composition,
engine capability rules, and bilingual product language.

Use Cargo.lock as the component version authority. Inspect installed component source when APIs
or interaction behavior matter. ADR-0002 supersedes the Zed UI dependency requirement; historical
acceptance documents remain behavioral evidence.

## Components and tokens

Use GPUI Kit Editor for source text, Input for single-line fields, and Kit components for buttons,
menus, rows, tabs, title bars, and dialog content. Reusable controls in `src/ui/components/` map
Astesia product roles onto Kit behavior and the active theme. Keep wrappers restricted to shared
product conventions; use a component directly when no application convention is required.

The light and dark palettes in `src/ui/theme.rs` follow the [Figma Screens](https://www.figma.com/design/okmpEaSEEhS2uX8VkuEhEY/Astesia?node-id=7-8) colors. Kit background is the editor/data canvas; sidebar is the persistent surface; title-bar color is the workspace/status chrome. Both configs are installed before applying the saved appearance so live system changes retain the same palette.

The active theme owns foreground, background, border, focus, selection, status, and typography.
UI text, including navigation and actions, uses the bundled Geist Mono family. Chinese text uses
platform glyph fallback. Queries, identifiers, and code-like values retain the editor monospace font. Engine colors identify database types and never substitute for action or error semantics.
State also needs readable text or an icon.

## Workspace and data surfaces

Keep the sidebar, active tab, work area, and runtime status legible. The window minimum remains
960 by 600 pixels. Long bilingual content scrolls or truncates within its owning region rather
than displacing actions or changing the shell hierarchy.

The data filter follows Figma node `132:641`: a 34px row with equal-width WHERE and ORDER BY
controls, 11px text, 14px icons, and 8px horizontal padding inside each label and input region.
Enter applies both fields; clearing a field and pressing Enter removes that condition. Keep the
input regions flush with their labels and use the editor background without inset input borders.

Custom GPUI drawing remains appropriate for result grids, charts, and ER diagrams. The catalog's
variable-height virtual list retains inline loading/error and Redis input rows while using Kit
list controls. Preserve scroll anchoring when asynchronous children appear or disappear.

The sidebar uses Kit's horizontal resize handle at its right edge. Its default width is 272px,
with a 200–560px range and at least 400px reserved for the work area. Hiding and showing the
sidebar preserves its width within the current workspace; the width is not saved across restarts.

The title bar uses Kit's platform controls. Theme selection supports light, dark, and live system
appearance. Form content scrolls independently of its persistent action footer.

## Local schema snapshots

Successful SQL catalog introspection is persisted beside the connection repository in
`connections.schema-cache.sqlite3`. Database lists, schemas, tables, columns, indexes,
constraints, foreign keys, views, routines, triggers, and enum values are reused across sessions
without expiry. Missing entries are loaded on demand. The sidebar, table structure view, and SQL
completion share these snapshots; row queries and mutation validation remain live.

The cache is scoped by connection profile revision and database. Explicit sidebar/database or
structure refresh invalidates the corresponding snapshot and completion memory; existing DDL
completion refreshes use the same boundary. Connection changes select a new scope. Failed loads
are not saved, and requests that started before invalidation cannot restore stale entries.
Redis keys, MongoDB documents, user accounts, and in-memory SQLite databases are not persisted in
this schema cache. Unreadable cache storage falls back to live metadata and records a warning.

## Interaction and recovery

Sidebar items execute their primary click action once per click sequence. Double-clicking has
the same result as a single click; connection establishment uses the explicit connection action.

- Preserve keyboard order, focus return, shortcut precedence, and Chinese IME composition.
- Query execution respects selection/current-statement/full-document scope. Search fields and
  completion menus own their input and confirmation shortcuts.
- SQL completion uses the clause before the cursor: relation positions offer tables and schemas;
  expressions offer functions, keywords, and columns from referenced tables. Alias-qualified column
  completion resolves the alias to its table. Unrelated cached columns do not enter the candidate list.
- Dirty queries and staged grid mutations require an explicit discard choice before navigation.
- Forms preserve failed input, identify validation errors, and cannot dismiss during execution.
- Destructive confirmations identify the qualified object and durable consequence.
- Per-node failures preserve siblings; completed operations refresh the owning catalog.
- Empty, loading, failed, stale, cancelled, and partial outcomes remain distinct.

Verify native surfaces in Chinese and English, light/dark appearances, and compact/wide windows.
Source checks establish API ownership; rendered and interaction claims require a running app.
