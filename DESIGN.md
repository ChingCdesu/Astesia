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

The active theme owns foreground, background, border, focus, selection, status, and typography.
Monospace belongs to queries, identifiers, and code-like values. Navigation and actions use the UI
font. Engine colors identify database types and never substitute for action or error semantics.
State also needs readable text or an icon.

## Workspace and data surfaces

Keep the sidebar, active tab, work area, and runtime status legible. The window minimum remains
960 by 600 pixels. Long bilingual content scrolls or truncates within its owning region rather
than displacing actions or changing the shell hierarchy.

Custom GPUI drawing remains appropriate for result grids, charts, and ER diagrams. The catalog's
variable-height virtual list retains inline loading/error and Redis input rows while using Kit
list controls. Preserve scroll anchoring when asynchronous children appear or disappear.

The title bar uses Kit's platform controls. Theme selection supports light, dark, and live system
appearance. Form content scrolls independently of its persistent action footer.

## Interaction and recovery

- Preserve keyboard order, focus return, shortcut precedence, and Chinese IME composition.
- Query execution respects selection/current-statement/full-document scope. Search fields and
  completion menus own their input and confirmation shortcuts.
- Dirty queries and staged grid mutations require an explicit discard choice before navigation.
- Forms preserve failed input, identify validation errors, and cannot dismiss during execution.
- Destructive confirmations identify the qualified object and durable consequence.
- Per-node failures preserve siblings; completed operations refresh the owning catalog.
- Empty, loading, failed, stale, cancelled, and partial outcomes remain distinct.

Verify native surfaces in Chinese and English, light/dark appearances, and compact/wide windows.
Source checks establish API ownership; rendered and interaction claims require a running app.
