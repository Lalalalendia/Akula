#!/usr/bin/env python3
"""AKCB-03: independent fixture-trace oracle for the inert MCP grant lab.

This validates the trace from the REAL Rust AgentCredentialStore/
AgentVaultGrantStore test. It does NOT model production MCP, search or OS roots.
Usage: python3 docs/research/akcb_03_derived_trace_check.py trace.json --self-test
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

SCHEMA = "akcb-03/derived-disclosure-lab/v1"
PRODUCER = "aquilum_core::mcp::akcb_03_composite_lab"
X = ["X/one.md"]
Y = ["Y/two.md"]
XY = X + Y
EXPECTED = {
    "a_only_x": ("SENT", X),
    "a_only_y": ("DENIED", None),
    "a_mixed": ("DENIED", None),
    "a_filtered_all": ("SENT", X),
    "a_filtered_hidden_changed": ("SENT", X),
    "b_mixed": ("SENT", XY),
    "b_y_only": ("SENT", Y),
    "b_old_mixed_after_y_revoke": ("DENIED", None),
    "b_x_after_y_revoke": ("SENT", X),
    "a_x_unaffected_by_b_revoke": ("SENT", X),
    "b_y_new_while_revoked": ("DENIED", None),
    "b_old_mixed_after_regrant": ("DENIED", None),
    "b_new_mixed_after_regrant": ("SENT", XY),
    "b_missing_lineage_lease": ("DENIED", None),
    "b_swapped_lineage_lease": ("DENIED", None),
    "b_partially_written_response": ("PARTIAL_DISCLOSURE_HOLD", None),
    "b_emit_during_revoke": ("SENT", XY),
    "b_cached_after_concurrent_revoke": ("DENIED", None),
    "b_x_after_credential_revoke": ("DENIED", None),
    "a_after_b_credential_revoke": ("SENT", X),
}
REQUIRED_FIELDS = {"scenario", "outcome", "bytes", "total", "titles"}
MAX_BYTES = 32768


def no_duplicate_keys(pairs: list[tuple[str, object]]) -> dict:
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate JSON key {key!r}")
        obj[key] = value
    return obj


def verify(trace: object) -> dict[str, int]:
    if not isinstance(trace, dict) or set(trace) != {"schema", "producer", "cases"}:
        raise AssertionError("invalid trace envelope")
    if trace["schema"] != SCHEMA or trace["producer"] != PRODUCER:
        raise AssertionError("trace is not AKCB-03 inert Rust producer")
    rows = trace["cases"]
    if not isinstance(rows, list) or len(rows) != len(EXPECTED):
        raise AssertionError("missing/extra AKCB-03 cases")
    seen = set()
    counts = {"sent": 0, "denied": 0, "partial_hold": 0}
    for row in rows:
        if not isinstance(row, dict) or set(row) != REQUIRED_FIELDS:
            raise AssertionError("malformed case fields")
        name = row["scenario"]
        if not isinstance(name, str) or name not in EXPECTED or name in seen:
            raise AssertionError("unexpected or duplicate scenario")
        seen.add(name)
        expected_outcome, expected_titles = EXPECTED[name]
        if row["outcome"] != expected_outcome:
            raise AssertionError(f"{name}: {row['outcome']!r} != {expected_outcome!r}")
        bytes_sent = row["bytes"]
        if type(bytes_sent) is not int or bytes_sent < 0 or bytes_sent > MAX_BYTES:
            raise AssertionError("invalid emitted byte count")
        if expected_outcome == "SENT":
            counts["sent"] += 1
            if bytes_sent == 0 or row["total"] != len(expected_titles) or row["titles"] != expected_titles:
                raise AssertionError(f"{name}: leaked/miscounted/omitted source lineage")
        elif expected_outcome == "DENIED":
            counts["denied"] += 1
            if bytes_sent != 0 or row["titles"] is not None or row["total"] is not None:
                raise AssertionError(f"{name}: deny must not release content or metadata")
        else:
            counts["partial_hold"] += 1
            if bytes_sent != 4 or row["titles"] is not None or row["total"] is not None:
                raise AssertionError("ambiguous partial emission must not claim SENT/zero-disclosure")
    if seen != EXPECTED.keys():
        raise AssertionError("not all expected scenarios present")
    if counts != {"sent": 10, "denied": 9, "partial_hold": 1}:
        raise AssertionError(f"unexpected coverage: {counts}")
    return counts


def self_test(good: dict) -> int:
    mutants = []
    changed = copy.deepcopy(good)
    changed["cases"][1]["outcome"] = "SENT"
    mutants.append(changed)
    reduced = copy.deepcopy(good)
    reduced["cases"].pop()
    mutants.append(reduced)
    duplicated = copy.deepcopy(good)
    duplicated["cases"][1] = duplicated["cases"][0]
    mutants.append(duplicated)
    leaked = copy.deepcopy(good)
    leaked["cases"][0]["titles"] = ["X/one.md", "Y/two.md"]
    mutants.append(leaked)
    for mutation in mutants:
        try:
            verify(mutation)
        except AssertionError:
            pass
        else:
            raise AssertionError("invalid fixture trace accepted")
    try:
        json.loads('{"schema":0,"schema":1}', object_pairs_hook=no_duplicate_keys)
    except ValueError:
        pass
    else:
        raise AssertionError("duplicate JSON keys accepted")
    return 5


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("trace", type=Path)
    parser.add_argument("--self-test", action="store_true")
    opts = parser.parse_args()
    if opts.trace.stat().st_size > MAX_BYTES:
        raise AssertionError("trace too large")
    trace = json.loads(opts.trace.read_text(encoding="utf-8"), object_pairs_hook=no_duplicate_keys)
    tally = verify(trace)
    mutants = self_test(trace) if opts.self_test else 0
    print(json.dumps({"status": "AKCB_03_RUST_TEST_TRACE_PASS",
                      "cases": len(EXPECTED), "result_counts": tally,
                      "negative_controls": mutants},
                     sort_keys=True))


if __name__ == "__main__":
    main()
