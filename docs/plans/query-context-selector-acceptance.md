# Query context selector acceptance

Verified on macOS on 2026-09-06.

- The toolbar offers connected SQL databases and PostgreSQL schemas, including the database default.
- GPUI keyboard tests select database and schema through actual popup menus, preserve SQL text,
  reset schema when changing databases, and reject schema changes during execution.
- Completion tests distinguish same-named tables in different selected schemas.
- A disposable localhost PostgreSQL 17.10 instance verified unqualified table resolution in two
  schemas, return to the default schema, rejection of unavailable schema names, stop-on-error
  behavior, and isolation after a failed query. No remote database was used.

Validation:

- `cargo test --locked`
- `cargo clippy --locked --all-targets`
- `cargo fmt -- --check`
- `git diff --check`
- `ASTESIA_TEST_PG_PORT=<disposable-local-port> cargo test --locked --lib query_schema_is_isolated_and_resolves_unqualified_tables -- --ignored`

The engine test is ignored by default and requires a disposable local PostgreSQL instance with
user `astesia_test` and database `postgres`. It creates test schemas and tables. The instance used
for this run was removed after verification. Manual native visual comparison remains unverified.
