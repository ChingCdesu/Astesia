# Operations and insight

## Job

Understand long-running work and database insight without confusing progress, cancellation, partial
output, stale state, or unsupported engine behavior.

## Contract

- Backup, restore, and copy state the source, destination, engine, structure/data scope, and durable
  output before starting.
- Task Center exposes running, cancelling, completed, failed, partial, and cancelled as distinct
  inspectable states. Progress is monotonic and terminal notification is emitted once.
- MCP Sidecar start, stop, restart, endpoint, token, configuration, and failure state remain
  accurate. Access uses explicitly enabled profiles and session-scoped destructive approval.
- Charts show only mappings supported by the result shape; empty and non-numeric data explain why
  no chart is available.
- ER diagrams preserve qualified table identity and remain operable for empty, small, and large
  schemas.
- Performance dashboards use engine-specific metrics and make manual versus interval refresh state
  visible.

## Evidence

- `docs/plans/gpui-milestone-0-acceptance.md`: O02-O08 and V01-V03.
- `docs/plans/gpui-milestone-6-acceptance.md` and `docs/plans/gpui-milestone-7-acceptance.md`.
- `src/ui/task_center_item.rs`, `src/ui/mcp_service_item.rs`, `src/ui/chart_view.rs`,
  `src/ui/er_diagram_item.rs`, and `src/ui/performance_item.rs`.
