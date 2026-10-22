#!/usr/bin/env bash
# Research-only standalone comparison. Main and the production renderer are untouched.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
REPORT="$ROOT/.rgba-comparison-v2"
SRC="scripts/benchmarks/zig-rgba-ab"
mkdir -p "$REPORT"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
URL="https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz"
EXPECTED="1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026"
curl -fLsS --retry 3 --connect-timeout 20 "$URL" -o "$TMP/zig.tar.xz"
printf '%s  %s\n' "$EXPECTED" "$TMP/zig.tar.xz" | sha256sum -c -
tar -xJf "$TMP/zig.tar.xz" -C "$TMP"
ZIG="$TMP/zig-x86_64-linux-0.17.0/zig"
{
  echo "GITHUB_SHA=$GITHUB_SHA"
  echo "GITHUB_RUN_ID=$GITHUB_RUN_ID"
  echo "CPU=$(grep -m1 'model name' /proc/cpuinfo || true)"
  echo "RUST_GENERIC=opt3 target-cpu=x86-64"
  echo "RUST_NATIVE=opt3 target-cpu=native"
  echo "ZIG_GENERIC=ReleaseFast target=x86_64-linux-gnu"
  echo "ZIG_NATIVE=ReleaseFast target=x86_64-linux-gnu mcpu=native"
  rustc --version --verbose
  "$ZIG" version
  objdump --version | head -1
  python3 --version
  echo "ZIG_SHA256=$EXPECTED"
} | tee "$REPORT/environment.txt"
/usr/bin/time -f 'rust_generic_wall_s=%e\nrust_generic_max_rss_kib=%M' \
  -o "$REPORT/rust-generic-build.txt" \
  rustc --edition=2021 --crate-type=cdylib -C opt-level=3 \
    -C target-cpu=x86-64 "$SRC/rgba-optimized.rs" \
    -o "$REPORT/rust_generic.so"
/usr/bin/time -f 'rust_native_wall_s=%e\nrust_native_max_rss_kib=%M' \
  -o "$REPORT/rust-native-build.txt" \
  rustc --edition=2021 --crate-type=cdylib -C opt-level=3 \
    -C target-cpu=native "$SRC/rgba-optimized.rs" \
    -o "$REPORT/rust_native.so"
/usr/bin/time -f 'zig_generic_wall_s=%e\nzig_generic_max_rss_kib=%M' \
  -o "$REPORT/zig-generic-build.txt" \
  "$ZIG" build-lib "$SRC/rgba.zig" -dynamic -O ReleaseFast \
    -target x86_64-linux-gnu -femit-bin="$REPORT/zig_generic.so"
/usr/bin/time -f 'zig_native_wall_s=%e\nzig_native_max_rss_kib=%M' \
  -o "$REPORT/zig-native-build.txt" \
  "$ZIG" build-lib "$SRC/rgba.zig" -dynamic -O ReleaseFast \
    -target x86_64-linux-gnu -mcpu=native \
    -femit-bin="$REPORT/zig_native.so"
stat -c '%n %s bytes' "$REPORT"/*.so | tee "$REPORT/library-sizes.txt"
python3 "$SRC/asm-v2.py" | tee "$REPORT/assembly.log"
for repeat in 0 1 2 3 4; do
  python3 "$SRC/bench-v2.py" \
    --rust-generic "$REPORT/rust_generic.so" \
    --rust-native "$REPORT/rust_native.so" \
    --zig-generic "$REPORT/zig_generic.so" \
    --zig-native "$REPORT/zig_native.so" \
    --repetition "$repeat" --output "$REPORT/result-$repeat.json" \
    | tee "$REPORT/bench-$repeat.log"
done
python3 "$SRC/summarize-v2.py" "$REPORT"/result-*.json | tee "$REPORT/summary.txt"
cat "$REPORT"/*-build.txt
