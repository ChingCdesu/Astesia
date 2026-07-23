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

Database password references still use the sidecar process environment. When
using the helper, launch Astesia from an environment containing each variable
referenced by `password_env`.

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

Connections use a password environment-variable reference rather than a password value. For example, pass `"password_env": "ASTESIA_ANALYTICS_PASSWORD"` to `create_connection`; the server reads that variable from its own environment when needed. Plaintext passwords are neither accepted in tool arguments nor returned in tool results. If the client does not support secret injection, start it from an environment where the variable is already set.

Connection profiles, saved queries, and update-confirmation preferences are
scoped to one MCP session. A stdio process has one session; each Streamable HTTP
session has its own catalog. They are not imported from the desktop app's
WebView storage. Recreate them after starting a new session.

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
| No destructive confirmation | Reads, connection testing/session access, additive creation, inserts |
| Confirmation required | Updates, deletes, permission changes, destructive DDL, and parseable SQL whose effects cannot be proven safe |
| Confirmation always required | All delete tools, `DELETE`, permission changes, `DROP`/`TRUNCATE`, and unknown SQL |

Only one successfully parsed SQL statement is accepted per saved query and execution request. Multi-statement and unparseable input is rejected rather than executed. Function-bearing `SELECT` statements are conservatively treated as unknown because a database function can have write side effects.

High-risk operations use MCP elicitation before any change is made. If a client
does not advertise elicitation support, Astesia fails closed: it returns an
error and performs no operation. Rejecting or dismissing a prompt likewise
leaves the target unchanged. The connected MCP client is responsible for
showing the prompt faithfully; possession of the Bearer token is therefore a
trusted security boundary.

For `UPDATE` operations only, the prompt includes `do_not_ask_again`. Accepting
it suppresses later update prompts only for the current MCP session and the same
`connection_id` plus `database`. The choice is not persisted and never
suppresses confirmations for deletes, permissions, destructive DDL, or unknown
SQL.

Structured row updates and deletes first verify the table metadata and currently require a single-column primary key. Database calls have a 60-second client-side timeout; verify database state before retrying a timed-out write. Add an explicit database-side row limit to large saved `SELECT` queries; the MCP response cap limits returned output but cannot prevent every driver from materializing a larger result internally.
