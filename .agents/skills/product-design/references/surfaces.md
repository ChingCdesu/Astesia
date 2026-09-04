# Surface routing

Load every surface materially changed by the request; do not load adjacent surfaces merely because
they share Application Core types.

| Surface | Load when the change affects | Reference |
| --- | --- | --- |
| Shell and workspace | startup, sidebar/tab/status composition, command palette, theme/language, native prompts, restart, global shortcuts | [surfaces-shell.md](surfaces-shell.md) |
| Connections and catalog | Connection Profiles, Database Sessions, grouping/filtering, catalog browsing, object actions, engine capability visibility | [surfaces-connections.md](surfaces-connections.md) |
| Query and data | SQL/Redis editing, files, completion, execution/results, relational grid, Mongo/Redis viewers, export | [surfaces-query-data.md](surfaces-query-data.md) |
| Operations and insight | background tasks, backup/restore/copy, MCP Sidecar, charts, ER diagrams, performance | [surfaces-operations.md](surfaces-operations.md) |

Cross-surface changes also load [rules.md](rules.md). Stateful, destructive, asynchronous, or
permission-sensitive changes load [resilience.md](resilience.md).
