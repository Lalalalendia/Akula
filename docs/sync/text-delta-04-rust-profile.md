# DELTA-TEXT-04 — actual Rust codec profile on public Git revisions

**22 October 2026. Isolated laboratory A/B; no new production sync wire.**

Related: [DELTA-TEXT-02 real Git/S3 study](text-delta-02-real-git-and-minio.md), [DELTA-TEXT-03 bounded Rust codec](text-delta-03-rust-codec.md), [Akula algorithm matrix](https://app.notion.com/p/40132a84beec81229572f4a28c44a66d). This experiment follows the previously validated optional Rust `sync-text-delta` implementation. Nothing is newly wired into the V1 immutable blobs, legacy V2 pack, S3 writer, File Gate or end-to-end Akula update flow.

## Why an additional experiment

Earlier DELTA-TEXT-02 measured a *different* `zstd --patch-from` command-line codec (89.87% less experimental warm-base sidecar traffic on 17 actual adjacent Git pairs). The Rust `DictionaryZstd` codec uses a **raw old-text dictionary**, not that CLI patch format. Its actual savings, encode/decode time and memory must be measured independently; do not copy the old percentage onto Rust.

DELTA-TEXT-04 adds a **new ignored Rust test** to `aquilum-app/core/src/sync/text_delta_codec.rs` and a Python measurement harness `scripts/benchmarks/sync-text-delta-rust-profile.py`. The existing production functions and codec wire schema remain unchanged. The dedicated workflow `.github/workflows/sync-text-delta-rust.yml` is expanded, rather than adding a new expensive CI fan-out.

## Corpus and exact pairing

Uses DELTA-TEXT-02's five public first-party Markdown paths and the same selection rule:

- latest adjacent commits touching the same path from complete `git log`;
- at most four **valid changed** UTF-8 pairs per path;
- old and new each at least 64 bytes and at most 2 MiB;
- full git history mandatory (`actions/checkout` with `fetch-depth: 0`);
- 9–20 cases allowed to accommodate legitimate history changes;
- every output contains public path, 12-character old/new commit IDs, old/new **BLAKE3** digests and lengths. No file content is exported.

The corpus is **history-rule-compatible** with DELTA-TEXT-02, **not necessarily byte-for-byte identical to the original October 18 fixed output after new Git commits**. Actual IDs/hashes in CSV are the source of truth; do not claim identical 17 cases without checking the recorded pairs.

Each mode processes *the same* set of valid pairs; the harness refuses a mismatch in paths, commit IDs, base hashes or target hashes.

## Four explicitly measured alternatives

| Name in CSV | Mode | Preconditions |
|---|---|---|
| `full` | `FullZstd` (complete verified frame) | no prior version |
| `splice` | `SpliceZstd` (single contiguous splice + Zstd) | exact old revision independently verified |
| `dictionary` | `DictionaryZstd` (raw old text as Zstd dictionary) | exact old revision independently verified |
| `selected` | **Actual Rust `encode()` policy** chooses full/splice/dictionary | falls back to full unless one dependent frame saves >=128 bytes AND >=15% of the complete full frame |

All candidates have the same **120-byte versioned envelope**, target revision ID, BLAKE3 target digest and bounds. Forced splice/dictionary are **diagnostic only**: the actual policy must not select them if their advantage is too small. Every encode/decode round-trip is verified byte-for-byte and refuses corrupt/wrong-base data via existing defensive tests.

Encode/decode median time: `std::time::Instant` wall nanoseconds inside Rust for **7 repetitions per Git pair per mode**. One process per mode; Linux `os.wait4()` measures *CPU seconds and peak RSS* for the Rust test process. CPU includes test logic but **excludes Git subprocess CPU**; RSS includes Rust runtime, test harness and temporary Git data, so it cannot be interpreted as the memory allocated by one frame. These are *not measurements of Akula running on a user's device*.

## Costs and denominators

Two **different** questions are reported:

1. Does the warm-base delta reduce the **experimental versioned frame** relative to full Zstd? Compare full `wire_bytes` with the selected `wire_bytes` (all 120-byte envelope costs included).
2. What if the current architecture still requires the complete immutable raw V1 blob *as well*? The measured model reports `target_raw_bytes + frame_bytes` for a **hypothetical mandatory V1 + sidecar sender**, not a real measured S3 PUT. A smaller sidecar cannot eliminate the separate full V1 transfer.

If the receiver does **not** already hold an independently authenticated matching old base, use `FullZstd` rather than fetching an untrusted/unknown base just for a patch. The CSV separately reports `cold_receiver_full_fallback_bytes` and the *counterfactual* `naive_cold_fetch_base_plus_selected_bytes`; latter counts raw old bytes and therefore is **not** an S3 packet capture.

This test neither verifies two devices nor changes immutable history, CAS, File Gate, remote authority, consent or user data.

## Reproduce

```bash
python3 scripts/benchmarks/sync-text-delta-rust-profile.py --self-test-only
python3 scripts/benchmarks/sync-text-delta-rust-profile.py \
  --repo . --output results/text-delta-04 --repetitions 7
```

Linux and a working Rust/Cargo toolchain, full Git checkout, locked `aquilum-app` workspace needed. The script compiles only `aquilum-core` library tests with `--no-default-features --features sync-text-delta` and runs its ignored benchmark function once in each of four isolated processes. The narrow existing GitHub workflow additionally checks old tests and legacy pack rejection.

Files:

- `real-git-pairs.csv`: old/new commit IDs, BLAKE3 digests, source/target bytes, each of three forced codec frame lengths, selected mode, warm-base savings and hypothetical cold and V1+sidecar costs.
- `mode-summary.csv`: four total wire lengths, representative Rust per-pair encode/decode median, **process CPU seconds**, **process peak RSS KiB**.
- `report.json` and `report.md`: context, limits and totals.

## Decision gate

Do **not** switch production sync based on this result. First verify whether total **actual** S3 PUT/GET bytes, CPU and wall time improve with hot/missing/wrong old base, proper V1 anchoring, negotiated capability, post-crash resume, File Gate, and strict legacy V2 rejection. The existing Rust codec remains an opt-in inert representation until those independent acceptance gates pass.

Even if dictionary or splice saves most sidecar bytes on 9–20 Git fixture pairs, this does not establish real user-data traffic savings, actual mobile energy reduction, or a viable alternative to mandatory V1 raw blobs.
