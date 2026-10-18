# Akula — adaptive Vello for one small dirty rectangle

The default render path uses a persistent zero-worker Vello context for **exactly
one nonempty final damage rectangle of at most 350,000 physical pixels**, when
the configured renderer has one or more worker threads. A large, fragmented or
empty frame always uses the original parallel context. The policy operates
only after Damage Tracker and live-layer updates finalize the region list.

This is a narrow promotion of the **existing** opt-in implementation from
[#218](https://github.com/Lalalalendia/Akula/pull/218). It does not add damage
merging, change glyph cache keys, alter atlas sampling, or weaken pixel parity.

## Immediate rollback

- `AKULA_VELLO_SINGLE_RECT=0` before starting the app disables the adaptive
  companion and restores the exact fixed-worker renderer (also avoids its
  extra memory allocation).
- The pre-existing experimental variable
  `AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT=0` retains the same disable behavior
  for compatibility. A value of `1` is now redundant. Either variable
  explicitly set to `0` wins.
- Zero-worker renderers never allocate a second context.
- The CPU-only branch choice is made once per frame, before encoding, not
  separately for each dirty region. On trim both persistent contexts are
  recreated without changing the selected worker policy.

## Existing measurements

The source is the same small-damage policy already measured by Akula #218:
a nine-process release-mode Linux RSS/A-B comparison (3 trials for fixed
serial, fixed two workers and adaptive, rotated order) found **16% lower median
sum of sampled mixed-frame times** than the best fixed policy:
16.584 ms versus 19.735 ms. The routed process peak VmHWM median was
51.54 MiB versus 50.34 MiB fixed serial (about **1.2 MiB higher**);
routed p95 was 1013 µs versus fixed serial 1473 µs on that one runner.
These measurements are not platform-general performance guarantees.

After #218, [#260](https://github.com/Lalalalendia/Akula/pull/260)
fixed the independent glyph alpha tie in `vello_common`: 20/20 strict
guard96 full-frame Linux runs, 5/5 each Windows and macOS; 32 changed pixels
out of 9,757,000 in a clean original-versus-patched full-frame A/B, none
above 1/255. This does not by itself prove the adaptive policy is faster.

## Release acceptance

1. All platform CI, normal scroll frame parity and existing full/crop
   outline-glyph tests stay green (strict 1/255 channel tolerance).
2. Test-only fixed-worker controls remain true *fixed-worker* renderers:
   `Renderer::new_fixed_for_test` bypasses the default route, so A/B cannot
   accidentally compare two adaptive contexts.
3. Re-run mixed 64-frame route parity and Linux 9-process memory/p95 A/B
   with the final vendored `vello_common`. Reject a default rollout if
   repeated timings regress, if process RSS grows disproportionately, or
   if there are glyph/crop pixel mismatches.
4. Confirm `AKULA_VELLO_SINGLE_RECT=0` exercises the original fixed path;
   the previously released setting `AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT=0`
   must work as a rollback too.

Do not increase the 350k threshold or enable multi-rectangle batching
without separate new measurements. No hardware or other routing defaults
are modified here.
