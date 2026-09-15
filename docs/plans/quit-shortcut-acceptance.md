# Quit shortcut acceptance — 2026-09-15

Cmd+Q was not bound and no native Quit menu action was registered. Both now route
through an application-scoped action. The handler defers its window update until
input dispatch completes, then asks the ready workspace to quit. Loading/error
states and an application with no windows can quit directly.

Unsaved tabs produce a native discard/cancel prompt. Repeated requests do not stack
prompts. Cancelling leaves all tabs and their edits intact.

Validation:
- Full test run before the final cancellation test: 426 library and 1 binary tests
  passed; 15 environment-dependent tests ignored.
- Both quit regression tests passed after adding cancellation coverage: Cmd+Q
  reaches app scope from editor focus; cancelling preserves a dirty tab and allows
  a subsequent quit request.
- Clippy passed with existing repository warnings; format and diff checks passed.
- macOS isolated debug app: Cmd+Q on an edited query showed the unsaved-tab warning.
  A separate clean startup exited with status 0 after Cmd+Q, with no process left.
- Native cancellation interaction was not conclusively observed; cancellation was
  verified by the GPUI regression test. Installed Astesia was not replaced.
