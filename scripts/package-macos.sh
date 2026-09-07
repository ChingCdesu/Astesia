#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$repository_root/Cargo.toml"
toolchain="1.97.1"
target="${1:-$(rustup run "$toolchain" rustc -vV | awk '/^host:/ { print $2 }')}"

case "$target" in
  aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *)
    echo "Unsupported macOS target: $target" >&2
    exit 2
    ;;
esac

version="$(awk -F ' = ' '/^version = / { gsub(/\"/, "", $2); print $2; exit }' "$manifest")"
target_root="${CARGO_TARGET_DIR:-$repository_root/target}"
release_dir="$target_root/$target/release"
package_dir="$target_root/package"
archive="$package_dir/astesia-$version-$target.zip"
stage="$(mktemp -d)"
bundle="$stage/Astesia.app"
trap 'rm -rf "$stage"' EXIT

rustup target add --toolchain "$toolchain" "$target"
rustup run "$toolchain" cargo build --release --locked --manifest-path "$manifest" --target "$target" \
  --bin astesia --bin astesia-mcp

mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$package_dir"
install -m 755 "$release_dir/astesia" "$bundle/Contents/MacOS/astesia"
install -m 755 "$release_dir/astesia-mcp" "$bundle/Contents/MacOS/astesia-mcp"
install -m 644 "$repository_root/icons/icon.icns" \
  "$bundle/Contents/Resources/icon.icns"
sed "s/@VERSION@/$version/g" "$repository_root/packaging/macos/Info.plist.in" \
  > "$bundle/Contents/Info.plist"

codesign --force --deep --sign "${CODESIGN_IDENTITY:--}" "$bundle"
codesign --verify --deep --strict "$bundle"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$bundle" "$archive"
echo "$archive"
