# Akula branch hygiene and merge policy

## Intent

Keep `main` protected and retain unmerged work while avoiding abandoned Git refs.

## Configurations that require a repository administrator

The GitHub integration used for this setup cannot edit repository settings or branch protection. An administrator must configure **Settings > General > Pull Requests**:

1. Enable **Allow auto-merge**. This makes auto-merge *available*; it does not automatically enqueue every PR.
2. Enable **Automatically delete head branches**. GitHub will then handle future merged branches natively, in addition to the fallback workflow.

Under **Settings > Rules > Rulesets**, create an active branch ruleset for `main`:

- Require pull requests before merging; block force pushes and deletions.
- Require status checks that are currently relevant to the changed paths. Start from observed successful checks on actual PRs; do not guess check names. Reconcile path-filtered workflows to avoid permanently pending checks.
- Require the branch to be up to date before merging, if this fits the CI workflow, or use merge queue once configured.
- Avoid bypass for automation unless audited, narrowly scoped and necessary.
- Use a review requirement if review resources exist; do not claim self-approval can satisfy the policy.

**Important:** Merely enabling `allow_auto_merge` does not protect `main`. Do not activate unconditional merge bots before required checks and protections are enforced. Draft PRs must remain unmerged.

## Workflow

`.github/workflows/branch-hygiene.yml` runs:

- Unit tests for safety invariants whenever its script/workflow changes in a PR.
- A cleanup pass after a PR is merged, without checking out untrusted PR code.
- A cleanup pass on the initial push to `main` that adds or modifies the workflow.
- A weekly read-only audit on Mondays at 04:23 UTC.
- A manual `workflow_dispatch` with `audit` (default) or `prune` mode.

Cleanup is allowed **only** for a branch that satisfies *all* of these conditions:

1. It is not the default branch and GitHub does not mark it protected.
2. It belongs to this repository, not a fork.
3. A merged PR points to that exact same branch and head commit SHA.
4. No open PR references the branch, including a final recheck before deletion.
5. The Git ref still points to the same SHA immediately before deletion.

A branch that belonged to a closed-but-unmerged PR, or has no associated PR, is never automatically deleted. A reused branch with new commits is also preserved. Deletions target refs, not commits, PR history or merged changes. GitHub can still refuse a deletion; the workflow logs it.

The workflow reports candidates, deleted refs and retained classes in the run summary. It caps automatic deletions at 50 refs per run to limit surprises. Any admin policy that enables native branch deletion remains preferable to this fallback.

## One-time state observed 2026-10-17

- `main`: 1 branch, not protected; repository rulesets list empty.
- 33 merged PR branches with matching current SHA: eligible for pruning after a final recheck.
- 6 active draft PR branches: keep.
- 8 closed/unmerged PR branches: **keep**, each contains commits absent from `main`.
- 1 branch without any PR (`feature/sync-stage03-durable-journal-20261017`): **keep**, three commits ahead of `main`.
- Native repo settings: `allow_auto_merge=false`, `delete_branch_on_merge=false`.

The 9 unmerged/unlinked branches need a separate review before deletion. Do not infer that "closed" means "merged" or "safe to delete".
