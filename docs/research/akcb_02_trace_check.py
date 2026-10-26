#!/usr/bin/env python3
"""AKCB-02: independent, bounded oracle for actual Rust Sync Planner outcomes.

NOT a proof of File Gate mutation, durable ACK, or Host Broker admission.
Usage: python3 docs/research/akcb_02_trace_check.py target/akcb_02.json --self-test
"""

from __future__ import annotations

import argparse
from collections import Counter
from itertools import product
import json
from pathlib import Path

SCHEMA = "akcb-02/sync-plan-trace/v1"
LABELS = (
    "root", "left", "right", "tombstone", "unrelated", "foreign",
    "bad_hash", "missing", "unknown", "inaccessible", "excluded", "placeholder",
)
UNTRUSTED = {"unknown", "inaccessible", "excluded", "placeholder"}
PRESENT = {"root", "left", "right", "unrelated", "foreign", "bad_hash"}
REF = {"bad_hash": "root"}
MAX_BYTES = 1_000_000
TOTAL = len(LABELS) ** 2 * 2 * 2 * 2


def ref(label: str) -> str:
    return REF.get(label, label)


def ancestor(older: str, newer: str) -> bool:
    return older == newer or (older == "root" and newer in ("left", "right", "tombstone"))


def expected(local: str, remote: str, baseline: bool, upload: bool, download: bool) -> str:
    if local in UNTRUSTED or remote in UNTRUSTED:
        return "hold:UntrustedObservation"
    # Rust verifies each observed record in local-then-remote order.
    for value in (local, remote):
        if value == "foreign":
            return "hold:IdentityMismatch"
        if value == "bad_hash":
            return "hold:HashMismatch"
    if baseline and any(
        value in PRESENT | {"tombstone"} and not ancestor("root", ref(value))
        for value in (local, remote)
    ):
        return "hold:BaselineDrift"
    if local in PRESENT and remote in PRESENT:
        a, b = ref(local), ref(remote)
        if a == b:
            return "no_change"
        if ancestor(b, a):
            return "upload_immutable"
        if ancestor(a, b):
            return "stage_download"
        return "preserve_conflict"
    if local == "tombstone" and remote == "tombstone":
        return "no_change"
    if (local == "tombstone" and remote in PRESENT) or (remote == "tombstone" and local in PRESENT):
        return "preserve_conflict"
    if local == "tombstone" or remote == "tombstone":
        return "hold:TombstoneNeedsReview"
    if local == "missing" and remote == "missing":
        return "hold:MissingWithoutTombstone" if baseline else "no_change"
    if local in PRESENT and remote == "missing":
        if baseline:
            return "hold:MissingWithoutTombstone"
        return "upload_immutable" if upload else "hold:InitialPreviewRequired"
    if local == "missing" and remote in PRESENT:
        if baseline:
            return "hold:MissingWithoutTombstone"
        return "stage_download" if download else "hold:InitialPreviewRequired"
    raise AssertionError(f"unexpected observation pair: {local}, {remote}")


def checked_record(record: object) -> tuple[str, str, bool, bool, bool, str]:
    if not isinstance(record, dict) or set(record) != {
        "local", "remote", "baseline", "allow_initial_upload", "allow_initial_download", "outcome"
    }:
        raise AssertionError("wrong record fields")
    local, remote = record["local"], record["remote"]
    if local not in LABELS or remote not in LABELS:
        raise AssertionError("unrecognized observation")
    flags = (record["baseline"], record["allow_initial_upload"], record["allow_initial_download"])
    if any(type(flag) is not bool for flag in flags):
        raise AssertionError("flags must be booleans")
    actual = record["outcome"]
    want = expected(local, remote, *flags)
    if actual != want:
        raise AssertionError(f"Rust planner mismatched AKCB-02: {(local, remote, *flags)}: {actual!r} != {want!r}")
    return local, remote, *flags, want


def validate(data: object) -> dict[str, int]:
    if not isinstance(data, dict) or set(data) != {"schema", "target", "cases"}:
        raise AssertionError("trace envelope must have exact fields")
    if data["schema"] != SCHEMA or data["target"] != "aquilum_core::sync::plan_snapshot":
        raise AssertionError("wrong trace schema or native source")
    rows = data["cases"]
    if not isinstance(rows, list) or len(rows) != TOTAL:
        raise AssertionError(f"incomplete or oversized trace: expected {TOTAL} distinct cases")
    expected_keys = set(product(LABELS, LABELS, (False, True), (False, True), (False, True)))
    counts: Counter[str] = Counter()
    for row in rows:
        local, remote, baseline, upload, download, label = checked_record(row)
        key = (local, remote, baseline, upload, download)
        if key not in expected_keys:
            raise AssertionError("duplicate or unexpected scenario")
        expected_keys.remove(key)
        counts[label] += 1
    if expected_keys:
        raise AssertionError("missing scenario(s)")
    for required in ("no_change", "upload_immutable", "stage_download", "preserve_conflict",
                     "hold:UntrustedObservation", "hold:HashMismatch", "hold:IdentityMismatch",
                     "hold:BaselineDrift", "hold:MissingWithoutTombstone", "hold:TombstoneNeedsReview",
                     "hold:InitialPreviewRequired"):
        if counts[required] == 0:
            raise AssertionError(f"witness not present: {required}")
    return dict(sorted(counts.items()))


def refuse_duplicates(pairs: list[tuple[str, object]]) -> dict:
    result: dict = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field {key!r}")
        result[key] = value
    return result


def self_test(good: dict) -> None:
    # If the checker accepts mutated output or incomplete coverage, it is vacuous.
    import copy
    changed = copy.deepcopy(good)
    changed["cases"][0]["outcome"] = "applied_ack"
    omitted = copy.deepcopy(good)
    omitted["cases"].pop()
    doubled = copy.deepcopy(good)
    doubled["cases"][1] = doubled["cases"][0]
    for name, mutation in (("invented_ack", changed), ("missing_case", omitted), ("duplicate_case", doubled)):
        try:
            validate(mutation)
        except AssertionError:
            continue
        raise AssertionError(f"mutant passed the independent oracle: {name}")
    try:
        json.loads('{"schema":0,"schema":1}', object_pairs_hook=refuse_duplicates)
    except ValueError:
        pass
    else:
        raise AssertionError("duplicate JSON fields accepted")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("trace_path", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.trace_path.stat().st_size > MAX_BYTES:
        raise AssertionError("trace above bounded input size")
    good = json.loads(args.trace_path.read_text(encoding="utf-8"), object_pairs_hook=refuse_duplicates)
    counts = validate(good)
    if args.self_test:
        self_test(good)
    print(json.dumps({"result": "AKCB_02_NATIVE_PLANNER_TRACE_PASS", "cases": TOTAL,
                      "outcomes": counts, "mutants_rejected": 4 if args.self_test else 0}, sort_keys=True))


if __name__ == "__main__":
    main()
