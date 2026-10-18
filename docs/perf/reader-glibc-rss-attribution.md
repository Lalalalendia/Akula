# Reader post-close RSS on Linux/glibc — measured attribution and stop condition

Status: **diagnostic complete (18 October 2026); no additional production change approved**.

Related accepted work: [#211](https://github.com/Lalalalendia/Akula/pull/211) (real first-text frame and RSS), [#221](https://github.com/Lalalalendia/Akula/pull/221) (real native page/scroll memory probe), [#228](https://github.com/Lalalalendia/Akula/pull/228) (Linux/glibc-only post-close `malloc_trim(0)`).

## Controlled local reproduction

- Same *locally supplied*, unmodified EPUB, 354,325 B, SHA-256 `b67db22d11eb1fd3a506e9142ddd71f97a3578543d854f5b053059e4e5703153`. **No original book text or screenshots were uploaded to GitHub.**
- Instrumented Linux release binary from the accepted #228 branch, native Winit/Masonry/softbuffer under Xvfb at 1200×800 and 4 renderer threads.
- Two fresh processes with the **same binary**: unmodified default glibc versus `MALLOC_ARENA_MAX=4`. Each completed four full Reader open/navigate/close cycles, 80 paginated next/prev and 80 scrolled next/prev actions each time: **1,280 navigation actions/process**.
- All **28/28 Reader-state samples, 4/4 first text-containing submitted frames and 4/4 close-time trims** were present for each process; both exited 0, and both navigation modes returned to their initial positions. Staging files were written outside the artifact-export directory to prevent partial live-log snapshots.
- Diagnostic-only `LD_PRELOAD` wrapper around the **existing** glibc `malloc_trim` captured `malloc_info(0)` for **all arenas**, `/proc/self/smaps_rollup`, and mapping-level `/proc/self/smaps` immediately before/after each close-time call. This wrapper is not shipped with Akula and slightly perturbs allocations/latencies.

## RSS after close-time trim (MiB)

| Reader close | Default glibc | 4-arena cap |
|---|---:|---:|
| Cycle 1 | 77.50 | 73.96 |
| Cycle 2 | 101.18 | 99.35 |
| Cycle 3 | 103.52 | 104.22 |
| Cycle 4 | **115.78** | **102.58** |

The 4-arena cap saved 13.20 MiB on cycle 4 **in this one matched scenario**, but was marginally worse at cycle 3. No robust general advantage is established.

After the fourth close:

| Observed memory | Default glibc | 4-arena cap |
|---|---:|---:|
| Number of glibc arenas | 22 | 4 |
| Counted live Rust heap | 29.56 MiB | 30.88 MiB |
| Glibc arena-plus-mmap used-size **estimate** | 32.91 MiB | 34.20 MiB |
| Glibc free chunks retained in arenas | 108.38 MiB | 73.80 MiB |
| Anonymous PSS | 80.99 MiB | 66.52 MiB |
| File PSS | 25.46 MiB | 26.74 MiB |

The glibc estimate is `system current - fast-free - rest-free + mmap`; it **does not count all live process memory** and cannot reliably isolate allocator tcache or independent native allocators. Free-chunk *virtual bytes* are not resident-byte totals. File PSS was flat **within** each four-cycle run, while anonymous RSS/PSS and arena free space increased.

In the default run, counted live Rust heap after each close was **29.38 / 29.54 / 29.54 / 29.56 MiB**, and the estimated glibc used size was **32.61 / 32.82 / 32.85 / 32.91 MiB**. `Session.layouts` stayed at 1–4 on-screen layouts. Thus RSS growth was **not matched by growth of counted live Reader layouts or glibc occupied chunks**. This supports allocator arena reuse/fragmentation as a major explanation, **not** a universal proof of zero native leaks.

A single-process diagnostic comparison of first text-frame medians was 223.28 versus 205.34 ms; native app frame p95 was 40.96 versus 39.94 ms (default versus four arenas). **Four first-frame samples per configuration and instrumentation overhead are insufficient to claim speed improvements, safe global malloc policy, or absence of latency regressions.**

## Decision and next trigger

1. **Keep** #228's bounded Linux/glibc `malloc_trim(0)` *only after Reader teardown*. Previous full CI and local real-book tests accepted it; Windows/macOS/non-gnu Linux are unchanged.
2. **Do not enable** global `MALLOC_ARENA_MAX=4`, aggressive trim-on-every-allocation, or further Parley cache invalidation in production from these measurements.
3. **Do not chase RSS alone.** Reinvestigate only when repeated complete-process runs establish sustained growth of *live* objects or a user-visible memory-pressure/latency regression. Profile allocations/call stacks and compare real GUI behavior on the affected OS before changing the allocator or caching policy.
4. Keep the diagnostic distinction explicit: `/proc/self/smaps_rollup` RSS/PSS, glibc `malloc_info` arena estimates and `Counting` live Rust heap are different accounting systems. `VmHWM` is process-lifetime high water, not a Reader-scoped peak.
5. Prefer advancing the Reader fidelity and functional acceptance backlog to another speculative allocator microoptimization.

References: [Linux `malloc_trim(3)`](https://man7.org/linux/man-pages/man3/malloc_trim.3.html), [`malloc_info(3)`](https://man7.org/linux/man-pages/man3/malloc_info.3.html), [Linux `smaps(5)`](https://man7.org/linux/man-pages/man5/proc_pid_smaps.5.html). `mallinfo2` is **not** used as the comprehensive multi-arena source because documented coverage varies and the manual warns about limitations.
