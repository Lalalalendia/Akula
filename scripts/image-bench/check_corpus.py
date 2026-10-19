"""Strict acceptance for a reproducible JPEG/PNG decoder matrix."""
import argparse
import csv
import statistics
from pathlib import Path

def read(path, *, delimiter=","):
    with Path(path).open("r", encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f, delimiter=delimiter))

def expected_modes(fmt):
    if fmt == "jpg": return ("akula", "turbo", "vips")
    if fmt == "png": return ("akula", "vips")
    raise ValueError(f"unknown format: {fmt}")

def verify(cases, metrics, failures, repeats):
    """Return a list of errors, and the per-(id,mode) numeric measurements."""
    errors = []
    cases_by_id = {c["id"]: c for c in cases}
    if len(cases_by_id) != len(cases): errors.append("duplicate input case id")
    expected = {(c["id"], m, str(n)) for c in cases
                for m in expected_modes(c["format"])
                for n in range(1, repeats+1)}
    seen = {}
    for row in metrics:
        key = (row["id"], row["mode"], row["attempt"])
        if key in seen: errors.append(f"duplicate successful run: {key}")
        if key not in expected: errors.append(f"unexpected run: {key}")
        if row["id"] in cases_by_id and row["format"] != cases_by_id[row["id"]]["format"]:
            errors.append(f"format mismatch for {key}")
        if row["status"] != "ok": errors.append(f"non-ok status for {key}")
        try:
            wall = float(row["elapsed_sec"]); rss = int(row["maxrss_kib"])
            if wall < 0 or rss <= 0: raise ValueError
        except (ValueError, KeyError): errors.append(f"invalid time/RSS for {key}")
        seen[key] = row
    for key in sorted(expected - set(seen)): errors.append(f"missing required success: {key}")
    for row in failures:
        errors.append(f"recorded failure: {(row.get('id'), row.get('mode'), row.get('attempt'))} {row.get('reason')}")
    return errors

def summarize(cases, metrics, out):
    by_key = {}
    for row in metrics: by_key.setdefault((row["id"], row["format"], row["mode"]), []).append(row)
    with Path(out).open("w", encoding="utf-8", newline="") as f:
        writer=csv.writer(f)
        writer.writerow(("id","format","mode","trials","median_wall_sec","median_maxrss_kib"))
        for key, vals in sorted(by_key.items()):
            writer.writerow((*key, len(vals), statistics.median(float(x["elapsed_sec"]) for x in vals),
                             statistics.median(int(x["maxrss_kib"]) for x in vals)))

def main():
    p=argparse.ArgumentParser()
    p.add_argument("--cases",required=True);p.add_argument("--metrics",required=True)
    p.add_argument("--failures",required=True);p.add_argument("--repeats",type=int,default=3)
    p.add_argument("--summary",required=True)
    a=p.parse_args()
    cases=read(a.cases,delimiter="\t");metrics=read(a.metrics);failures=read(a.failures)
    errors=verify(cases,metrics,failures,a.repeats)
    summarize(cases,metrics,a.summary)
    if errors:
        for err in errors: print(f"MATRIX FAIL: {err}")
        raise SystemExit(1)
    print(f"MATRIX PASS: {len(cases)} images x {a.repeats} trials, all expected decoders succeeded")

if __name__=="__main__":main()
