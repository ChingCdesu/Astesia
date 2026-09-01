---
name: Astesia
description: Compact native database operations workspace
colors:
  light-focus: "#7d82e8ff"
  light-workspace: "#dcdcddff"
  light-panel: "#ebebecff"
  light-editor: "#fafafaff"
  light-border: "#c9c9caff"
  light-text: "#242529ff"
  light-muted: "#58585aff"
  dark-focus: "#47679eff"
  dark-workspace: "#3b414dff"
  dark-panel: "#2f343eff"
  dark-editor: "#282c33ff"
  dark-border: "#464b57ff"
  dark-text: "#dce0e5ff"
  dark-muted: "#a9afbcff"
  connected: "#22c55e"
  busy: "#eab308"
  attention: "#ef4444"
  idle: "#a1a1aa"
  mysql: "#00758F"
  postgresql: "#336791"
  sqlite: "#003B57"
  sql-server: "#CC2927"
  mongodb: "#47A248"
  redis: "#DC382D"
  clickhouse: "#FFCC01"
typography:
  title:
    fontFamily: '".ZedSans", "IBM Plex Sans", system-ui, sans-serif'
    fontSize: "12px"
    fontWeight: 600
  body:
    fontFamily: '".ZedSans", "IBM Plex Sans", system-ui, sans-serif'
    fontSize: "12px"
    fontWeight: 400
  label:
    fontFamily: '".ZedSans", "IBM Plex Sans", system-ui, sans-serif'
    fontSize: "10px"
    fontWeight: 400
  editor:
    fontFamily: '".ZedMono", "Lilex", monospace'
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.618
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  full: "9999px"
spacing:
  xxs: "2px"
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
components:
  button-filled:
    backgroundColor: "{colors.light-panel}"
    textColor: "{colors.light-text}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 4px"
    height: "18px"
  button-outlined:
    backgroundColor: "{colors.light-panel}"
    textColor: "{colors.light-text}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: "0 4px"
    height: "18px"
  input-field:
    backgroundColor: "{colors.light-editor}"
    textColor: "{colors.light-text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "6px 8px"
    height: "32px"
  connection-profile-row:
    backgroundColor: "{colors.light-panel}"
    textColor: "{colors.light-text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "8px 10px"
  query-tab-active:
    backgroundColor: "{colors.light-editor}"
    textColor: "{colors.light-text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 8px"
    height: "36px"
  command-palette:
    backgroundColor: "{colors.light-panel}"
    textColor: "{colors.light-text}"
    rounded: "{rounded.lg}"
    width: "560px"
  transient-notification:
    backgroundColor: "{colors.light-panel}"
    textColor: "{colors.light-text}"
    typography: "{typography.label}"
    rounded: "{rounded.md}"
    padding: "8px"
    width: "360px"
---

# Design System: Astesia

## Overview

**Creative North Star: "The Native Operator Console"**

Astesia is a compact, neutral desktop workspace built for sustained database operation. Its visual system keeps connections, query context, results, and runtime state continuously visible while allowing the native editor and data to remain the dominant material.

The shell follows the operating system by default through the paired One Light and One Dark themes. It uses familiar desktop controls, shallow tonal separation, small labels, and dense toolbars instead of decorative branding or oversized presentation typography.

**Key Characteristics:**

- Compact multi-pane geometry with a fixed connection sidebar, tabbed work area, and persistent status bar.
- Theme-semantic surfaces and text, with color reserved for focus, engine identity, and meaningful status.
- Native Zed UI controls and a local Zed editor, with Simplified Chinese and English occupying the same visual hierarchy.
- Borders and tonal layers establish structure; elevation is reserved for temporary overlays.

## Colors

The palette is restrained in both system modes: closely related neutral surfaces carry the workspace, one cool focus color marks interaction, and semantic colors communicate engine or runtime identity.

### Primary

- **Light Focus Indigo:** the light-mode keyboard focus and interaction accent.
- **Dark Focus Blue:** the dark-mode counterpart, tuned for the darker neutral field.

### Secondary

- **Engine Identity Set:** MySQL, PostgreSQL, SQLite, SQL Server, MongoDB, Redis, and ClickHouse retain their extracted engine colors on profile identity marks. These colors identify an engine; they do not become general-purpose accents.

### Neutral

- **Workspace:** the outer application and status surface in each mode.
- **Panel:** sidebars, toolbars, inactive tabs, and result headers.
- **Editor:** the active query tab and editor canvas, the lightest light-mode or deepest dark-mode working surface.
- **Border:** one-pixel pane, row, field, and toolbar separators.
- **Text and Muted Text:** primary operator content and secondary metadata.

### Named Rules

**The Semantic Surface Rule.** New shell surfaces use the active theme's workspace, panel, editor, border, text, and muted roles; they do not introduce an independent gray palette.

**The Identity-Is-Not-Action Rule.** Engine colors identify Connection Profiles and database types. Actions continue to use native control states and the focus color.

**The State-Is-Written Rule.** Connection and error state always includes text or an icon; the green, amber, red, and gray status dots never carry meaning alone.

## Typography

**Display Font:** none; this operator shell has no display tier.
**Body Font:** `.ZedSans`, currently backed by IBM Plex Sans, with system sans-serif fallbacks.
**Label/Mono Font:** `.ZedMono`, currently backed by Lilex, for query editing.

**Character:** The UI uses a workhorse sans serif at compact sizes, while the embedded editor owns its monospace typography. Weight, muted color, and position create hierarchy instead of large jumps in size.

### Hierarchy

- **Title** (semibold, 12px): sidebar headings, result headings, and active structural labels.
- **Body** (regular or medium, 12px): profile names, tab titles, buttons, and actionable rows.
- **Label** (regular, 10px): endpoints, engine names, shortcuts, tags, counts, status summaries, and result cells.
- **Editor** (regular, 15px, comfortable line height): editable query text only.

### Named Rules

**The Two-Scale Rule.** Shell chrome stays primarily at body and label sizes. Larger type belongs to modal structure supplied by the native component library, not routine workspace chrome.

**The Editor Owns Mono Rule.** Monospace type is for editable query content and code-like values, not for general labels or navigation.

## Layout

The desktop composition is a horizontal connection sidebar and flexible work area above a full-width status bar. The sidebar is fixed at 272px, while the query region absorbs remaining width; the supported window floor is 960 by 600 pixels.

Horizontal dividers establish a 40px sidebar header, 40px query toolbar, 40px tab strip, 30px result header, and 32px status bar. The active query editor begins at 280px high and yields remaining height to results. Repeated internal spacing follows a compact 4px, 8px, 12px, and 16px rhythm; 24px is reserved for sparse startup or failure states.

The command palette is a centered 560px overlay, and transient notifications form a 360px stack at the upper-right. Content that can grow—tabs, database objects, command results, and result grids—scrolls within its pane rather than expanding the window.

**The Continuous-State Rule.** Hiding the connection sidebar may enlarge the work area, but the tab strip and status bar remain present so query and runtime context is not lost.

## Elevation & Depth

Astesia is flat by default. Workspace hierarchy comes from adjacent neutral tones and one-pixel borders. Medium shadow appears on notifications and large shadow appears on the command palette; ordinary panes, tabs, rows, toolbars, and data grids remain unshadowed.

### Shadow Vocabulary

- **Transient Notice:** two soft black layers at 10% opacity, offset 4px and 2px, for upper-right notifications.
- **Modal Command Surface:** two soft black layers at 10% opacity, offset 10px and 4px, for the command palette above its dimmed backdrop.

### Named Rules

**The Overlay-Only Elevation Rule.** Shadows signal temporary content above the workspace. Persistent application structure uses borders and tonal layering.

## Shapes

Controls and inline tags use gently compact 4px corners. Notices, fields, profile rows, and tab tops use 6px corners. The command palette alone uses the broader 8px overlay radius. Status and identity marks are circular.

Borders are structural rather than decorative: one-pixel strokes separate panes, bound fields and overlays, and expose keyboard focus. Active tabs keep rounded top corners but meet the editor canvas with a straight lower edge.

## Components

### Buttons

- **Shape:** compact controls use 4px corners and an 18px control height.
- **Filled:** tonal panel fill with standard text; used for the primary query execution and profile confirmation actions.
- **Outlined:** panel fill plus a structural border; used for secondary or reversible actions.
- **Transparent:** no resting fill; used for low-emphasis status-bar settings and icon actions.
- **Hover / Focus:** hover shifts to the theme's ghost-element state; focus remains visibly distinguished by the native focus treatment.

### Chips

- **Style:** engine pickers use compact toggle buttons; profile tags use a quiet element fill, 4px corners, label-size text, and tight horizontal padding.
- **State:** selected engine controls use the native accent tint. Tags are metadata, not actions.

### Cards / Containers

- **Corner Style:** persistent panes are square; interactive profile rows use 6px corners inside the sidebar.
- **Background:** workspace, panel, and editor theme roles form the layer stack.
- **Shadow Strategy:** persistent containers have no shadow.
- **Border:** one-pixel separators and focus borders carry containment.
- **Internal Padding:** dense rows use 8–10px horizontally and 8px vertically; empty and error states use 16–24px.

### Inputs / Fields

- **Style:** editor-surface fill, 6px corners, one-pixel variant border, and 8px horizontal padding.
- **Focus:** the border changes to the active theme's focus color.
- **Error / Disabled:** validation changes the border to the semantic error treatment and includes a text explanation below the field.

### Navigation

The connection sidebar uses grouped, full-width profile rows with explicit selected fill, disclosure depth for databases and objects, and visible MCP and session metadata. Query tabs are 36px high, use top-only rounding, and distinguish active state by moving from the tab-bar surface to the editor surface. The status bar places state summary on the left and global commands, language, theme, session, and activity details on the right.

### Command Palette

The palette is a 560px elevated search surface with a 44px input row and scrollable results. Each result shows a body-size title, label-size category, and optional shortcut. Selection uses the native ghost-selected fill; an empty search result explains that no commands match.

### Notifications

Notifications form a maximum stack of four compact semantic surfaces. Each includes readable text, a semantic border and fill, and a dismiss action; long messages clamp rather than widening the stack.

## Do's and Don'ts

### Do:

- **Do** use the current theme's semantic roles for every persistent surface, separator, text tier, and focus state.
- **Do** keep operational state visible in labels, status summaries, notices, or icons in addition to color.
- **Do** preserve compact row heights, internal scrolling, and the sidebar–tabs–status composition for new Milestone 3 surfaces.
- **Do** use the engine identity set only where database type or Connection Profile identity is being communicated.

### Don't:

- **Don't** add large display typography, decorative gradients, glass effects, or standalone marketing-style cards to the operator workspace.
- **Don't** use shadows on persistent panes, toolbars, tabs, rows, or data grids.
- **Don't** turn status or engine colors into generic primary-button colors.
- **Don't** replace text status with color-only dots, especially for errors, connection activity, or Usage Lease state.
