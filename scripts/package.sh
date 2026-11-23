#!/usr/bin/env bash
# Builds one download into <target>/dist/: scripts/package.sh macos-universal|windows-x64|linux-x64
set -euo pipefail

cd "$(dirname "$0")/.."

# jq.exe writes CRLF line endings on Windows.
json() { jq -r "$1" | tr -d '\r'; }
# cargo prints Windows paths on Windows; Git Bash's tools need Unix ones.
unix_path() { if command -v cygpath >/dev/null; then cygpath -u "$1"; else printf '%s\n' "$1"; fi; }
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d ' ' -f 1; }

platform="${1:-}"
case "$platform" in
macos-universal)
    targets=(aarch64-apple-darwin x86_64-apple-darwin)
    pdfium_archive=pdfium-mac-univ.tgz
    pdfium_sha256=5484abb015b38e7b2598f0ac94c6fb889b5bc12f29dd5660693c8e8d8e57bcb1
    pdfium_library=lib/libpdfium.dylib
    exe=
    ;;
windows-x64)
    targets=(x86_64-pc-windows-msvc)
    pdfium_archive=pdfium-win-x64.tgz
    pdfium_sha256=0b08b606792a6cc593426efdefc6622611bce446d9e0270743846956ea1554ca
    pdfium_library=bin/pdfium.dll
    exe=.exe
    # Links the C runtime into the .exe, so it runs without the Visual C++ Redistributable.
    export RUSTFLAGS="-C target-feature=+crt-static"
    ;;
linux-x64)
    targets=(x86_64-unknown-linux-gnu)
    pdfium_archive=pdfium-linux-x64.tgz
    pdfium_sha256=9329a3c4b19b3c8d0a93af5440f44be84e4bd879a204e47b1a7a160e96809da4
    pdfium_library=lib/libpdfium.so
    exe=
    ;;
*)
    echo "usage: scripts/package.sh macos-universal|windows-x64|linux-x64" >&2
    exit 2
    ;;
esac

for tool in jq cargo-about curl; do
    command -v "$tool" >/dev/null || { echo "scripts/package.sh needs $tool" >&2; exit 1; }
done

metadata=$(cargo metadata --format-version 1 --no-deps --locked)
version=$(json '.packages[0].version' <<<"$metadata")
target_dir=$(unix_path "$(json '.target_directory' <<<"$metadata")")
name="bookserver-server-$version-$platform"
dist="$target_dir/dist"
out="$dist/$name"
rm -rf "$out" "$dist/$name.zip" "$dist/$name.tar.gz"
mkdir -p "$out/pdfium"

for target in "${targets[@]}"; do
    cargo build --release --locked --target "$target"
done
if [ "$platform" = macos-universal ]; then
    lipo -create -output "$out/bookserver" \
        "$target_dir/aarch64-apple-darwin/release/bookserver_backend" \
        "$target_dir/x86_64-apple-darwin/release/bookserver_backend"
else
    cp "$target_dir/${targets[0]}/release/bookserver_backend$exe" "$out/bookserver$exe"
fi

archive="$dist/$pdfium_archive"
curl -fsSL -o "$archive" "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7543/$pdfium_archive"
# Pinned, so a swapped or corrupted archive fails the build instead of shipping.
if [ "$(sha256 "$archive")" != "$pdfium_sha256" ]; then
    echo "$pdfium_archive doesn't match its pinned SHA-256" >&2
    exit 1
fi
unpacked="$dist/pdfium-$platform"
rm -rf "$unpacked"
mkdir -p "$unpacked"
tar -xzf "$archive" -C "$unpacked"
cp "$unpacked/$pdfium_library" "$out/"
cp -R "$unpacked/LICENSE" "$unpacked/licenses" "$out/pdfium/"

if [ "$platform" = macos-universal ]; then
    # Apple Silicon refuses to run unsigned code; `-` signs ad hoc, with no Apple team.
    codesign --force --sign - "$out/bookserver" "$out/libpdfium.dylib"
fi

cp LICENSE packaging/README.txt "$out/"
cargo about generate --locked --fail about.hbs -o "$out/THIRD_PARTY_LICENSES.html"
# Apache-2.0 asks for a crate's NOTICE file to ship with the binary.
cargo about generate --locked --fail --format json |
    json '.crates[].package | "\(.name)-\(.version)\t\(.manifest_path)"' |
    while IFS=$'\t' read -r crate manifest; do
        for notice in "$(dirname "$(unix_path "$manifest")")"/NOTICE*; do
            [ -f "$notice" ] || continue
            mkdir -p "$out/notices/$crate"
            cp "$notice" "$out/notices/$crate/"
        done
    done

cd "$dist"
if [ "$platform" = linux-x64 ]; then
    package="$name.tar.gz"
    tar -czf "$package" "$name"
elif command -v zip >/dev/null; then
    package="$name.zip"
    zip -qr "$package" "$name"
else
    package="$name.zip"
    7z a -tzip "$package" "$name" >/dev/null
fi
echo "$dist/$package"
