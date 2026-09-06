# Connections and catalog

## Job

Configure a Connection Profile, establish a Database Session, and discover only operations the
selected engine and current state can perform.

## Contract

- Profile forms expose engine-correct fields and defaults, preserve stored credentials when the
  password remains blank, and distinguish validation, credential, repository, and connection
  failure.
- Connecting, connected, disconnecting, MCP-use, and failure remain readable without color alone.
- Usage Leases and revisions block unsafe edits, deletion, or disconnection; stale data refreshes
  before another mutation.
- Catalog nodes load lazily and in place. Unsupported sections and actions are absent, not displayed
  as misleading empty or enabled controls.
- Destructive actions name the profile or qualified database object and its consequence. The owning
  catalog refreshes only after success.

## Evidence

- `docs/plans/gpui-milestone-0-acceptance.md`: capability matrix, C01-C06, D01-D05, and confirmation
  contract.
- `src/ui/connection_profile_form.rs` and `src/ui/connections/`.
- `src/db/engine.rs` for current capability ownership.
