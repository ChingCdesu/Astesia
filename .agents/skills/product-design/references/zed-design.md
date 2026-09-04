# Zed design authority

Load for every visual or interaction change. Astesia follows the Zed source pinned in `Cargo.toml`;
inspect that exact checkout before relying on a component name, default, or behavior. A Zed website,
screenshot, current `main`, or another installed version is not the authority for this repository.

## Inheritance contract

Use Zed as a component system, not as a screenshot to imitate:

- Prefer `zed_ui` components such as `Button`, `IconButton`, `Label`, `ListItem`, `Tab`, `Modal`,
  `ModalHeader`, `ModalFooter`, `Tooltip`, `Indicator`, `Notification`, and data-table primitives.
- Use `ButtonStyle` for emphasis and semantics: Subtle is the common default, Filled adds emphasis,
  Outlined carries secondary emphasis, Transparent is quiet, and Tinted uses `TintColor` for a
  semantic state.
- Use `ButtonSize`, `LabelSize`, `TextSize`, the configured UI font, and the configured buffer font.
  Their runtime values follow the user's UI scale; local pixel copies do not.
- Use `DynamicSpacing` so Compact, Default, and Comfortable UI density remain coherent. Raw spacing
  is reserved for product geometry with no Zed token or for boundaries imposed by data rendering.
- Use `ElevationIndex`: Background, Surface, and EditorSurface own persistent structure;
  ElevatedSurface owns floating content; ModalSurface owns dialogs and alerts. Use the elevation's
  background and shadow rather than recreating them.
- Use `cx.theme().colors()` for surface, element, ghost-element, border, focus, selection, and text
  roles; use `cx.theme().status()` or Zed `Color` / `TintColor` for status semantics.
- Use component tooltip and accessibility APIs for meaningful controls. Visible text can supply the
  accessible name; icon-only, value-bearing, expandable, checkable, and composite controls declare
  the additional role, value, state, shortcut, or description that Zed exposes.

## Astesia extensions

Astesia owns the database workspace composition, engine capability visibility, bilingual product
copy, and the restrained engine identity palette. These extend Zed without restyling its controls.
An engine color identifies MySQL, PostgreSQL, SQLite, SQL Server, ClickHouse, MongoDB, or Redis; it
does not replace Zed action, focus, selection, warning, success, or error roles.

Custom query grids, charts, ER diagrams, and database visualizations may need raw GPUI rendering.
Build them from the active Zed theme and density values, then match surrounding Zed focus, disabled,
hover, selection, tooltip, and accessibility behavior.

## Review test

For every changed primitive, identify its Zed owner and inspect the exact pinned implementation.
When no owner exists, document the gap, reuse Zed semantic APIs, and verify default, hover, active,
focused, selected, disabled, loading, overflow, light/dark, and density behavior that the surface can
reach. Completion requires either a Zed component path or an evidenced exception.

## Current pinned sources

- `crates/ui/src/components/` for component behavior and accessibility.
- `crates/ui/src/styles/spacing.rs` for `DynamicSpacing` and density.
- `crates/ui/src/styles/typography.rs` for UI/buffer fonts and semantic text sizes.
- `crates/ui/src/styles/elevation.rs` for surface roles and shadows.
- `crates/ui/src/styles/color.rs` and `crates/theme/src/styles/` for semantic color roles.

Resolve these paths from Cargo metadata; do not encode the local Cargo checkout path in product
documentation.
