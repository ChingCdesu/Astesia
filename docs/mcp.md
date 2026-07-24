# Astesia MCP Server

`astesia-mcp` exposes Astesia database operations to MCP-compatible AI tools.
It supports stdio for standalone clients and authenticated Streamable HTTP when
managed by the Astesia desktop app.

## Build

From the repository root, build the release binary:

```bash
pnpm mcp:build
```

This builds a release executable and stages it at
`src-tauri/binaries/astesia-mcp-<target-triple>` (`.exe` on Windows), which is
the naming convention Tauri uses for external binaries. `pnpm tauri:dev`
prepares the debug sidecar automatically; `pnpm tauri:build` prepares and
bundles the release sidecar in the installer.

## Use the Desktop MCP Helper

Select **MCP** in the app status bar to open the helper. It shows sidecar
availability, process state, PID, endpoint, version, and recent startup errors.
Choose a local port from `1024` to `65535` (default `43677`), then use
**Start**, **Stop**, or **Restart**. The app stops its child process when the app
exits, and the sidecar also shuts itself down if its parent process terminates
unexpectedly.

The service binds only to `127.0.0.1` and requires a randomly generated Bearer
token. Use **Copy Configuration** to copy the endpoint and authorization header
for your AI client. The token is stored locally by Astesia, passed to the
sidecar through its environment, and never placed in the process arguments or
URL. Client configuration formats vary; adapt the copied object to the format
required by your client. The helper hides the token by default and can rotate it;
rotation restarts a running service. The endpoint is intended for native local
MCP clients and does not enable browser CORS.

Database password references still use the App process environment. For
App-managed HTTP, variables referenced by `password_env` must use the dedicated
`ASTESIA_DB_PASSWORD_` prefix, for example
`ASTESIA_DB_PASSWORD_ANALYTICS`. Launch Astesia with only the database
credentials that HTTP MCP sessions are allowed to use; other environment
variables cannot be selected as connection passwords. Before the first test or
access that would use one of these credentials, Astesia asks the MCP client to
confirm the exact database type, host, port, user, and database. The approval is
limited to that profile in the current HTTP session. If the client cannot show
the prompt, the credential is not read and the connection attempt fails closed.

## Configure a Standalone Stdio Client

MCP client configuration formats vary, but a generic stdio entry looks like this:

```json
{
  "mcpServers": {
    "astesia": {
      "command": "/absolute/path/to/Astesia/src-tauri/binaries/astesia-mcp-<target-triple>",
      "args": [],
      "env": {
        "ASTESIA_ANALYTICS_PASSWORD": "<inject with your client's secret manager>"
      }
    }
  }
}
```

Use an absolute executable path. Do not commit a configuration containing real credentials.

Connections use a password environment-variable reference rather than a
password value. A standalone stdio client may pass, for example,
`"password_env": "ASTESIA_ANALYTICS_PASSWORD"` to `create_connection`. Before
reading that variable, Astesia asks the client to confirm the exact endpoint.
After approval, stdio saves the password in the current user's system
credential store and writes only a random credential reference to the shared
SQLite metadata repository. Plaintext passwords are neither accepted in tool
arguments nor returned in tool results.

Desktop and stdio connection profiles are persistent and bidirectionally
shared. A connection created by either surface appears in the other; edits and
confirmed deletes also persist. Live drivers and `connected` state remain local
to each process, so a stdio connection does not appear connected in the App.
Saved MCP queries and update-confirmation preferences remain scoped to one MCP
session. For a shared profile, `mcp_enabled=true` is the App-side authorization
for stdio to resolve that profile's system-store credential; it does not prompt
again in every MCP session. Importing a new `password_env` credential through
stdio still requires endpoint-specific elicitation before the variable is read.

On the first App start after upgrading, Astesia blocks access until legacy
WebView connection data has been migrated. It explains the change before
starting, creates one random master key in the system credential store, and
encrypts each password independently in a shared local vault with AES-256-GCM.
The authenticated data binds every ciphertext to its connection ID, database
type, endpoint, account, and database. The bundled `astesia-mcp` then proves it
can open the exact enabled-credential set at the same repository revision. The
App deletes the old `localStorage` value only after repository migration,
legacy-item cleanup, and sidecar verification all succeed. The same verification
runs on later App starts so a sidecar changed by an upgrade fails closed before
use.

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

On macOS, the App and `astesia-mcp` are separate executables. Keychain may ask
the user to authorize the sidecar the first time it reads the single App-created
master-key item; it no longer reads one Keychain item per connection. Choose
**Always Allow**, not a one-time approval, in a graphical login session before
using a non-interactive stdio client. Release builds must sign the App and
sidecar with stable identities so an upgrade does not invalidate authorization.
Windows Credential Manager normally resolves the same single master-key item
without an interactive prompt.

Standalone stdio and the bundled verifier run in strict mode: they never read or
delete an older per-connection system-credential item. If App migration has not
finished, MCP returns `credential_migration_required` with instructions to open
Astesia, preserves the old item, and refuses the operation.

Each App-managed Streamable HTTP session keeps a transient catalog, but its
connection lifecycle is mirrored into the desktop app. Mirrored profiles are
removed when their MCP session or helper service ends and are not written to
the shared connection repository.

The desktop app opens its own driver for a connected HTTP profile so the
mirrored entry can be queried normally. It resolves the same `password_env`
inside the App process; resolved password values are never sent to the WebView
or synchronization events. Mirrored HTTP profile lifecycle controls are
read-only in the app and remain owned by the HTTP MCP session.

## Tools

Tools are grouped by purpose:

- Connections: `list_connections`, `create_connection`, `test_connection`, `connect_connection`, `disconnect_connection`, `delete_connection`
- Objects and schema: `create_database_object`, `delete_database_object`, `create_schema`, `delete_schema`, `create_table`, `delete_table`
- Saved queries: `list_queries`, `create_query`, `execute_query`, `delete_query`
- Rows: `insert_row`, `read_rows`, `update_row`, `delete_rows`

Connect a connection before using database, schema, query, or row tools. Saved-query tools manage Astesia query definitions; deleting one does not itself run SQL against the database.

Structured schema, table, query, insert, update, and delete tools currently target MySQL, PostgreSQL, SQLite, and SQL Server. MongoDB and Redis support connection lifecycle operations and `read_rows` through their existing Astesia drivers.

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

App-managed HTTP additionally requires a one-time, endpoint-specific
confirmation before a connection test or access can read a referenced database
credential. Standalone stdio requires the same endpoint-specific confirmation
when `create_connection` imports an environment credential into the system
credential store. Existing shared profiles marked `mcp_enabled` use the App's
authorization instead of repeating this credential prompt. These prompts are
independent of destructive SQL confirmation.

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
in the current MCP session, and only while the exact profile revision,
endpoint, credential reference, and target database remain unchanged. HTTP
profiles with revision zero are isolated by their endpoint fingerprint. The
choice is not persisted and never suppresses confirmations for deletes,
permissions, destructive DDL, or unknown SQL.

Structured row updates and deletes first verify the table metadata and currently require a single-column primary key. Database calls have a 60-second client-side timeout; verify database state before retrying a timed-out write. Add an explicit database-side row limit to large saved `SELECT` queries; the MCP response cap limits returned output but cannot prevent every driver from materializing a larger result internally.
