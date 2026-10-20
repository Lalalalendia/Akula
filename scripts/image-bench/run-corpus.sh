#!/usr/bin/env bash
# Benchmark vetted real images. A successful CI job requires EVERY expected cell.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${BENCH_OUTPUT_DIR:-$HERE/results/corpus}"
TIER="${CORPUS_TIER:-smoke}"
REPEATS="${REPEATS:-3}"
case "$TIER" in smoke|extended|stress) ;; *) echo "invalid corpus tier: $TIER (expected smoke, extended or stress)" >&2; exit 2;; esac
case "$REPEATS" in 1|2|3|4|5|6|7|8|9|10) ;; *) echo "repeats 1..10 required" >&2; exit 2;; esac
# Validate the dispatch contract offline before downloading external files.
# The same validator runs in CI unit tests for smoke/extended/stress.
if [[ "${BENCH_VALIDATE_CONFIG_ONLY:-0}" == 1 ]]; then
  printf "BENCH_CONFIG_OK tier=%s repeats=%s\n" "$TIER" "$REPEATS"
  exit 0
fi
mkdir -p "$OUT" "$OUT/inputs" "$OUT/reference"
: > "$OUT/decode.log"; : > "$OUT/quality.txt"
printf 'id,format,mode,attempt,status,elapsed_sec,maxrss_kib,user_sec,system_sec\n' > "$OUT/metrics.csv"
printf 'id,format,mode,attempt,reason\n' > "$OUT/failures.csv"
python3 "$HERE/corpus.py" --tier "$TIER" --output "$OUT/inputs"
if [[ "$TIER" == stress ]]; then
  jpegtran -progressive -copy all -outfile "$OUT/inputs/1407-progressive.jpg" "$OUT/inputs/1407.jpg"
  python3 "$HERE/jpeg_markers.py" --cases "$OUT/inputs/cases.tsv" --source-id 1407 --derived "$OUT/inputs/1407-progressive.jpg" --record "$OUT/progressive-provenance.json"
fi
cargo build --release --manifest-path "$HERE/Cargo.toml"
BIN="$HERE/target/release/akula-image-bench"
cc -O2 -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -o "$OUT/turbo" "$HERE/native/turbo.c" $(pkg-config --cflags --libs libjpeg) -lm
cc -O2 -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -o "$OUT/vips" "$HERE/native/vips.c" $(pkg-config --cflags --libs vips) -lm
"$OUT/turbo" --check-portrait-fit | tee -a "$OUT/decode.log"
printf "repo_sha=%s\nrustc=%s\nlibvips=%s\nlibjpeg=%s\nuname=%s\n" "$(git -C "$HERE/../.." rev-parse HEAD)" "$(rustc --version)" "$(pkg-config --modversion vips)" "$(pkg-config --modversion libjpeg)" "$(uname -a)" > "$OUT/environment.txt"
while IFS="$(printf '\t')" read -r id format width height sha file license bytes; do
  [[ "$id" == "id" ]] && continue
  case "$format" in
    jpg) modes=(akula turbo vips) ;;
    png) modes=(akula vips) ;;
    *) echo "unsupported format in manifest: $format" >&2; exit 2 ;;
  esac
  for attempt in $(seq 1 "$REPEATS"); do
    count="${#modes[@]}"
    # Rotate the testing order to moderate page-cache and thermal bias.
    shift_count=$((attempt % count))
    for ((p=0;p<count;p++)); do
      pos=$(((p+shift_count)%count))
      mode="${modes[$pos]}"
      dest="$OUT/$id-$mode.rgba"
      rm -f "$dest"
      case "$mode" in
        akula) cmd=("$BIN" akula "$file" "$dest") ;;
        turbo|vips) cmd=("$OUT/$mode" "$file" "$dest") ;;
      esac
      set +e
      timeout 90s prlimit --as=1610612736 -- /usr/bin/time -f "$id,$format,$mode,$attempt,ok,%e,%M,%U,%S" -o "$OUT/one_time.txt" "${cmd[@]}" >> "$OUT/decode.log" 2>&1
      result=$?
      set -e
      if [[ "$result" == 0 && -s "$dest" ]]; then
        cat "$OUT/one_time.txt" >> "$OUT/metrics.csv"
        if [[ "$attempt" == 1 ]]; then mv "$dest" "$OUT/reference/$id-$mode.rgba";
        else rm -f "$dest"; fi
      else
        printf "%s,%s,%s,%s,exit_%s\n" "$id" "$format" "$mode" "$attempt" "$result" >> "$OUT/failures.csv"
        rm -f "$dest"
        echo "FAILED id=$id mode=$mode attempt=$attempt exit=$result" | tee -a "$OUT/decode.log" >&2
      fi
    done
  done
  a="$OUT/reference/$id-akula.rgba"
  if [[ -f "$a" ]]; then
    for mode in "${modes[@]}"; do
      [[ "$mode" == akula ]] && continue
      b="$OUT/reference/$id-$mode.rgba"
      if [[ -f "$b" ]]; then
        if ! "$BIN" compare "$a" "$b" >> "$OUT/quality.txt" 2>&1; then
          printf "%s,%s,%s,%s,%s\n" "$id" "$format" "$mode" 0 quality_mismatch >> "$OUT/failures.csv"
        fi
      fi
    done
  fi
done < "$OUT/inputs/cases.tsv"
python3 "$HERE/check_corpus.py" --cases "$OUT/inputs/cases.tsv" --metrics "$OUT/metrics.csv" --failures "$OUT/failures.csv" --repeats "$REPEATS" --summary "$OUT/summary.csv"
echo "Corpus results at $OUT"
