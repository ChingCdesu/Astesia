# Shell and workspace

## Job

Keep the active database context, open work, and runtime state continuously legible while the
operator moves quickly by keyboard.

## Contract

- Startup runs the Native State Probe before initializing repository state. Loading, no profiles,
  and recoverable failure are distinct.
- The connection sidebar, tab strip, work area, status surface, overlays, and notifications share
  one synchronized active profile/database context.
- Workspace shortcuts win only in their declared context and do not steal normal editor, field, or
  IME input.
- Tab selection stays stable. Closing, bulk-closing, opening another file, or restarting cannot
  discard dirty queries without an explicit choice.
- Theme and language update the complete shell. System appearance remains live when selected.
- Native file prompts distinguish cancellation from platform failure; restart and preferences
  report persistence failure honestly.

## Evidence

- `docs/plans/gpui-milestone-0-acceptance.md`: S01-S06, shortcut contract, state contract, and
  confirmation contract.
- `docs/plans/gpui-milestone-8-acceptance.md`: P01-P03.
- `src/ui/mod.rs`, `src/ui/workspace.rs`, `src/ui/tabs.rs`, `src/ui/command_palette.rs`, and
  `src/ui/shell.rs`.
