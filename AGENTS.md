# Repository Guidelines

## Project Structure & Module Organization

The repository root is the Cargo package, and the native Rust application lives in `src/`:
UI-independent workflows belong in
`application/`, platform adapters in `platform/`, GPUI views in `ui/`, database adapters in `db/`,
MCP tools in `mcp/`, and background work in `tasks/`. The standalone MCP entry point is
`src/bin/astesia-mcp.rs`. Internal package definitions live in `packaging/`, with build
entry points under `scripts/`.

## Build, Test, and Development Commands

- `cargo run --locked --bin astesia` starts the native app.
- `cargo test --locked` compiles and tests the application.
- `cargo clippy --locked --all-targets` checks Rust lints.
- `cargo fmt -- --check` checks Rust formatting.
- `scripts/package-macos.sh <target>` builds an internal macOS application archive.
- `scripts/package-linux.sh x86_64-unknown-linux-gnu` builds the internal Linux archive.
- `scripts/package-windows.ps1 -Target x86_64-pc-windows-msvc` builds the Windows archive.

## Coding Style & Naming Conventions

Use standard four-space `rustfmt` output. Rust modules and functions use `snake_case`; structs and
enums use `PascalCase`. Keep Application Core types independent of GPUI, and preserve intentional
serialized snake_case fields such as `db_type`.

## Testing Guidelines

Changes should pass the Rust tests, Clippy, formatting, and `git diff --check`. Manually exercise
affected database engines through the native application, including failure paths. Put Rust unit
tests in `#[cfg(test)]` modules; keep environment-dependent engine checks ignored by default and
record explicit runs in the relevant acceptance document.

## Commit & Pull Request Guidelines

Recent history favors `type: concise summary`, especially `feat:`, `fix:`, and `ci:`; reserve `release:` for version commits. Use focused kebab-case branches such as `feat/query-improvements`. Pull requests should explain user-visible impact, identify affected database engines, link relevant issues, and list validation performed. Include screenshots or short recordings for UI changes.

## Security & Configuration

Never commit database credentials, signing keys, or real connection strings. Use disposable test
accounts. Keep MCP Sidecar secrets in environment variables rather than process arguments, and
explain any expansion of native platform access.

## Agent skills

### Issue tracker

Issues and specs are tracked in GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the five canonical triage labels. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo with `CONTEXT.md` at the root and ADRs under `docs/adr/`. See `docs/agents/domain.md`.
