#!/usr/bin/env bash
# One-shot research job on a branch. Does not alter Akula runtime or PR checks.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
REPORT="$ROOT/.rgba-comparison"
mkdir -p "$REPORT"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
URL="https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz"
EXPECTED="1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026"
curl -fLsS --retry 3 --connect-timeout 20 "$URL" -o "$TMP/zig.tar.xz"
printf '%s  %s\n' "$EXPECTED" "$TMP/zig.tar.xz" | sha256sum -c -
tar -xJf "$TMP/zig.tar.xz" -C "$TMP"
ZIG="$TMP/zig-x86_64-linux-0.17.0/zig"
RUST="rustc"
{
  echo "GITHUB_SHA=$GITHUB_SHA"
  echo "RUNNER=$RUNNER_NAME"
  echo "CPU=$(grep -m1 'model name' /proc/cpuinfo || true)"
  echo "ARCH=$(uname -m)"
  "$RUST" --version
  "$ZIG" version
  python3 --version
  echo "ZIG_SHA256=$EXPECTED"
} | tee "$REPORT/environment.txt"
SRC="scripts/benchmarks/zig-rgba-ab"
RUST_LIB="$REPORT/librust_rgba.so"
ZIG_LIB="$REPORT/libzig_rgba.so"
/usr/bin/time -f 'rust_build_wall_s=%e\nrust_build_max_rss_kib=%M' \
  -o "$REPORT/rust-build.txt" \
  "$RUST" --edition=2021 --crate-type=cdylib -C opt-level=3 -C target-cpu=x86-64 \
  "$SRC/rgba.rs" -o "$RUST_LIB"
/usr/bin/time -f 'zig_build_wall_s=%e\nzig_build_max_rss_kib=%M' \
  -o "$REPORT/zig-build.txt" \
  "$ZIG" build-lib "$SRC/rgba.zig" -dynamic -O fast \
  -target x86_64-linux-gnu -femit-bin="$ZIG_LIB"
stat -c 'rust_shared_object_bytes=%s' "$RUST_LIB" > "$REPORT/library-sizes.txt"
stat -c 'zig_shared_object_bytes=%s' "$ZIG_LIB" >> "$REPORT/library-sizes.txt"
python3 "$SRC/bench.py" --rust "$RUST_LIB" --zig "$ZIG_LIB" \
  --report "$REPORT/results.json" | tee "$REPORT/bench.log"
: > "$REPORT/rss.jsonl"
for arm in rust_scalar zig_scalar rust_packed zig_packed; do
  python3 "$SRC/bench.py" --rust "$RUST_LIB" --zig "$ZIG_LIB" \
    --rss-one "$arm" >> "$REPORT/rss.jsonl"
done
cat "$REPORT/rust-build.txt" "$REPORT/zig-build.txt" "$REPORT/library-sizes.txt"
cat "$REPORT/rss.jsonl"
