#!/usr/bin/env bash
# Experimental comparison: same host Rust build/tests on one GitHub runner.
# Keep real signed-guest acceptance after this script. Never write a PR cache.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != true || "${RUNNER_OS:-}" != Linux ||
      "${GITHUB_HEAD_REF:-}" != ci/rust-cache-three-mode-b1b2b-20261022 ]]; then
  echo 'CACHE_BENCH_REFUSED: only the dedicated PR experiment can run this' >&2
  exit 2
fi
if [[ -z "${RUNNER_TEMP:-}" || ! -x /usr/bin/time ]]; then
  echo 'CACHE_BENCH_REFUSED: runner timer unavailable' >&2
  exit 2
fi
cd "$(dirname "$0")/../../aquilum-app"
root="$PWD"
base_home="${CARGO_HOME:-$HOME/.cargo}"
experiment="$RUNNER_TEMP/akula-b1b2b-cache-compare"
slim_home="$experiment/registry-home"
slim_target="$experiment/registry-target"
cold_home="$experiment/none-home"
cold_target="$experiment/none-target"
mkdir -p "$slim_home/registry" "$cold_home"
rustfmt --edition 2021 core/examples/akula_b1b2b_release_fixture.rs
echo "CACHE_BENCH_SHA=$(git rev-parse HEAD)"
echo "CACHE_BENCH_RUSTC=$(rustc --version)"
echo "CACHE_BENCH_CARGO=$(cargo --version)"
echo "CACHE_BENCH_SHARED_CACHE_HIT=${AKULA_CACHE_HIT:-unknown}"

# Snapshot only dependency archives and git sources from the trusted main
# cache. No target artifacts or workspace binaries enter registry-only mode.
# The copy/staging time is logged separately, NOT called network restore time.
stage_start=$SECONDS
if [[ -d "$base_home/registry/cache" ]]; then
  cp -a "$base_home/registry/cache" "$slim_home/registry/cache"
fi
if [[ -d "$base_home/registry/index" ]]; then
  cp -a "$base_home/registry/index" "$slim_home/registry/index"
fi
if [[ -d "$base_home/git" ]]; then
  cp -a "$base_home/git" "$slim_home/git"
fi
stage_secs=$((SECONDS - stage_start))
slim_bytes=$(du -sb "$slim_home" | cut -f1)
full_bytes=$(du -sb "$base_home/registry" "$base_home/git" 2>/dev/null | awk '{sum+=$1} END {print sum+0}')
target_bytes=$(du -sb "$root/../.artifacts/cargo-native" | cut -f1)
echo "CACHE_BENCH_STAGE_SECONDS=$stage_secs"
echo "CACHE_BENCH_REGISTRY_BYTES=$slim_bytes"
echo "CACHE_BENCH_FULL_TARGET_BYTES=$target_bytes"
echo "CACHE_BENCH_ORIGIN_DEP_BYTES=$full_bytes"

run_mode(){
  local mode="$1" home="$2" target="$3"
  local logfile="$experiment/$mode.log" timing="$experiment/$mode.time"
  local started=$SECONDS
  mkdir -p "$target"
  echo "CACHE_BENCH_START mode=$mode home=$home target=$target"
  if ! env CARGO_HOME="$home" CARGO_TARGET_DIR="$target" CARGO_INCREMENTAL=0 \
       /usr/bin/time -f 'wall=%e user=%U system=%S rss_kb=%M' -o "$timing" \
       bash -euo pipefail -c '
         cargo build --locked -p aquilum-core --features node-lab --bin akula-node-lab
         cargo build --locked -p aquilum-core --features server-bootstrap-b1a-release-lab --example akula_b1b2b_release_fixture
         cargo test --locked -p aquilum-core --features server-bootstrap-b1a-release-lab --test server_bootstrap_b1a -- --nocapture
       ' > "$logfile" 2>&1; then
    echo "CACHE_BENCH_FAILURE mode=$mode"
    tail -n 110 "$logfile"
    exit 1
  fi
  local elapsed=$((SECONDS - started))
  local disk_bytes
  disk_bytes=$(du -sb "$target" | cut -f1)
  echo "CACHE_BENCH_RESULT mode=$mode seconds=$elapsed $(cat "$timing") target_bytes=$disk_bytes"
  tail -n 14 "$logfile"
  printf '| %s | %s | %s | %s |\n' "$mode" "$elapsed" "$disk_bytes" "$(sed 's/|/ /g' "$timing")" >> "$experiment/summary.md"
}

# The full trusted cache was restored by the existing job step. This is the
# same path needed later by the unchanged real signed QEMU lab.
run_mode full "$base_home" "$root/../.artifacts/cargo-native"
run_mode registry-only "$slim_home" "$slim_target"
run_mode no-cache "$cold_home" "$cold_target"

{
  echo '### B1b2b Rust cache comparison (single runner, three identical host suites)'
  echo ''
  echo "Commit: $(git rev-parse --short HEAD); compiler: $(rustc --version)"
  echo ''
  echo '| Mode | Build+tests seconds | Target bytes | Time metrics |'
  echo '| --- | ---: | ---: | --- |'
  cat "$experiment/summary.md"
  echo ''
  echo "Trusted full cache hit: ${AKULA_CACHE_HIT:-unknown}. Registry snapshot: $slim_bytes bytes; copied locally in $stage_secs s."
  echo 'Full restore network time is measured by the existing rust-cache GitHub Action step.'
  echo 'Registry-only mode uses a local dependency snapshot; its *network restore time is NOT measured*.'
  echo 'No-cache mode starts with an empty CARGO_HOME and target and may download dependencies.'
  echo 'The following unchanged QEMU/guest step remains the release/security acceptance.'
} >> "$GITHUB_STEP_SUMMARY"
echo 'CACHE_BENCH_PASS: all three host suites passed; proceeding to real VM'
