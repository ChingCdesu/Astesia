---
name: Astesia
description: A Zed-native workspace for database operations
colors:
  mysql: "#00758F"
  postgresql: "#336791"
  sqlite: "#003B57"
  sql-server: "#CC2927"
  mongodb: "#47A248"
  redis: "#DC382D"
  clickhouse: "#FFCC01"
typography:
  ui:
    fontFamily: '".ZedSans", "IBM Plex Sans", system-ui, sans-serif'
  buffer:
    fontFamily: '".ZedMono", "Lilex", monospace'
---

# Design System: Astesia

## Overview

**Creative North Star: "A Zed Workspace for Databases"**

Astesia should feel like a focused database workflow that belongs inside Zed: native, compact,
keyboard-first, theme-aware, and built from the same component behavior rather than a visual copy of
one Zed screenshot. The pinned Zed UI source owns components, typography, density, semantic colors,
elevation, focus, and interaction states.

Astesia owns the database workspace composition, engine capabilities, bilingual product language,
and the small set of engine identity colors. Query text, results, and runtime state remain the
dominant material.

**Key Characteristics:**

- Zed UI components and semantic theme roles form the visual foundation.
- Compact multi-pane geometry keeps Connection Profiles, tabs, work, and status continuously
  visible.
- UI scale and Compact, Default, or Comfortable density come from Zed rather than fixed local pixels.
- Engine color identifies database type; Zed semantic roles communicate actions and state.

## Colors

Runtime color comes from the active Zed theme. Astesia does not define its own light/dark surface
palette or freeze One Light and One Dark values into application code.

### Primary

- **Zed semantic roles:** use `background`, `surface_background`, `editor_background`,
  `elevated_surface_background`, element and ghost-element states, border and focus roles, and the
  text hierarchy from `cx.theme().colors()`.

### Secondary

- **Zed status roles:** use `cx.theme().status()`, `zed_ui::Color`, and `TintColor` for info,
  success, warning, error, disabled, selected, modified, and other semantic states.

### Tertiary

- **Engine identity set:** MySQL, PostgreSQL, SQLite, SQL Server, MongoDB, Redis, and ClickHouse use
  the frontmatter colors only where the interface identifies an engine or Connection Profile.

### Named Rules

**The Zed-Is-The-System Rule.** Use the pinned Zed theme role or component API, not a copied hex,
radius, shadow, or state color. Reinspect the exact pinned revision when Zed changes.

**The Identity-Is-Not-Action Rule.** Engine color identifies database type. Buttons, focus,
selection, validation, success, warning, and failure continue to use Zed semantics.

**The State-Is-Written Rule.** Connection and error state includes text or an icon; color never
carries the only meaning.

## Typography

- **Display Font:** none in the routine operator workspace.
- **UI Font:** the Zed UI font and user-configured UI scale.
- **Buffer Font:** the Zed buffer font and user-configured buffer size for query editing and
  code-like values.

Use `Label`, `Headline`, `LabelSize`, and `TextSize` rather than local font sizes. Default UI text is
the ordinary control and row voice; Small and XSmall carry dense metadata; Large or Headline sizes
belong only where the Zed component hierarchy calls for them. Weight, semantic color, and placement
create hierarchy before size does.

**The Buffer Owns Mono Rule.** Monospace belongs to editable queries and code-like values. General
navigation, actions, and status use the Zed UI font.

## Layout

The application keeps an Astesia-owned horizontal composition: a Connection Profile and catalog
sidebar beside a flexible tabbed work area, with persistent runtime status. The supported window
floor remains 960 by 600 pixels; growing content scrolls or truncates within its owning pane.

Use `DynamicSpacing` for component gaps, padding, and insets so Zed's Compact, Default, and
Comfortable densities remain coherent. Fixed geometry is reserved for product boundaries such as
the window minimum, sidebar behavior, result-grid columns, chart canvases, and other data surfaces
that cannot be expressed by a Zed spacing role.

The active editor and results absorb available space. Overlays remain bounded, lists and grids own
their scrolling, and long bilingual content never pushes the primary action out of reach.

**The Continuous-State Rule.** Hiding the sidebar may enlarge the work area, but the active tab and
runtime context stay visible.

## Elevation & Depth

Use `ElevationIndex` as the complete depth vocabulary:

- **Background:** application background below panes and panels.
- **Surface:** persistent panes, panels, and containers.
- **EditorSurface:** editable buffers and work surfaces.
- **ElevatedSurface:** floating content below dialogs.
- **ModalSurface:** dialogs, alerts, and modal work.

Use the elevation's background and shadow. Persistent Background, Surface, and EditorSurface layers
stay flat; floating and modal content receive the Zed-provided depth treatment. Astesia does not
maintain a parallel shadow scale.

## Shapes

Component shape belongs to Zed. Use the component's default rounding, border, clipping, and grouped
edge behavior. Compose raw GPUI geometry only when no suitable Zed primitive exists, and derive its
surface, borders, focus, spacing, and adjacent alignment from the surrounding Zed components.

Persistent pane boundaries remain structural. Engine and status marks may be circular, but their
shape does not replace a text or accessibility label.

## Components

### Buttons

- Use `Button` or `IconButton`; use `ButtonLike` only for a composition a standard button cannot
  express.
- `ButtonStyle::Subtle` is the common action, Filled adds emphasis, Outlined is secondary,
  Transparent is quiet, and Tinted carries semantic Accent, Error, Warning, or Success state.
- Use `ButtonSize`; Compact is appropriate for dense toolbars, while forms and dialogs use the size
  owned by their surrounding Zed component.
- Use built-in loading, selected, disabled, tooltip, key-binding, focus, and accessibility behavior.

### Labels and status

- Use `Label`, `LabelSize`, and semantic `Color` values.
- Use `Indicator` or a status icon with readable text for connection, task, and failure state.
- Truncate only dynamic text; static action labels remain complete.

### Lists, trees, and navigation

- Use `ListItem`, `TreeViewItem`, `Tab`, and `TabBar` behavior where their contracts fit.
- Selection, disclosure, indentation, density, end-slot visibility, hover, focus, and accessible
  roles come from the component instead of local replicas.
- Astesia owns the profile/catalog hierarchy and tab content, not a second navigation style.

### Inputs and forms

- Use Zed input, checkbox, toggle, section, modal, header, and footer components.
- Validation preserves input, identifies the field, and uses semantic error treatment with readable
  remediation.
- Destructive confirmation names the exact database object and consequence; button styling follows
  Zed semantics.

### Overlays and feedback

- Use Zed modal, popover, context-menu, tooltip, notification, and progress patterns where available.
- Match persistence to importance: transient confirmation may disappear; partial, failed, or
  uncertain durable work remains inspectable.
- Custom command, task, or notification surfaces use the appropriate Zed elevation and theme roles.

### Data-specific surfaces

Query grids, charts, ER diagrams, MongoDB documents, Redis values, and performance views may require
custom GPUI rendering. They still inherit Zed typography, density, semantic colors, focus,
selection, disabled states, tooltips, and accessibility behavior.

## Do's and Don'ts

### Do:

- **Do** inspect the exact Zed component in the pinned checkout before changing or creating a visual
  primitive.
- **Do** use Zed components and semantic APIs so themes, UI scale, density, state, and accessibility
  remain live.
- **Do** preserve Astesia's compact sidebar–tabs–work–status composition and internal scrolling.
- **Do** use engine identity colors only where database type is being communicated.
- **Do** verify light/dark appearance and Compact/Default/Comfortable density for material changes.

### Don't:

- **Don't** reproduce Zed with hard-coded hex values, pixel dimensions, radii, shadows, or hover and
  focus colors.
- **Don't** create a custom component when the pinned Zed UI library already owns the interaction.
- **Don't** introduce marketing typography, decorative gradients, glass effects, or card-stack
  layouts into the operator workspace.
- **Don't** turn engine identity colors into action or status semantics.
- **Don't** claim Zed conformity from source inspection alone; verify the rendered native surface.
