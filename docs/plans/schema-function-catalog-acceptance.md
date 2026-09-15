# Schema/function catalog acceptance — 2026-09-15

The schema-aware catalog renders Database → Schema → Tables / Functions as
selectable, collapsible folder rows, with muted counts and no section-header toolbar.
Tables defaults open and Functions closed. Create actions are in folder context menus.
PostgreSQL and SQL Server supply the function schema as structured metadata.
MySQL and ClickHouse retain unscoped functions; SQLite's unsupported function
section stays absent. Other catalog object sections are unchanged.

Validation:
- 425 library tests passed, 15 environment-dependent checks ignored.
- New tests cover schema metadata grouping, dots in schema/type names, overloads,
  empty/function-only schemas, failed/loading table metadata, legacy payloads,
  folder depth, row rendering, independent folder expansion, row selection,
  default expansion, and schema-collapse behavior.
- Existing catalog scroll anchoring and large-list virtualization tests passed.
- Clippy passed with existing repository warnings; formatting and diff checks passed.

The UI tests use fixture catalog metadata. No live PostgreSQL or SQL Server catalog
was queried, and the updated hierarchy has not been visually verified in a native
window. Native acceptance against these two engines remains outstanding.
