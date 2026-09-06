# Exemplar: native cutover

The editor integration described here is historical; ADR-0002 now selects GPUI Kit.

## Decision worth repeating

Astesia replaced the Legacy Shell with a standalone GPUI Shell while retaining one UI-independent
Application Core. Native prompts, clipboard, preferences, restart, and sidecar discovery have
explicit boundaries. The embedded Zed editor runs locally without initializing Zed authentication,
collaboration, telemetry, marketplace, or network services.

The product chose truthful failure over silent migration: the Native State Probe protects existing
repository and credential state, WebView-only state is not imported, cancellation differs from a
platform error, and restart asks before discarding dirty tabs.

## Evidence

- `docs/adr/0001-rebuild-the-desktop-shell-with-gpui.md`
- `docs/plans/gpui-milestone-8-acceptance.md`
- `src/ui/mod.rs` and `src/ui/workspace.rs`
- Commit `e695512` (`docs: close native runtime milestone`)

## Repeat

- Put workflow truth in Application Core and let GPUI present it.
- Make unsupported migration or platform behavior explicit before users rely on it.
- Verify native input, dialogs, clipboard, packaging, and failure behavior on the exercised platform.

## Do not copy as precedent

- Historical nested `src-tauri` paths; the current Cargo package lives at the repository root.
- Internal-only distribution assumptions for a future public release.
- A static or cross-compiled artifact as evidence of runtime behavior on an unexercised platform.
