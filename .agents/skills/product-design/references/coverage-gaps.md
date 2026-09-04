# Coverage gaps

These are evidence limits, not approved product work or permission to expand a task.

- Windows x64 has compile-and-link acceptance but no current native runtime or visual acceptance.
- External distribution, licensing, notarization, public updater compatibility, and automatic
  WebView-state migration remain outside the internal product contract.
- The repository has no automated rendered-state or screenshot regression suite for GPUI surfaces;
  source checks cannot establish visual parity.
- The 960 by 600 window floor is accepted, but no product behavior exists for smaller windows.
- Bilingual strings are owned by their GPUI modules; there is no separate canonical translation
  catalog or accepted RTL product requirement.
- No project-specific design linter is accepted yet. A candidate belongs here until source can
  detect it with low false positives and the team approves the rule.
