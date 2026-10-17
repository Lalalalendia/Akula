'use strict';

/**
 * Find only branches whose current tip matches the head of a merged PR.
 * This never treats a closed-but-unmerged PR, or an unlinked branch, as safe.
 */
function classifyBranches({ branches, pulls, repository }) {
  const byHead = new Map();
  for (const pr of pulls) {
    if (pr.head?.repo?.full_name !== repository.full_name || !pr.head.ref) continue;
    const list = byHead.get(pr.head.ref) || [];
    list.push(pr);
    byHead.set(pr.head.ref, list);
  }

  const candidates = [];
  const held = [];
  for (const branch of branches) {
    const prs = byHead.get(branch.name) || [];
    let reason;
    if (branch.name === repository.default_branch) reason = 'default_branch';
    else if (branch.protected) reason = 'protected';
    else if (prs.some((pr) => pr.state === 'open')) reason = 'open_pr';
    else if (prs.some((pr) => pr.merged_at && pr.head.sha === branch.commit.sha)) {
      candidates.push({ name: branch.name, sha: branch.commit.sha });
      continue;
    } else if (prs.some((pr) => pr.merged_at)) reason = 'merged_pr_sha_changed';
    else if (prs.length) reason = 'closed_unmerged_pr';
    else reason = 'no_matching_pr';
    held.push({ name: branch.name, reason });
  }
  return { candidates, held };
}

async function run({ github, context, core, mode, maxDeletes = 50 }) {
  if (!['audit', 'prune'].includes(mode)) throw new Error('Invalid branch cleanup mode');
  const { owner, repo } = context.repo;
  const [repoResult, branches, pulls] = await Promise.all([
    github.rest.repos.get({ owner, repo }),
    github.paginate(github.rest.repos.listBranches, { owner, repo, per_page: 100 }),
    github.paginate(github.rest.pulls.list, { owner, repo, state: 'all', per_page: 100 }),
  ]);
  const repository = repoResult.data;
  if (repository.full_name !== `${owner}/${repo}`) throw new Error('Repository identity mismatch');
  const { candidates, held } = classifyBranches({ branches, pulls, repository });
  const deleted = [];
  const skipped = [];
  const failures = [];

  if (mode === 'prune') {
    for (const candidate of candidates.slice(0, maxDeletes)) {
      try {
        // Recheck for a newly opened PR, rather than relying on the initial snapshot.
        const { data: open } = await github.rest.pulls.list({
          owner, repo, state: 'open', head: `${owner}:${candidate.name}`, per_page: 2,
        });
        if (open.length) {
          skipped.push({ name: candidate.name, reason: 'new_open_pr' });
          continue;
        }
        const ref = `heads/${candidate.name}`;
        const { data: current } = await github.rest.git.getRef({ owner, repo, ref });
        if (current.object?.type !== 'commit' || current.object.sha !== candidate.sha) {
          skipped.push({ name: candidate.name, reason: 'head_sha_changed' });
          continue;
        }
        // Deleting a Git ref never deletes commit history or merged PR records.
        await github.rest.git.deleteRef({ owner, repo, ref });
        deleted.push(candidate.name);
      } catch (error) {
        if (error.status === 404 || error.status === 422) {
          skipped.push({ name: candidate.name, reason: `GitHub_${error.status}` });
        } else {
          failures.push(`${candidate.name}: ${error.message}`);
        }
      }
    }
  }

  const counts = new Map();
  for (const branch of held) counts.set(branch.reason, (counts.get(branch.reason) || 0) + 1);
  const lines = [
    '## Branch hygiene',
    `- Mode: **${mode}**`,
    `- Merged-and-unchanged branch candidates: **${candidates.length}**`,
    `- Deleted refs: **${deleted.length}**`,
    `- Skipped at final safety check: **${skipped.length}**`,
    `- Errors: **${failures.length}**`,
    '',
    '### Kept branches',
    ...[...counts.entries()].sort().map(([reason, count]) => `- ${reason}: ${count}`),
    '',
    '### Deleted',
    ...(deleted.length ? deleted.map((name) => `- \`${name}\``) : ['- None']),
    '',
    '### Skipped / needs review',
    ...(skipped.length ? skipped.map((entry) => `- \`${entry.name}\`: ${entry.reason}`) : ['- None']),
    ...(failures.length ? ['', '### Errors', ...failures.map((message) => `- ${message}`)] : []),
  ];
  if (candidates.length > maxDeletes && mode === 'prune') {
    lines.push('', `Cleanup limited to ${maxDeletes} branches per run.`);
  }
  await core.summary.addRaw(lines.join('\n')).write();
  core.info(`Branch hygiene mode=${mode}: candidates=${candidates.length} deleted=${deleted.length} held=${held.length} skipped=${skipped.length}`);
  if (failures.length) throw new Error(`Branch hygiene could not process ${failures.length} refs`);
  return { candidates, held, deleted, skipped };
}

module.exports = { classifyBranches, run };
