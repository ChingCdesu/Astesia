# Product copy

Load for visible text, accessible names, confirmations, errors, notifications, and terminology.

## Language contract

Simplified Chinese is the default and English remains complete. Change both strings together and
inspect both rendered layouts. Preserve exact product terms from `CONTEXT.md`; current Chinese terms
come from the owning GPUI surface rather than an invented global translation.

Name the exact product object and action. Prefer `连接配置` / `Connection Profile`, `数据库会话` /
`Database Session`, and `MCP 服务` or `MCP Sidecar` according to the surrounding established copy.
Use engine-native terms such as database, schema, table, collection, and key rather than collapsing
them into a generic item.

## Consequential copy

- **Title:** action plus exact object, such as `删除连接配置` / `Delete connection profile`.
- **Detail:** durable scope, credentials or dependent objects affected, reversibility, and any
  partial-output risk.
- **Primary action:** the concrete verb, such as Delete, Drop, Discard and Open, or Discard and
  Restart. Avoid generic approval wording.
- **Cancel:** leaves state unchanged and remains visually secondary.

## Status and errors

Use present-progress language for active work, a distinct terminal word for Completed, Failed,
Partial, or Cancelled, and stable object names throughout. An error says what failed, what state was
preserved or may be uncertain, and the next safe action. Never claim success from task dispatch or
when refresh, credential cleanup, or durable output remains uncertain.

Accessible labels describe the control's action and target. Status color is supplemental; names and
announcements carry the meaning.

## Evidence

- `CONTEXT.md` for canonical English domain names.
- `src/ui/localization.rs` and the owning `src/ui/` module for current bilingual copy.
- `docs/plans/gpui-milestone-0-acceptance.md` confirmation and state contracts.
