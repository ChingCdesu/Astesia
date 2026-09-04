#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$repository_root/src-tauri/Cargo.toml"
toolchain="1.97.1"
target="${1:-x86_64-unknown-linux-gnu}"

case "$target" in
  x86_64-unknown-linux-gnu) ;;
  *)
    echo "Unsupported Linux target: $target" >&2
    exit 2
    ;;
esac

version="$(awk -F ' = ' '/^version = / { gsub(/\"/, "", $2); print $2; exit }' "$manifest")"
target_root="${CARGO_TARGET_DIR:-$repository_root/src-tauri/target}"
release_dir="$target_root/$target/release"
package_dir="$target_root/package"
archive="$package_dir/astesia-$version-$target.tar.gz"
stage="$(mktemp -d)"
bundle="$stage/Astesia"
trap 'rm -rf "$stage"' EXIT

rustup target add --toolchain "$toolchain" "$target"
rustup run "$toolchain" cargo build --release --locked --manifest-path "$manifest" --target "$target" \
  --bin astesia --bin astesia-mcp

mkdir -p "$bundle/bin" \
  "$bundle/share/applications" \
  "$bundle/share/icons/hicolor/512x512/apps" \
  "$package_dir"
install -m 755 "$release_dir/astesia" "$bundle/bin/astesia"
install -m 755 "$release_dir/astesia-mcp" "$bundle/bin/astesia-mcp"
install -m 644 "$repository_root/packaging/linux/com.astesia.app.desktop" \
  "$bundle/share/applications/com.astesia.app.desktop"
install -m 644 "$repository_root/src-tauri/icons/icon.png" \
  "$bundle/share/icons/hicolor/512x512/apps/com.astesia.app.png"

rm -f "$archive"
tar -czf "$archive" -C "$stage" Astesia
echo "$archive"
