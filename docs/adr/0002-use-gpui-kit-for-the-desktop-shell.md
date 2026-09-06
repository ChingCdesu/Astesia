# Use GPUI Kit for the desktop shell

The desktop shell uses GPUI Kit instead of Zed's editor, project, workspace, theme, and UI
crates. This supersedes the embedded-editor decision in ADR-0001. The Application Core,
Native State Probe, credential boundaries, and database behavior remain unchanged.
The user approved a complete migration with existing functionality preserved.
The supported toolchain is Rust 1.98.0, as explicitly requested for this migration.

GPUI Kit owns the input and editor engines, highlighting, completion presentation, search,
component behavior, themes, platform integration, and window chrome. Astesia owns its
database workspace, tab lifecycle, guarded modal dismissal, SQL completion provider,
query execution, the find/replace toolbar, qualified catalog identities, and variable-height catalog rows.
The catalog keeps its virtual row model because inline errors, loading states, and Redis
search fields have different heights; row controls use GPUI Kit ListItem. The find/replace
toolbar uses Kit inputs and the editor matcher while owning the replacement value and shortcut
precedence. Menu triggers use a Kit Button and PopupMenu so keyboard and accessibility activation
reach the same handler as pointer activation.

Application code imports GPUI APIs and macros through `gpui_kit`. Only `gpui-kit` is
declared directly; its `test-support` feature enables native tests. The locked `gpui-pre`
package is the transitive runtime, with no separate application version pin.
Adding gpui-unofficial or Zed GPUI alongside these packages is not the migration path.

Only the SQL grammar feature is explicitly enabled. Database completion remains local to the
Application Core and rejects responses from obsolete sessions. Query selection uses UTF-8
byte offsets; completion edits use LSP UTF-16 positions.

Startup initializes no Zed services and no longer changes Zed's data-directory configuration.
Application data and credentials continue to use Astesia's existing storage. The UI asset
client remains blocked from HTTP; database and MCP networking belongs to existing services.

Acceptance requires the Rust checks and native evidence for editing, search and replace,
completion, Chinese IME, form focus, guarded dismissal, themes, catalog navigation, and dirty
work. Dependency removal is not proof of faster compilation: comparisons require equivalent
profiles and cache states.
