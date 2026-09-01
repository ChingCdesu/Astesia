# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Users

Astesia is an internal desktop tool for developers and data operators who work across several database engines and need to inspect schemas, run queries, edit data, diagnose performance, and expose selected database access to local MCP clients.

## Product Purpose

Astesia provides one native workspace for saved database access, live Database Sessions, querying, data and schema operations, long-running maintenance tasks, and MCP access. The GPUI rebuild succeeds when it preserves the Legacy Shell's workflows while removing the React, WebView, and Tauri runtime.

## Positioning

The same Connection Profiles, credential bindings, revision checks, Usage Leases, engine capability rules, and MCP state drive both the desktop workspace and local MCP access. The UI is a client of this shared Application Core rather than a second implementation of database behavior.

## Operating Context

- Desktop use with macOS implemented and validated first; Windows and Linux follow after macOS workflow parity.
- Frequent keyboard-driven work in a multi-pane database workspace, including Chinese IME, query editing, schema browsing, tab navigation, and long-running tasks.
- Seven supported engines: MySQL, PostgreSQL, SQLite, SQL Server, ClickHouse, MongoDB, and Redis.
- Existing native Connection Profiles remain authoritative when readable. WebView-only preferences and connections are not imported.
- The MCP Sidecar is loopback-only and shares repository state with the desktop application.

## Capabilities and Constraints

- The final `astesia` process is a standalone GPUI Shell with no Tauri or WebView dependency.
- Rust 1.97.1 and Zed commit `399258feeaf90ad8a3a208c99221ee87b6452f38` are fixed baselines.
- Astesia embeds a local Zed `Editor` without initializing Zed authentication, collaboration, telemetry, AI, marketplace, or network services.
- Zed runtime data is isolated below Astesia's application data directory.
- The Application Core stays independent of GPUI and Tauri. Platform access is exposed through narrow interfaces such as events, sidecars, dialogs, files, and preferences.
- Startup must surface unreadable or corrupt native state without replacing it with a newly initialized empty repository.
- The first GPUI pass preserves behavior, shortcuts, destructive confirmations, and information architecture rather than matching CSS pixels.
- External licensing, public distribution, notarization, signing, updater compatibility, and automatic WebView-state migration are outside the current internal rebuild.

## Brand Commitments

- Product name: Astesia.
- Simplified Chinese is the default language and English remains available.
- The incumbent interface is compact, neutral, and task-oriented: a connection/sidebar region, tabbed work area, and persistent status surface with restrained engine and state colors.

## Evidence on Hand

- Product capabilities and development contract: `README.md` and `AGENTS.md`.
- Domain language and migration boundaries: `CONTEXT.md`.
- Approved runtime decision: `docs/adr/0001-rebuild-the-desktop-shell-with-gpui.md`.
- Staged delivery and acceptance contract: `docs/plans/gpui-ui-rebuild.md`.
- Incumbent interaction and visual evidence: `src/components/`, `src/stores/`, `src/i18n/`, and `src/styles/global.css`.
- No customer testimonials, public usage claims, or benchmark claims are available and future UI work must not invent them.

## Product Principles

1. Preserve database and credential safety before preserving convenience.
2. Keep one authoritative Application Core for desktop and MCP workflows.
3. Make connection, execution, task, and error state continuously visible.
4. Prefer compact native operator workflows and strong keyboard behavior over decorative chrome.
5. Add engine-specific actions only when the capability matrix permits them.

## Accessibility & Inclusion

Keyboard focus, shortcut precedence, editable-focus handling, Chinese IME composition, readable status changes, and non-color-only error/connection indicators are required interaction contracts.
