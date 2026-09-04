# Astesia MCP Server

`astesia-mcp` exposes Astesia database operations to MCP-compatible AI tools.
It supports stdio for standalone clients and authenticated Streamable HTTP when
managed by the Astesia desktop app.

## Build

From the repository root, build the release binary:

```bash
cargo build --release --locked --manifest-path src-tauri/Cargo.toml --bin astesia-mcp
```

The desktop package scripts build `astesia-mcp` beside the `astesia` executable so the native
process host discovers the exact sidecar shipped with the application. A direct Cargo build places
the executable under `src-tauri/target/release/`.

## Desktop Integration Status

The GPUI Shell exposes start, stop, restart, status, and client-configuration controls for the
bundled HTTP sidecar. Standalone stdio remains available. Both modes share the Application's
connection repository and synchronization contracts.

Streamable HTTP and stdio both open the desktop app's shared connection
repository. On connection tests and connection requests, the MCP server resolves
and decrypts the saved credential on the server side using the App-managed
vault and its OS-stored master key. Passwords and credential references are
never sent to the MCP client, WebView, or synchronization channel. MCP clients
cannot create, edit, or delete connection profiles; create and maintain them in
the desktop app, then explicitly enable the profiles that MCP may use.

## Cross-process Connection Leases

Both transports acquire a per-connection shared OS file lock before
`test_connection` starts and hold it for the entire test. A successfully
connected driver retains the same kind of shared lease until it is disconnected,
its session ends, or its process exits. Multiple MCP users may hold compatible
shared leases for one connection at the same time.

Before saving or deleting a connection, the desktop backend attempts a
non-blocking exclusive lock for that connection. If any Streamable HTTP or
stdio MCP process holds a shared lease, the backend returns
`connection_in_use` without changing the profile, credential, or repository
revision. This backend check is authoritative even when the UI cannot observe a
standalone stdio session.

Leases are released when their driver/test guard is dropped, and the operating
system kernel releases them if a sidecar exits abnormally. The lock files are
intentionally retained and contain no profile metadata or credentials; their
names are hashes of connection IDs. File presence does not indicate occupancy:
only the live OS lock does.

This lease contract is part of shared repository schema v3. The App upgrades
the repository first, and bundled-sidecar verification must succeed against
that schema before MCP access is enabled, so an older lease-unaware sidecar is
not accepted.

## Configure a Standalone Stdio Client

MCP client configuration formats vary, but a generic stdio entry looks like this:

```json
{
  "mcpServers": {
    "astesia": {
      "command": "/absolute/path/to/Astesia/src-tauri/target/release/astesia-mcp",
      "args": []
    }
  }
}
```

Use an absolute executable path. The MCP client configuration does not contain
database credentials.

A standalone stdio sidecar opens the same shared connection repository as the
desktop app. It accepts only existing connection IDs, and resolves encrypted
credentials on the server side when a test or connection is requested. MCP
clients cannot create, edit, or delete connection profiles and never receive a
password or credential reference. Configure profiles in the desktop app and
set `mcp_enabled=true` for every profile that stdio may use.

Live drivers and `connected` state remain local to each process, so a stdio
connection does not appear connected in the App. Unlike App-managed HTTP,
stdio has no bidirectional control channel: the desktop app cannot push a
force-disconnect command to a stdio session or receive its real-time driver
state. Its OS lease remains visible to the desktop backend, so profile saves
and deletes are still rejected while stdio uses the connection. If the user
requests an App-side disconnect while stdio still holds the lease, Astesia
disconnects the drivers it can reach and returns a `DisconnectReport` whose
external usage is `StillInUse`. The UI instructs the user to call
`disconnect_connection` in the MCP client or close the stdio process. Saved MCP
queries and update-confirmation preferences remain scoped to one MCP session.

The GPUI rebuild does not import WebView `localStorage`. Before constructing the
Application, it probes the native shared repository and leaves corrupt or
unreadable state untouched. Existing native repository and vault data remain
available; profiles that existed only in WebView storage must be recreated.
Metadata loads without opening the credential vault, and App and MCP credential
access fail closed when an operation first needs a password.

On Linux, a Secret Service provider and session D-Bus must be available (for
example GNOME Keyring, KWallet, or KeePassXC Secret Service). If the system
credential store is missing, locked, or denies access, metadata remains
listable but migration, connection access, and credential changes fail closed
with a remediation message. Astesia never falls back to a plaintext password
file. The encrypted vault is protected by an OS-stored random key; it is not the
user-master-password fallback discussed below. This release does not include
that fallback, so install or enable a Secret Service provider before migrating.

The master-password fallback threshold was reviewed in July 2026. Public
desktop data does not measure Secret Service availability directly, so Astesia
uses the conservative identifiable-platform bound: even if every Linux desktop
lacked a provider, Linux represented 4.36% of worldwide desktop traffic in
[StatCounter's June 2026 data](https://gs.statcounter.com/global-stats/os-market-share/desktop/worldwide/),
below the 20% product threshold. Windows and macOS use their native credential
stores. Reassess this decision if platform-specific Astesia telemetry or newer
market data exceeds the threshold.

On macOS, the App and `astesia-mcp` are separate executables with separate data
protection Keychain scopes. Each process lazily imports the shared master key
into its own user-presence-protected item when that process first needs a
database password. macOS then chooses Touch ID, Apple Watch, or the local
account password according to System Settings. The legacy shared item remains
as a migration bridge so an independently launched sidecar can import the same
key without exposing it through WebView or MCP payloads. If an ad-hoc build
lacks the entitlement required by the data-protection Keychain, Astesia safely
continues using that classic Keychain item instead of blocking existing
encrypted connections. Authentication UI requires a graphical login session.
Release builds should still sign the App and sidecar with stable identities so
protected items survive upgrades. Windows Credential Manager normally resolves
the same single master-key item without an interactive prompt.

Standalone stdio and the bundled verifier run in strict mode: they never read or
delete an older per-connection system-credential item. If App migration has not
finished, MCP returns `credential_migration_required` with instructions to open
Astesia, preserves the old item, and refuses the operation.

Each App-managed Streamable HTTP session opens and owns its database drivers in
the sidecar while reading profile metadata and credentials from the shared
repository. The desktop app does not create a duplicate driver for an HTTP
session.

A private synchronization channel reports sanitized occupancy and disconnect
state to the App using only the connection ID, profile revision, session
generation, phase, and optional error information. It never transports profile
details or credentials. This lets the App display HTTP occupancy and proactively
disable editing and deletion; the cross-process exclusive-lock check remains
the authoritative protection for both HTTP and stdio.

When the user disconnects a connection from the App, Astesia closes its own
driver and sends generation-scoped control commands to every reachable HTTP
session. A connection test is reported as HTTP occupancy too; the control
command cancels it and waits for its test future and shared lease to be dropped.
Each HTTP session otherwise closes its driver and acknowledges the result before
the App checks the cross-process lease again. If stdio or another external MCP
process still holds a lease, the operation returns a `DisconnectReport` whose
external usage is `StillInUse`. Its remediation tells the user to call
`disconnect_connection` in that MCP client or close the stdio process.

Ending an HTTP session or stopping the helper releases its synchronized
occupancy state and driver lease. Stale commands cannot disconnect a newer
session generation.

## Tools

Tools are grouped by purpose:

- Connections: `list_connections`, `test_connection`, `connect_connection`, `disconnect_connection`
- Objects and schema: `create_database_object`, `delete_database_object`, `create_schema`, `delete_schema`, `create_table`, `delete_table`
- Saved queries: `list_queries`, `create_query`, `execute_query`, `delete_query`
- Rows: `insert_row`, `read_rows`, `update_row`, `delete_rows`

Connect a connection before using database, schema, query, or row tools. Saved-query tools manage Astesia query definitions; deleting one does not itself run SQL against the database.

Structured table, query, insert, update, and delete tools currently target MySQL, PostgreSQL, SQLite, SQL Server, and ClickHouse. Schema tools remain unavailable for SQLite and ClickHouse. MongoDB and Redis support connection lifecycle operations and `read_rows` through their existing Astesia drivers.

`create_database_object` accepts databases, views, functions, procedures, and triggers. Database user credentials are intentionally outside this API. Credential-bearing permission SQL (for example, a password or token clause) is rejected; configure those secrets through a trusted database administration path.

## Risk and Confirmation Model

Astesia classifies SQL as `ReadOnly`, `Additive`, `Update`, `Delete`, `Permissions`, `Destructive`, or `Unknown`.

| Behavior | Operations |
|---|---|
| No destructive confirmation | Reads, additive creation, inserts, and credential-free connection testing/session access |
| Confirmation required | Updates, deletes, permission changes, destructive DDL, and parseable SQL whose effects cannot be proven safe |
| Confirmation always required | All delete tools, `DELETE`, permission changes, `DROP`/`TRUNCATE`, and unknown SQL |

Only one successfully parsed SQL statement is accepted per saved query and
execution request. Multi-statement and unparseable input is rejected rather
than executed. Unqualified standard aggregates (`COUNT`, `SUM`, `AVG`, `MIN`,
and `MAX`) remain read-only, including conditional aggregation, window clauses,
and `UNION ALL`, but only with their standard positional argument shapes
(`COUNT` requires exactly one wildcard or expression; the others require
exactly one expression).
Other function calls are conservatively treated as unknown because a database
function can have write side effects. Database administrators must not shadow
these standard aggregate names with side-effecting functions in an earlier
function-resolution path.

Connection tests and access resolve credentials entirely on the server side
and only for shared profiles marked `mcp_enabled`. MCP clients do not import
environment credentials or participate in password handling. This connection
authorization is independent of destructive SQL confirmation.

High-risk operations use MCP elicitation before any change is made. If a client
does not advertise elicitation support, Astesia fails closed: it returns an
error and performs no operation. Tool errors retain the `error` text field and
also include `error_code`, `retryable`, and `details`. An unsupported client
receives `astesia.approval.unsupported` with an explicit explanation that it did
not declare form elicitation support. Rejecting or dismissing a prompt likewise
leaves the target unchanged. The connected MCP client is responsible for
showing the prompt faithfully; possession of the Bearer token is therefore a
trusted security boundary.

For `UPDATE` operations only, the prompt includes `do_not_ask_again`. Accepting
it suppresses later update prompts only until that connection is disconnected
in the current MCP session, and only while the exact shared profile revision,
endpoint configuration, and target database remain unchanged. The choice is
not persisted and never suppresses confirmations for deletes, permissions,
destructive DDL, or unknown SQL.

Structured row updates and deletes first verify the table metadata and currently require a single-column primary key. Database calls have a 60-second client-side timeout; verify database state before retrying a timed-out write. Add an explicit database-side row limit to large saved `SELECT` queries; the MCP response cap limits returned output but cannot prevent every driver from materializing a larger result internally.
