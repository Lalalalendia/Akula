#!/usr/bin/env python3
"""Summarize test-only tab phase timings from Akula's native note matrix."""
from __future__ import annotations

import math
import sys
from pathlib import Path


def attrs(text: str) -> dict[str, str]:
    return dict(part.split("=", 1) for part in text.split() if "=" in part)


def p95(records: list[dict[str, float]], key: str) -> float:
    values = sorted(record[key] for record in records)
    if not values:
        raise ValueError(f"Missing samples for {key}")
    return values[max(0, math.ceil(0.95 * len(values)) - 1)]


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: summarize-tab-phases.py note-latency.log")

    groups: dict[str, dict] = {}
    current: str | None = None
    for line in Path(sys.argv[1]).read_text(encoding="utf-8").splitlines():
        if "NOTE_PHASE_BEGIN " in line:
            marker = attrs(line.split("NOTE_PHASE_BEGIN ", 1)[1])
            current = marker["kib"]
            if current in groups:
                raise RuntimeError(f"Duplicate fixture group {current} KiB")
            groups[current] = {
                "cycles": int(marker["cycles"]),
                "tab": [],
                "chrome": [],
                "closed": False,
            }
            continue
        if "NOTE_PHASE_END " in line:
            marker = attrs(line.split("NOTE_PHASE_END ", 1)[1])
            if current != marker["kib"]:
                raise RuntimeError("Mismatched begin/end markers")
            groups[current]["closed"] = True
            current = None
            continue
        if current is None:
            continue
        for marker, key in (
            ("NOTE_TAB_PHASE ", "tab"),
            ("NOTE_CHROME_PHASE ", "chrome"),
        ):
            if marker in line:
                group = groups[current][key]
                group.append(
                    {
                        name: float(value)
                        for name, value in attrs(line.split(marker, 1)[1]).items()
                    }
                )

    if current is not None:
        raise RuntimeError(f"Unclosed fixture group {current}")
    if sorted(groups, key=int) != ["64", "512", "4096"]:
        raise RuntimeError(f"Unexpected fixture groups: {list(groups)}")

    for kib, group in groups.items():
        for key in ("tab", "chrome"):
            if len(group[key]) != group["cycles"] or not group["closed"]:
                raise RuntimeError(
                    f"Incomplete {kib} KiB {key} timings: "
                    f"{len(group[key])}/{group['cycles']}"
                )

    print("\n### Tab activation phase p95 (ms)\n")
    print(
        "| Note size | Previous note cleanup | File read and decode | "
        "Editor page creation | Remaining note work | Other UI chrome |"
    )
    print("|---|---:|---:|---:|---:|---:|")
    for kib in sorted(groups, key=int):
        inner = groups[kib]["tab"]
        outer = groups[kib]["chrome"]
        chrome_extra = [
            {"ms": row["total_ms"] - row["show_tab_ms"]} for row in outer
        ]
        print(
            f"| {kib} KiB | {p95(inner, 'leave_ms'):.2f} | "
            f"{p95(inner, 'read_ms'):.2f} | "
            f"{p95(inner, 'render_ms'):.2f} | "
            f"{p95(inner, 'final_ms'):.2f} | "
            f"{p95(chrome_extra, 'ms'):.2f} |"
        )
    print(
        "\nDiagnostic breakdown from the same tab-switch loop. Values are "
        "per-stage p95, not additive: different iterations may contribute "
        "each stage's p95. This test-only probe does not imply release-mode "
        "end-to-end performance.\n"
    )


if __name__ == "__main__":
    main()
