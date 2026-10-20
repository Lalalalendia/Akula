#!/usr/bin/env python3
"""Summarize official MCP conformance checks without upgrading failed checks to green CI."""
import argparse
import json
from collections import Counter
from pathlib import Path


def collect(name: str, root: Path) -> dict:
    checks = []
    paths = sorted(root.rglob("checks.json")) if root.is_dir() else []
    for path in paths:
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, list):
            raise ValueError(f"Expected array of conformance checks: {path}")
        label = path.parent.name
        for entry in data:
            checks.append({
                "scenario": label,
                "id": entry.get("id", "unnamed"),
                "status": entry.get("status", "UNKNOWN"),
                "error": entry.get("errorMessage"),
                "specReferences": entry.get("specReferences", []),
            })
    counts = Counter(item["status"] for item in checks)
    return {
        "name": name,
        "scenario_directories": len(paths),
        "checks": len(checks),
        "success": counts["SUCCESS"],
        "failure": counts["FAILURE"],
        "warning": counts["WARNING"],
        "other": len(checks) - counts["SUCCESS"] - counts["FAILURE"] - counts["WARNING"],
        "results": checks,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--results-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    groups = [
        collect("initialize-smoke", args.results_root / "initialize"),
        collect("active-2025-06-18", args.results_root / "matrix"),
    ]
    report = {
        "runner": "@modelcontextprotocol/conformance@0.1.16",
        "protocol_revision": "2025-06-18",
        "transport": "local temporary bearer-injecting HTTP proxy",
        "not_covered": ["direct auth", "true SSE framing", "chunked bodies", "DNS rebinding"],
        "groups": groups,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for group in groups:
        print(
            "AKAI_MCP_MATRIX group={name} dirs={scenario_directories} "
            "checks={checks} success={success} failure={failure} "
            "warning={warning} other={other}".format(**group)
        )
        for item in group["results"]:
            if item["status"] == "FAILURE":
                print(f"AKAI_MCP_FAILURE scenario={item['scenario']} check={item['id']}")
    if groups[0]["checks"] == 0 or groups[1]["checks"] == 0:
        print("AKAI_MCP_MATRIX_INCOMPLETE official checks.json missing for one group")
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
