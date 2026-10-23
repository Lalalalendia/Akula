# Akula: bounded pull request queue, 140 -> 50

Policy start: 2026-10-23T16:12:00Z. Initial GitHub inventory: 136 open PRs (132 drafts, 52 targeting non-main branches).

## Admission policy

1. **Absolute ceiling: 140 open PRs**, counting Draft and stacked PRs. Every newly opened or reopened PR is checked after the GitHub event arrives. When the total is over the current ceiling, only that newly submitted PR is commented on and closed. Its branch, commits, and discussion remain intact. To resume, reduce the queue and reopen it.
2. **Target: 50 open PRs.** The dynamic ceiling starts at 140 and falls by one per previously existing PR closed or merged after the policy start, down to 50. A PR is "previously existing" only if created before the policy start. Closing any new PR does **not** lower the cap. No older PR is automatically closed by this rule.
3. **Priority over novelty.** Above 50, focus on finishing existing user-facing PRs and resolving obsolete experimental chains. A security emergency still needs a free slot: retire an obsolete PR first. Do not bypass required CI, change permissions, or merge unsafe drafts to meet the count.
4. **Weekly manual triage.** The existing Branch hygiene workflow reports current open count, current ceiling, target, drafts, stacked PRs, and the 15 oldest-to-review candidates, prioritizing stacked PRs. Review whether to merge (only after exact-head CI and physical acceptance where required), close as superseded, or keep with a blocker and next step.
5. **Branches.** Native GitHub delete-after-merge is enabled. The existing Branch hygiene job prunes only branches whose tip still matches a merged PR. Closed-but-unmerged branches and active stacked branches are not deleted automatically.

## Known limits

- GitHub webhooks run after a PR is created; the rule cannot stop the creation instant. Simultaneous new PRs, GitHub API outages, and unavailable Actions workers can temporarily overshoot the limit; the check does a second API read before closing. A maintainer should review the weekly report for discrepancies.
- Reopening a previously closed legacy PR can move the computed ceiling back upward because GitHub's current `closed_at` is no longer set. Do not reopen retired legacy work merely to regain slots; reopen only after review. The unconditional absolute limit remains 140.
- A passing CI build is not equivalent to real-device or installed-app acceptance. A closed PR can be reopened; neither this policy nor branch cleanup deletes its history.
- A workflow change must merge into `main` before the event guard becomes active. No enforcement is claimed while this policy is still an unmerged draft.

## Backlog reduction procedure

1. First review P0 user-data problems and broken main/security gates (e.g., IME draft #643). Rebase only when necessary and rerun exact-head checks.
2. Finish one real user scenario per batch: PDF import/search (#648), S3 A/B (#649), Dropbox read-only integration (#670), or one-file P2P (#642); require relevant Windows tests and leave incomplete drafts open.
3. For older laboratory/stacked PRs, map the full dependency chain before any closure. Close abandoned/superseded PRs with a traceable reason; retain Git refs unless they are safely merged.
4. Never trade File Gate, RustSec, or signed release safety for a better numeric queue metric.
