# Query and data

## Job

Author and execute engine-correct work, understand ordered results, and edit supported data without
losing query text, row identity, type intent, or staged work.

## Contract

- SQL files preserve text, file identity, and dirty state across open/save cancellation and failure.
- Selection, current-statement, and full-editor execution target the intended range. Results remain
  ordered and per-statement errors do not hide earlier results.
- Editor-native selection, completion, find/replace, undo/redo, shortcuts, and Chinese IME coexist
  with workspace commands.
- Empty results, affected rows, timing, mixed results, and errors remain distinguishable.
- Relational edits require a usable primary key, remain staged until Save, and expose Undo/Discard.
  Navigation that would lose staged work is blocked.
- Batch save is transactional. Failure keeps every staged edit recoverable.
- ClickHouse and MongoDB remain read-only where the capability matrix says so; Redis exposes its own
  typed operations rather than relational controls.
- Export names current/all/range and column scope before choosing a durable file.

## Evidence

- `docs/plans/gpui-milestone-0-acceptance.md`: Q01-Q08, D06-D11, E01-E04, O01, shortcut contract,
  and state contract.
- `src/ui/query_item.rs`, `src/ui/data_grid_item/`, `src/ui/document_item.rs`, and
  `src/ui/redis_item.rs`.
