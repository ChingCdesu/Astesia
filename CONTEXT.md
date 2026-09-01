# Astesia

Astesia is a native database workspace for managing saved database access, live sessions, queries, data, and MCP access. This language also distinguishes the legacy and GPUI editions during the rebuild.

## Language

**Legacy Shell**:
The final React/Tauri edition of Astesia, retained only in version control and previously signed artifacts as the behavioral reference for the GPUI rebuild.
_Avoid_: Old UI, Tauri app

**GPUI Shell**:
The native Astesia application that replaces the Legacy Shell.
_Avoid_: New UI, GPUI frontend

**Application Core**:
The UI-runtime-independent Rust model and services that define Astesia's application workflows.
_Avoid_: Backend, Tauri commands

**Cutover Gate**:
The complete acceptance contract the GPUI Shell must satisfy before it replaces the Legacy Shell for users.
_Avoid_: MVP, beta checklist

**Native State Probe**:
The startup eligibility check that determines whether an existing native repository and its credentials are safe for the GPUI Shell to use.
_Avoid_: Migration, marker check

**Connection Profile**:
A saved configuration identifying how Astesia accesses a database.
_Avoid_: Connection, account

**Database Session**:
An active runtime relationship with a database established from a Connection Profile.
_Avoid_: Connection, profile

**Usage Lease**:
An exclusive claim that prevents a Connection Profile from changing while Astesia or an MCP client is using it.
_Avoid_: Lock, usage flag

**MCP Sidecar**:
The companion Astesia process that exposes database capabilities to MCP clients.
_Avoid_: MCP helper, MCP server binary
