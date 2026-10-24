#!/usr/bin/env bash
set -euo pipefail
DEST="${RUNNER_TEMP:-/tmp}/language-compare-toolchains"
mkdir -p "$DEST"

download() {
  local language="$1" url="$2" path="$3"
  if ! curl --fail --location --silent --show-error --retry 3 --retry-delay 2 --connect-timeout 20 --max-time 240 "$url" -o "$path"; then
    echo "Optional toolchain unavailable: $language" >&2
    return 1
  fi
  echo "Downloaded $language: $(sha256sum "$path")"
}

ZIG_URL="https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz"
if download zig "$ZIG_URL" "$DEST/zig.tar.xz"; then
  if echo '1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026  '"$DEST/zig.tar.xz" | sha256sum -c -; then
    mkdir -p "$DEST/zig"
    tar -xJf "$DEST/zig.tar.xz" -C "$DEST/zig" --strip-components=1
    echo "$DEST/zig" >> "$GITHUB_PATH"
  else echo "Zig archive SHA256 mismatch; skipped" >&2; fi
fi

ODIN_URL="https://github.com/odin-lang/Odin/releases/download/dev-2026-10/odin-linux-amd64-dev-2026-10.tar.gz"
if download odin "$ODIN_URL" "$DEST/odin.tar.gz"; then
  mkdir -p "$DEST/odin"
  tar -xzf "$DEST/odin.tar.gz" -C "$DEST/odin"
  p="$(find "$DEST/odin" -type f -name odin -perm /111 | head -n 1 || true)"
  if [ -n "$p" ]; then echo "$(dirname "$p")" >> "$GITHUB_PATH"; else echo "odin executable not found" >&2; fi
fi

C3_URL="https://github.com/c3lang/c3c/releases/download/v0.8.5/c3-linux.tar.gz"
if download c3 "$C3_URL" "$DEST/c3.tar.gz"; then
  mkdir -p "$DEST/c3"
  tar -xzf "$DEST/c3.tar.gz" -C "$DEST/c3"
  p="$(find "$DEST/c3" -type f -name c3c -perm /111 | head -n 1 || true)"
  if [ -n "$p" ]; then echo "$(dirname "$p")" >> "$GITHUB_PATH"; else echo "c3c executable not found" >&2; fi
fi
