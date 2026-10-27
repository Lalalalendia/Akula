#!/usr/bin/env python3
"""Non-gating same-runner Release A/B of Akula's actual read_directory_impl."""
import argparse
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile

COHORTS = ("mostly_unsupported", "mixed", "mostly_supported")
ROUNDS = 4
LINE = re.compile(
    r"WORKSPACE_SCAN_ABBA cohort=(\S+) accepted=(\d+) checksum=([0-9a-f]+) "
    r"runs=(\d+) p50_us=([0-9.]+) p95_us=([0-9.]+)"
)


def make_fixture(root: Path):
    settings = {
        "mostly_unsupported": (10000, 100),
        "mixed": (5000, 5000),
        "mostly_supported": (150, 9000),
    }
    for cohort, (unsupported, supported) in settings.items():
        path = root / cohort
        path.mkdir()
        for i in range(unsupported):
            (path / f"unsupported-{i:06d}.json").touch()
        suffixes = ("md", "TXT", "EPUB", "PNG", "PDF")
        for i in range(supported):
            (path / f"accepted-{i:06d}.{suffixes[i % len(suffixes)]}").touch()
        for name in (".заметки", "user-directory", ".git", ".akula-private"):
            (path / name).mkdir()
        (path / ".hidden.md").touch()
        (path / "._mac.md").touch()


def measure(binary: Path, corpus: Path, out: Path, index: int, label: str):
    env = dict(os.environ, AKULA_WORKSPACE_SCAN_ABBA_DIR=str(corpus))
    result = subprocess.run(
        [str(binary), "benchmark_workspace_scan_abba", "--ignored", "--nocapture", "--test-threads=1"],
        cwd=str(corpus.parent),
        env=env,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    )
    (out / f"{index:02d}-{label}.log").write_text(result.stdout)
    if result.returncode != 0:
        raise RuntimeError(f"{label} benchmark failed exit={result.returncode}, log={index:02d}-{label}.log")
    records = {}
    for line in result.stdout.splitlines():
        match = LINE.search(line)
        if match:
            cohort, count, digest, runs, p50, p95 = match.groups()
            if cohort in records:
                raise RuntimeError(f"duplicate {cohort} in {label}")
            records[cohort] = (int(count), digest, int(runs), float(p50), float(p95))
    if set(records) != set(COHORTS):
        raise RuntimeError(f"unexpected cohorts for {label}: {list(records)}")
    if any(v[2] != 40 for v in records.values()):
        raise RuntimeError("missing iteration data")
    return records


def run(args):
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    baselines = {"base": [], "candidate": []}
    with tempfile.TemporaryDirectory(prefix="akula-workspace-scan-") as directory:
        corpus = Path(directory)
        make_fixture(corpus)
        binaries = {"base": Path(args.baseline).resolve(), "candidate": Path(args.candidate).resolve()}
        for kind in ("base", "candidate"):
            measure(binaries[kind], corpus, out, 0, f"{kind}-warmup")
        counter = 0
        for _ in range(ROUNDS):
            for kind in ("base", "candidate", "candidate", "base"):
                counter += 1
                sample = measure(binaries[kind], corpus, out, counter, kind)
                baselines[kind].append(sample)
        # Include deterministic IDs, names, types and accepted-count parity across all runs.
        for cohort in COHORTS:
            keys = {(sample[cohort][0], sample[cohort][1])
                    for samples in baselines.values() for sample in samples}
            if len(keys) != 1:
                raise RuntimeError(f"directory contents changed across binaries for {cohort}: {keys}")
    report = [
        "# Workspace directory scan: real Rust Release A/B (Argon 8.7)",
        "",
        "Four ABBA rounds: eight 40-scan samples per revision and cohort.",
        "Same runner, identical deterministic directory corpus. Compilation excluded.",
        "Checksum/accepted-count parity checked across every sample.",
        "Diagnostic wall-clock timings in microseconds; smaller is better.",
        "This is NOT whole-app FileTree startup time or a Windows 7 execution.",
        "",
        "| Corpus | Accepted entries | Base p50 (us) | Candidate p50 (us) | Base/Candidate |",
        "|---|---:|---:|---:|---:|",
    ]
    for cohort in COHORTS:
        accepted = baselines["base"][0][cohort][0]
        b = statistics.median(sample[cohort][3] for sample in baselines["base"])
        c = statistics.median(sample[cohort][3] for sample in baselines["candidate"])
        ratio = b / c if c else float("nan")
        report.append(f"| {cohort} | {accepted} | {b:.2f} | {c:.2f} | {ratio:.3f}x |")
    report += [
        "",
        "No automatic success threshold, no FPS claim. Evaluate regressions across",
        "supported-heavy and unsupported-heavy files before merging.",
    ]
    (out / "summary.md").write_text("\n".join(report) + "\n")
    print("\n".join(report))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--out", required=True)
    run(parser.parse_args())


if __name__ == "__main__":
    main()
