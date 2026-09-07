#!/usr/bin/env python3
"""Create the disposable 100,000-row fixture used by release memory workloads."""

import argparse
import hashlib
import json
from pathlib import Path
import sqlite3


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    database = args.directory / "fixture.sqlite3"
    script = database.with_suffix(".sql")
    if database.exists() or script.exists():
        parser.error("fixture files already exist; choose a new directory")
    schema = "CREATE TABLE memory_rows (id INTEGER PRIMARY KEY, value INTEGER NOT NULL, payload TEXT NOT NULL)"
    connection = sqlite3.connect(database)
    connection.execute(schema)
    connection.executemany(
        "INSERT INTO memory_rows VALUES (?, ?, ?)",
        ((index, index % 1000, "x" * 1024) for index in range(100_000)),
    )
    connection.commit()
    with script.open("w") as output:
        output.write(schema + ";\n")
        for index, value, payload in connection.execute("SELECT id, value, payload FROM memory_rows ORDER BY id"):
            output.write(f"INSERT INTO memory_rows VALUES ({index},{value},'{payload}');\n")
    connection.close()
    metadata = {
        "rows": 100_000, "payload_bytes": 1024, "sqlite_version": sqlite3.sqlite_version,
        "database_sha256": hashlib.file_digest(database.open("rb"), "sha256").hexdigest(),
        "script_sha256": hashlib.file_digest(script.open("rb"), "sha256").hexdigest(),
    }
    (args.directory / "fixture.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(database)


if __name__ == "__main__":
    main()
