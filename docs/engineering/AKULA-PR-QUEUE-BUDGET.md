# Akula: fixed ceiling of 140 open pull requests

Decision: 2026-10-24, owner-approved revision from fixed 130 to fixed 140. The previous shrinking 140-to-50 ratchet remains retired.
The limit is **140 open pull requests**, including Draft and stacked PRs whose base is not `main`.

## Admission policy

1. **Fixed cap: 140 open PRs.** Only `opened` and `reopened` events are checked for rejection. If the new/reopened PR makes the total exceed 140, the automation comments on and closes **that PR only**. No branch, commit, discussion, or existing PR is deleted.
2. **No ratchet or automatic backlog reduction.** Closing or merging *any* open PR releases one slot without lowering the cap. At 140 open, closing one leaves 139, permitting one new PR. The ceiling never falls to 50, and historical dates have no effect on it.
3. **Preserve existing work.** The admission guard never scans and closes old open PRs to meet the target. Backlog triage remains manual; stacked or partially tested branches must not be silently discarded.
4. **Safe priority decisions.** If the queue is full, finish or explicitly close a reviewed superseded PR to open a slot. Do not bypass the `Main PR gate`, weaken RustSec / File Gate, or merge unfinished Drafts simply to reduce the count.
5. **Weekly reporting.** Existing Branch hygiene reports the open count, fixed ceiling, Draft/stacked breakdown, overflow and up to 15 old candidates for manual review.
6. **Security and concurrency.** The admission workflow runs `pull_request_target` from protected default-branch code with narrow permissions; never execute an incoming PR head. Admission events share a global non-cancelling concurrency group. A second API read checks for concurrent merges before closing an incoming PR.

## Practical boundaries

- GitHub triggers the workflow *after* a PR is created. Temporary overshoot is possible before the webhook executes, during GitHub outages, or while admission jobs wait for runners; this is an enforcement policy, not an atomic GitHub-side quota.
- The API fetch is scoped to `state: open`, avoiding the historical closed-PR backlog. A closed PR can be reopened only when the same fixed 140 cap allows it.
- Existing branches are managed separately: pruning may remove only unchanged, safely merged branches. Closed-but-unmerged PR branches remain recoverable.
- Passing CI is not a replacement for installed-app or real-device acceptance. The limit does not change required product or security gates.
- Admission logic becomes effective only after this change is merged into `main`.

## Triage approach

Prioritize P0 data safety, real installed Windows acceptance, and user-visible release scenarios. Review dependency chains before closing any stacked PR. Keep a documented reason for closing superseded work; never alter the backlog merely to make a number look better.
