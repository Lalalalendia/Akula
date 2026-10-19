#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/results/corpus"
python3 "$HERE/corpus.py" --tier smoke --output "$OUT/inputs"
cargo build --release --manifest-path "$HERE/Cargo.toml"
BIN="$HERE/target/release/akula-image-bench"
cc -O2 -std=c11 -D_POSIX_C_SOURCE=200809L -o "$OUT/turbo" "$HERE/native/turbo.c" $(pkg-config --cflags --libs libjpeg) -lm
printf 'id,format,mode,attempt,status,elapsed_sec,maxrss_kib,user_sec,system_sec\n' > "$OUT/metrics.csv"
printf 'id,format,mode,attempt,reason\n' > "$OUT/skipped.csv"
while IFS=$(printf '\t') read -r id format width height sha file license bytes; do
  [ "$id" = id ] && continue
  for attempt in 1 2 3; do
    if [ "$attempt" = 2 ]; then order="turbo akula"; else order="akula turbo"; fi
    for mode in $order; do
      if [ "$format" = png ] && [ "$mode" = turbo ]; then
        printf "%s,%s,%s,%s,%s\n" "$id" "$format" "$mode" "$attempt" not_jpeg >> "$OUT/skipped.csv"
        continue
      fi
      if [ "$mode" = turbo ]; then CMD="$OUT/turbo"; else CMD="$BIN"; fi
      set +e
      if [ "$mode" = akula ]; then
        timeout 35s /usr/bin/time -f "$id,$format,$mode,$attempt,ok,%e,%M,%U,%S" -o "$OUT/timing.txt" "$CMD" akula "$file" "$OUT/out.rgba" >> "$OUT/decode.log" 2>&1
      else
        timeout 35s /usr/bin/time -f "$id,$format,$mode,$attempt,ok,%e,%M,%U,%S" -o "$OUT/timing.txt" "$CMD" "$file" "$OUT/out.rgba" >> "$OUT/decode.log" 2>&1
      fi
      result=$?
      set -e
      if [ "$result" = 0 ]; then cat "$OUT/timing.txt" >> "$OUT/metrics.csv"; else
        printf "%s,%s,%s,%s,exit_%s\n" "$id" "$format" "$mode" "$attempt" "$result" >> "$OUT/skipped.csv"; fi
      rm -f "$OUT/out.rgba"
    done
  done
done < "$OUT/inputs/cases.tsv"
echo "Corpus results at $OUT"
