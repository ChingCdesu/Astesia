# GPUI Kit design authority

Load for visual or interaction changes. Cargo.lock owns the GPUI Kit and gpui-pre versions;
inspect those installed sources before relying on component names or behavior. ADR-0002
supersedes the prior pinned-Zed-component requirement.

Use GPUI Kit controls and active semantic theme tokens. Astesia's shared controls in
`src/ui/components/` own product roles; avoid duplicate palettes or copies of component styling.
Use Editor for source text and Input for single-line values. Menus, tabs, dialogs, focus,
selection, disabled state, and platform title bars should inherit Kit behavior.

Custom grids, charts, ER diagrams, and variable-height catalog lists retain product-specific
rendering. Use the current theme and preserve keyboard, accessibility, and loading/error states.
Read DESIGN.md for workspace composition and the surface references for behavior.

When no Kit primitive fits, state the concrete gap and verify the custom implementation in
its default, focused, selected, disabled, loading, overflow, light, and dark states as applicable.
Do not treat a successful compile as native interaction or rendered evidence.
