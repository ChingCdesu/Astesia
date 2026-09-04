#!/usr/bin/env python3

import argparse
import os
from pathlib import Path
import re
import tempfile
import tomllib


SEMVER = re.compile(
    r"^(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)


def _astesia_version(document: dict, source: Path) -> str:
    package = document.get("package")
    if isinstance(package, dict):
        if package.get("name") != "astesia" or not isinstance(package.get("version"), str):
            raise ValueError(f"{source} does not define the astesia package version")
        return package["version"]

    packages = [
        entry
        for entry in document.get("package", [])
        if isinstance(entry, dict) and entry.get("name") == "astesia"
    ]
    if len(packages) != 1 or not isinstance(packages[0].get("version"), str):
        raise ValueError(f"{source} must contain exactly one astesia package version")
    return packages[0]["version"]


def _replace_package_version(
    content: str,
    table_header: str,
    current_version: str,
    next_version: str,
    source: Path,
) -> str:
    table_pattern = re.compile(
        rf"(?ms)^(?P<header>{re.escape(table_header)}[ \t]*\r?\n)"
        r"(?P<body>.*?)(?=^\[|\Z)"
    )
    replacement_count = 0

    def replace_table(match: re.Match[str]) -> str:
        nonlocal replacement_count
        body = match.group("body")
        if not re.search(r'^name\s*=\s*"astesia"\s*$', body, re.MULTILINE):
            return match.group(0)
        version_pattern = re.compile(
            rf'^(?P<prefix>version\s*=\s*"){re.escape(current_version)}(?P<suffix>"\s*)$',
            re.MULTILINE,
        )
        body, count = version_pattern.subn(
            rf"\g<prefix>{next_version}\g<suffix>", body
        )
        if count != 1:
            raise ValueError(
                f"{source} must contain exactly one matching astesia version entry"
            )
        replacement_count += 1
        return match.group("header") + body

    updated = table_pattern.sub(replace_table, content)
    if replacement_count != 1:
        raise ValueError(f"{source} must contain exactly one astesia package table")
    return updated


def _prepare_atomic_write(path: Path, content: str) -> Path:
    descriptor, temporary_name = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="") as stream:
            stream.write(content)
        temporary.chmod(path.stat().st_mode)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
    return temporary


def bump_versions(manifest: Path, lockfile: Path) -> str:
    manifest_content = manifest.read_text(encoding="utf-8")
    lockfile_content = lockfile.read_text(encoding="utf-8")
    manifest_version = _astesia_version(tomllib.loads(manifest_content), manifest)
    lockfile_version = _astesia_version(tomllib.loads(lockfile_content), lockfile)
    if manifest_version != lockfile_version:
        raise ValueError(
            f"Astesia version mismatch: {manifest} has {manifest_version}, "
            f"but {lockfile} has {lockfile_version}"
        )
    match = SEMVER.fullmatch(manifest_version)
    if match is None:
        raise ValueError(f"Unexpected Astesia version format: {manifest_version}")
    next_version = ".".join(
        (
            match.group("major"),
            match.group("minor"),
            str(int(match.group("patch")) + 1),
        )
    )
    updated_manifest = _replace_package_version(
        manifest_content, "[package]", manifest_version, next_version, manifest
    )
    updated_lockfile = _replace_package_version(
        lockfile_content, "[[package]]", lockfile_version, next_version, lockfile
    )

    manifest_temporary = _prepare_atomic_write(manifest, updated_manifest)
    try:
        lockfile_temporary = _prepare_atomic_write(lockfile, updated_lockfile)
    except BaseException:
        manifest_temporary.unlink(missing_ok=True)
        raise
    try:
        os.replace(lockfile_temporary, lockfile)
        os.replace(manifest_temporary, manifest)
    finally:
        manifest_temporary.unlink(missing_ok=True)
        lockfile_temporary.unlink(missing_ok=True)
    return next_version


def main() -> None:
    repository_root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(
        description="Bump Astesia's Cargo package and lockfile patch version together."
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=repository_root / "src-tauri" / "Cargo.toml",
    )
    parser.add_argument(
        "--lockfile",
        type=Path,
        default=repository_root / "src-tauri" / "Cargo.lock",
    )
    arguments = parser.parse_args()
    print(bump_versions(arguments.manifest, arguments.lockfile))


if __name__ == "__main__":
    main()
