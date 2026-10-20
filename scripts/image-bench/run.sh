#!/usr/bin/env bash
# Only deterministic locally generated JPEGs; not a hostile-file parser.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WORK="${BENCH_OUTPUT_DIR:-$HERE/results}"
REPEATS="${REPEATS:-3}"
LARGE="${LARGE:-0}"
case "$REPEATS" in 1|2|3|4|5|6|7|8|9|10) ;; *) echo "REPEATS must be 1..10" >&2; exit 2;; esac
for tool in cargo cc pkg-config cjpeg /usr/bin/time; do
  command -v "$tool" >/dev/null || { echo "Missing: $tool" >&2; exit 2; }
done
pkg-config --exists libjpeg && pkg-config --exists vips || {
  echo "Requires libjpeg and vips development headers/pkg-config" >&2; exit 2;
}
mkdir -p "$WORK/build" "$WORK/data"
cargo build --release --manifest-path "$HERE/Cargo.toml"
BIN="$HERE/target/release/akula-image-bench"
cc -O2 -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -o "$WORK/build/turbo" \
  "$HERE/native/turbo.c" $(pkg-config --cflags --libs libjpeg) -lm
cc -O2 -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -o "$WORK/build/vips" \
  "$HERE/native/vips.c" $(pkg-config --cflags --libs vips) -lm
printf 'fixture,mode,iteration,elapsed_sec,maxrss_kib,user_sec,system_sec\n' > "$WORK/metrics.csv"
{
  printf 'git_sha=%s\n' "$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)"
  printf 'rustc=%s\n' "$(rustc --version)"
  printf 'libvips=%s\n' "$(pkg-config --modversion vips)"
  printf 'libjpeg=%s\n' "$(pkg-config --modversion libjpeg)"
  printf 'uname=%s\n' "$(uname -a)"
  printf 'repeats=%s\n' "$REPEATS"
  printf 'large_enabled=%s\n' "$LARGE"
  if command -v lscpu >/dev/null; then lscpu; fi
} > "$WORK/environment.txt"
sizes=("4096:3072")
if [[ "$LARGE" == 1 ]]; then sizes+=("8000:6000"); fi
for dims in "${sizes[@]}"; do
  IFS=: read -r w h <<< "$dims"
  ppm="$WORK/data/image-${w}x${h}.ppm"
  "$BIN" generate "$ppm" "$w" "$h"
  for variant in baseline progressive; do
    src="$WORK/data/image-${w}x${h}-${variant}.jpg"
    if [[ "$variant" == progressive ]]; then
      cjpeg -progressive -quality 87 -outfile "$src" "$ppm"
    else
      cjpeg -quality 87 -outfile "$src" "$ppm"
    fi
    sha256sum "$src" >> "$WORK/fixtures.sha256"
    echo "Fixture $src bytes=$(wc -c < "$src")"
    for iteration in $(seq 1 "$REPEATS"); do
      for mode in akula turbo vips; do
        case "$mode" in
          akula) cmd=("$BIN" akula);;
          turbo) cmd=("$WORK/build/turbo");;
          vips) cmd=("$WORK/build/vips");;
        esac
        rgba="$WORK/data/$mode.rgba"
        times="$WORK/data/time.txt"
        /usr/bin/time -f "$variant-${w}x${h},$mode,$iteration,%e,%M,%U,%S" -o "$times" \
          "${cmd[@]}" "$src" "$rgba" | tee -a "$WORK/decode-info.txt"
        cat "$times" >> "$WORK/metrics.csv"
        if [[ "$iteration" == 1 ]]; then
          cp "$rgba" "$WORK/data/${variant}-${w}x${h}-$mode.rgba"
        fi
      done
    done
    "$BIN" compare "$WORK/data/${variant}-${w}x${h}-akula.rgba" \
      "$WORK/data/${variant}-${w}x${h}-turbo.rgba" | tee -a "$WORK/quality.txt"
    "$BIN" compare "$WORK/data/${variant}-${w}x${h}-akula.rgba" \
      "$WORK/data/${variant}-${w}x${h}-vips.rgba" | tee -a "$WORK/quality.txt"
    rm -f "$WORK/data/"*.rgba
  done
  rm -f "$ppm"
done
echo "Results: $WORK"
