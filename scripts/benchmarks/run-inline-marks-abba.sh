#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
source_dir="aquilum-app/native/src/ui/editor/markdown/inline/marks"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/baseline" "$scratch/candidate"
# Compare the PR tree with the current target branch in one Rust binary.
# The baseline is fetched as exact source bytes, not hand-reimplemented.
git fetch --no-tags origin main
base_ref="$(git rev-parse origin/main)"
echo "Baseline main: $base_ref"
echo "Candidate HEAD: $(git rev-parse HEAD)"
for file in atoms.rs delimited.rs; do
  git show "${base_ref}:${source_dir}/${file}" > "$scratch/baseline/$file"
  cp "$repo_root/$source_dir/$file" "$scratch/candidate/$file"
done

echo "Rust compiler: $(rustc --version)"
BENCH_BASE_DIR="$scratch/baseline" BENCH_CAND_DIR="$scratch/candidate" \
  rustc --edition=2024 -C opt-level=3 -C target-cpu=x86-64 \
  "$repo_root/scripts/benchmarks/inline_marks_abba.rs" \
  -o "$scratch/inline-marks-bench"
"$scratch/inline-marks-bench"
