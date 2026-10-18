#!/usr/bin/env python3
"""DELTA-TEXT-01: isolated byte-exact A/B; NOT an Akula V1/V2 wire format.

Compares full Zstd, one byte-level splice + Zstd, and Zstd --patch-from.
Reads public repo Markdown only; never calls S3 or writes user files.
Requires Python 3.10+ and the Zstd CLI.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import random
import shutil
import statistics
import struct
import subprocess
import tempfile
import time

MAGIC = b"AKDAB1"  # Experimental only: must not be accepted by production Core.
MAX_BYTES = 2 * 1024 * 1024
FULL, SPLICE, ZSTD_PATCH = range(3)
MIN_SAVING_BYTES = 128
MIN_SAVING_PCT = 0.15
REPO_FILES = (
    "docs/sync/adr-0001-built-in-sync-contract.md",
    "docs/sync/coord-02f-10k-parity.md",
    "docs/sync/adr-0017-cooperative-os-lock-writer-lifetime.md",
    "docs/sync/s3-minio-pr05-implementation.md",
    "docs/performance-roadmap.md",
    "aquilum-app/native/testdata/markdown-test.md",
)
SCENARIOS = (
    "one-character", "insert-sentence", "append-paragraph",
    "scattered-ten-edits", "delete-middle", "reorder-paragraphs", "rewrite-all",
)


class DeltaFailure(Exception):
    """Fail closed on unknown encoding, missing base or tampering."""


def sha(data: bytes) -> bytes:
    # SHA-256 is only a fixture digest; production Akula uses BLAKE3.
    return hashlib.sha256(data).digest()


def envelope(mode: int, base: bytes, target: bytes, frame: bytes) -> bytes:
    if mode not in (FULL, SPLICE, ZSTD_PATCH):
        raise DeltaFailure("unsupported mode")
    if max(len(base), len(target), len(frame)) > MAX_BYTES:
        raise DeltaFailure("over budget")
    # Include a 32-byte base identity for delta modes when comparing bytes.
    return (MAGIC + bytes((mode,)) + struct.pack("<I", len(target))
            + sha(target) + (sha(base) if mode != FULL else b"") + frame)


def parse(wire: bytes):
    minimum = len(MAGIC) + 1 + 4 + 32
    if len(wire) < minimum or not wire.startswith(MAGIC):
        raise DeltaFailure("invalid envelope")
    mode = wire[len(MAGIC)]
    if mode not in (FULL, SPLICE, ZSTD_PATCH):
        raise DeltaFailure("unsupported encoding")
    header = minimum + (32 if mode != FULL else 0)
    if not header < len(wire) <= header + MAX_BYTES:
        raise DeltaFailure("invalid frame size")
    length = struct.unpack("<I", wire[len(MAGIC)+1:len(MAGIC)+5])[0]
    if length > MAX_BYTES:
        raise DeltaFailure("output limit")
    target_hash = wire[len(MAGIC)+5:minimum]
    base_hash = wire[minimum:header] if mode != FULL else None
    return mode, length, target_hash, base_hash, wire[header:]


def splice(base: bytes, target: bytes) -> bytes:
    prefix = 0
    while prefix < min(len(base), len(target)) and base[prefix] == target[prefix]:
        prefix += 1
    suffix = 0
    while (suffix < min(len(base), len(target))-prefix
           and base[-suffix-1] == target[-suffix-1]):
        suffix += 1
    inserted = target[prefix:len(target)-suffix if suffix else len(target)]
    return struct.pack("<III", prefix, len(base)-prefix-suffix, len(inserted)) + inserted


def apply_splice(base: bytes, patch: bytes) -> bytes:
    if len(patch) < 12:
        raise DeltaFailure("short splice")
    start, removed, inserted = struct.unpack("<III", patch[:12])
    if (start > len(base) or removed > len(base)-start
            or inserted > MAX_BYTES or len(patch) != 12 + inserted):
        raise DeltaFailure("splice outside verified base")
    output = base[:start] + patch[12:] + base[start+removed:]
    if len(output) > MAX_BYTES:
        raise DeltaFailure("splice output too large")
    return output


def run_zstd(args: list[str], *, max_output=MAX_BYTES) -> bytes:
    process = subprocess.Popen(["zstd", "-q", *args], stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
    try:
        # Memory-bounded stdout. This is a test harness, NOT a production decoder.
        output = process.stdout.read(max_output + 1)
        if len(output) > max_output:
            process.kill()
            process.communicate()
            raise DeltaFailure("zstd output budget exceeded")
        stderr = process.stderr.read(8192)
        if process.wait(timeout=15) != 0:
            raise DeltaFailure("zstd error: " + stderr.decode(errors="replace")[:180])
        return output
    except (OSError, subprocess.TimeoutExpired) as exc:
        process.kill()
        process.communicate()
        raise DeltaFailure("zstd execution failed") from exc
    finally:
        process.stdout.close()
        process.stderr.close()


def encode(mode: int, base: bytes, target: bytes, folder: Path):
    old, new, patch = folder/"old.md", folder/"new.md", folder/"patch.bin"
    old.write_bytes(base)
    new.write_bytes(target)
    start = time.perf_counter_ns()
    if mode == FULL:
        frame = run_zstd(["-3", "-c", str(new)])
    elif mode == SPLICE:
        patch.write_bytes(splice(base, target))
        frame = run_zstd(["-3", "-c", str(patch)])
    elif mode == ZSTD_PATCH:
        # Zstd's dictionary patch encoder is unsafe for a zero-byte base on
        # some CLI builds; never invoke it on a tiny or unknown base.
        if len(base) < 64:
            raise DeltaFailure("base too short for patch-from")
        frame = run_zstd(["-3", "--patch-from=" + str(old), "-c", str(new)])
    else:
        raise DeltaFailure("unsupported mode")
    elapsed = (time.perf_counter_ns()-start)/1e6
    return envelope(mode, base, target, frame), elapsed


def decode(wire: bytes, base: bytes | None, folder: Path):
    mode, length, expected, base_hash, frame = parse(wire)
    if base_hash is not None and (base is None or sha(base) != base_hash):
        raise DeltaFailure("verified base missing or incorrect")
    start = time.perf_counter_ns()
    raw = folder/"payload.zst"
    raw.write_bytes(frame)
    if mode == ZSTD_PATCH:
        old = folder/"verified.md"
        old.write_bytes(base)
        data = run_zstd(["-d", "--patch-from=" + str(old), "-c", str(raw)])
    else:
        data = run_zstd(["-d", "-c", str(raw)])
        if mode == SPLICE:
            data = apply_splice(base, data)
    if len(data) != length or sha(data) != expected:
        raise DeltaFailure("target digest or size mismatch")
    return data, (time.perf_counter_ns()-start)/1e6


def mutate(base: str, kind: str) -> str:
    n = len(base)
    if kind == "one-character":
        i = max(1, n//2)
        return base[:i] + "Ж" + base[i+1:]
    if kind == "insert-sentence":
        i = n//2
        return base[:i] + "\nДобавили строку: строго проверяем содержимое.\n" + base[i:]
    if kind == "append-paragraph":
        return base + "\n\n## Новая секция\nДанные остаются проверяемыми.\n"
    if kind == "scattered-ten-edits":
        chars = list(base)
        for i in range(1, 11):
            j = min(n-1, n*i//11)
            chars[j] = "Я" if chars[j] != "Я" else "А"
        return "".join(chars)
    if kind == "delete-middle":
        return base[:n//3] + base[n//3+n//8:]
    if kind == "reorder-paragraphs":
        paragraphs = base.split("\n\n")
        if len(paragraphs) < 4:
            return base[n//2:] + base[:n//2]
        i = len(paragraphs)//2
        paragraphs[i-1], paragraphs[i] = paragraphs[i], paragraphs[i-1]
        return "\n\n".join(paragraphs)
    if kind == "rewrite-all":
        rng = random.Random(20261018+len(base))
        symbols = "abcdefghijklmnopqRSTUVWXYZабвгдежзийклмнопрстуфхцчшщэюя 0123456789\n"
        return "".join(rng.choices(symbols, k=n))
    raise ValueError(kind)


def must_reject(wire: bytes, base: bytes | None, folder: Path):
    try:
        decode(wire, base, folder)
    except DeltaFailure:
        return
    raise AssertionError("malformed/wrong-base transport accepted")


def self_test():
    samples = [
        (b"", "тест".encode()),
        (b"A", b""),
        ("Unicode 🙂".encode(), "Unicode 🟢".encode()),
        (b"abcabcabc", b"abcxyzabc"),
        (b"A"*300, b"A"*150+b"X"+b"A"*149),
    ]
    with tempfile.TemporaryDirectory(prefix="akula-text-test-") as tmp:
        folder = Path(tmp)
        for base, target in samples:
            assert apply_splice(base, splice(base, target)) == target
            for mode in (FULL, SPLICE, ZSTD_PATCH):
                if mode == ZSTD_PATCH and len(base) < 64:
                    continue
                wire, _ = encode(mode, base, target, folder)
                assert decode(wire, base, folder)[0] == target
                if mode != FULL:
                    must_reject(wire, b"WRONG"+base, folder)
                    must_reject(wire, None, folder)
                changed = bytearray(wire)
                changed[len(MAGIC)+5] ^= 1  # Falsify target hash
                must_reject(bytes(changed), base, folder)
                must_reject(wire[:9], base, folder)
                must_reject(wire[:-1], base, folder)
                must_reject(wire[:len(MAGIC)]+b"\xff"+wire[len(MAGIC)+1:], base, folder)

        parent = b"shared markdown parent\n" * 100
        left = parent[:240] + b"LEFT" + parent[240:]
        right = parent[:850] + b"RIGHT" + parent[850:]
        l, _ = encode(ZSTD_PATCH, parent, left, folder)
        r, _ = encode(ZSTD_PATCH, parent, right, folder)
        assert decode(l, parent, folder)[0] == left
        assert decode(r, parent, folder)[0] == right
        must_reject(l, right, folder)
        must_reject(r, left, folder)
        for bad in (struct.pack("<III",999,1,1)+b"X",
                    struct.pack("<III",0,0,9)+b"Z"):
            try:
                apply_splice(b"hello", bad)
            except DeltaFailure:
                pass
            else:
                raise AssertionError("forged splice accepted")
        try:
            envelope(FULL, b"", b"a"*(MAX_BYTES+1), b"data")
        except DeltaFailure:
            pass
        else:
            raise AssertionError("oversized target accepted")
    print("DELTA_TEXT_AB_SELF_TEST: PASS")


def benchmark(repo: Path, files: list[str], out: Path, repetitions: int):
    self_test()
    result_rows = []
    totals = {"full_zstd": 0, "splice_zstd": 0, "zstd_patch": 0}
    with tempfile.TemporaryDirectory(prefix="akula-text-ab-") as tmp:
        folder = Path(tmp)
        for relative in files:
            source = (repo/relative).resolve()
            if not source.is_file() or not source.is_relative_to(repo.resolve()):
                raise SystemExit("unsafe/missing fixture: " + relative)
            base = source.read_bytes()
            if len(base) > MAX_BYTES:
                raise SystemExit("fixture too large")
            original = base.decode("utf-8")
            for kind in SCENARIOS:
                target = mutate(original,kind).encode("utf-8")
                if len(target) > MAX_BYTES:
                    raise SystemExit("target too large")
                results = {}
                for mode,name in ((FULL,"full_zstd"),(SPLICE,"splice_zstd"),
                                  (ZSTD_PATCH,"zstd_patch")):
                    sizes, enc_times, dec_times = [],[],[]
                    for _ in range(repetitions):
                        wire, enc_ms = encode(mode,base,target,folder)
                        reconstructed, dec_ms = decode(wire,base,folder)
                        assert reconstructed == target, "byte mismatch"
                        sizes.append(len(wire))
                        enc_times.append(enc_ms)
                        dec_times.append(dec_ms)
                    assert len(set(sizes)) == 1
                    results[name] = {
                        "wire_bytes":sizes[0],
                        "encode_ms_median":round(statistics.median(enc_times),3),
                        "decode_ms_median":round(statistics.median(dec_times),3),
                    }
                winner = min(results,key=lambda name:results[name]["wire_bytes"])
                full = results["full_zstd"]["wire_bytes"]
                saved = full-results[winner]["wire_bytes"]
                selected = (winner if winner != "full_zstd"
                            and saved >= MIN_SAVING_BYTES
                            and saved >= MIN_SAVING_PCT*full else "full_zstd")
                totals[selected] += 1
                result_rows.append({
                    "fixture":relative,"scenario":kind,"base_bytes":len(base),
                    "target_bytes":len(target),
                    "modes":results,"smallest_mode":winner,
                    "example_policy_selected":selected,
                    "savings_percent_vs_full_zstd":round(
                        100*(full-results[selected]["wire_bytes"])/full,2),
                    "includes_envelope":True,"fixture_is_public_repo_markdown":True,
                    "assurance":"isolated-local-roundtrip-no-V1-V2-wire-no-S3-no-File-Gate",
                })
                print(f"{relative.rsplit('/',1)[-1][:23]:23} {kind:19} "
                      f"full={full:6} splice={results['splice_zstd']['wire_bytes']:6} "
                      f"patch={results['zstd_patch']['wire_bytes']:6} chosen={selected}")
    out.parent.mkdir(parents=True,exist_ok=True)
    out.write_text("".join(json.dumps(row,sort_keys=True,ensure_ascii=False)+"\n"
                           for row in result_rows),encoding="utf-8")
    total_full=sum(row["modes"]["full_zstd"]["wire_bytes"] for row in result_rows)
    total_sel=sum(row["modes"][row["example_policy_selected"]]["wire_bytes"] for row in result_rows)
    print("DELTA_TEXT_AB_SUMMARY "+json.dumps({
        "fixtures":len(files),"scenarios":len(result_rows),"selection_counts":totals,
        "full_zstd_bytes":total_full,"example_selected_bytes":total_sel,
        "relative_saving_percent":round(100*(total_full-total_sel)/total_full,2),
        "not_a_full_sync_speedup_claim":True,
    },sort_keys=True))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo",type=Path,default=Path.cwd())
    parser.add_argument("--file",action="append",dest="files")
    parser.add_argument("--output",type=Path,default=Path("sync-text-delta-ab.jsonl"))
    parser.add_argument("--repetitions",type=int,default=3)
    parser.add_argument("--self-test-only",action="store_true")
    args=parser.parse_args()
    if shutil.which("zstd") is None:
        raise SystemExit("zstd CLI required")
    if args.self_test_only:
        self_test()
    else:
        if not 1 <= args.repetitions <= 10:
            raise SystemExit("repetitions must be 1..10")
        benchmark(args.repo,args.files or list(REPO_FILES),args.output,args.repetitions)


if __name__ == "__main__":
    main()
