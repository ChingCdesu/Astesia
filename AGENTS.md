# Repository Guidelines

## Project Structure & Module Organization

The React 19/TypeScript frontend lives in `src/`: feature UI belongs in `components/<Feature>/index.tsx`, shared primitives in `components/ui/`, Zustand state in `stores/`, and common code in `types/`, `lib/`, `i18n/`, and `styles/`. The Rust backend is in `src-tauri/src/`; keep IPC handlers in `commands/`, database adapters in `db/`, MCP tools in `mcp/`, and background work in `tasks/`. Static assets are under `public/`; Tauri permissions and configuration are under `src-tauri/capabilities/` and `tauri.conf.json`.

## Build, Test, and Development Commands

- `pnpm install` installs locked frontend and Tauri CLI dependencies.
- `pnpm tauri:dev` starts the full desktop app with Vite hot reload.
- `pnpm dev` runs the browser frontend only.
- `pnpm lint` checks TypeScript and React rules with ESLint.
- `pnpm build` type-checks TypeScript and creates the frontend production bundle.
- `pnpm mcp:prepare:debug` builds and stages the target-named debug MCP sidecar.
- `cargo test --manifest-path src-tauri/Cargo.toml` compiles and tests the Rust backend.
- `pnpm mcp:build` compiles and stages the release MCP sidecar.
- `pnpm tauri:build` produces platform installers; use it for release validation.

## Coding Style & Naming Conventions

Use two-space indentation in TypeScript/TSX and standard four-space `rustfmt` output in Rust. Follow surrounding TypeScript style (single quotes and semicolons) and run ESLint before committing. Name React components and types in `PascalCase`, functions and variables in `camelCase`, Zustand files as `camelCaseStore.ts`, and UI primitives in lowercase kebab-case. Rust modules and functions use `snake_case`; structs and enums use `PascalCase`. Preserve intentional snake_case fields shared across Tauri IPC, such as `db_type`.

## Testing Guidelines

No frontend test suite or coverage threshold is configured. Changes should pass `pnpm lint`, `pnpm build`, and the Rust tests above. Manually exercise affected database engines through `pnpm tauri:dev`, including failure paths. Name future frontend tests `*.test.ts(x)`; place Rust unit tests in `#[cfg(test)]` modules.

## Commit & Pull Request Guidelines

Recent history favors `type: concise summary`, especially `feat:`, `fix:`, and `ci:`; reserve `release:` for version commits. Use focused kebab-case branches such as `feat/query-improvements`. Pull requests should explain user-visible impact, identify affected database engines, link relevant issues, and list validation performed. Include screenshots or short recordings for UI changes.

## Security & Configuration

Never commit database credentials, signing keys, or real connection strings. Use disposable test accounts. Treat `src-tauri/capabilities/default.json` and security settings in `tauri.conf.json` as sensitive; explain and minimize any permission expansion.
