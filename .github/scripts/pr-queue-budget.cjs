'use strict';

// Fixed PR admission ceiling. Closed/merged PRs release one slot.
const MAX_OPEN_PRS = 140;

function calculateBudget(pulls) {
  if (!Array.isArray(pulls)) throw new TypeError('Expected an array of pull requests');
  const open = pulls.filter(pr => pr.state === 'open');
  return {
    openCount: open.length,
    draftCount: open.filter(pr => pr.draft).length,
    stackedCount: open.filter(pr => pr.base?.ref !== 'main').length,
    ceiling: MAX_OPEN_PRS,
    overflow: Math.max(0, open.length - MAX_OPEN_PRS),
  };
}

function oldestCandidates(pulls, max = 15) {
  return pulls.filter(pr => pr.state === 'open').sort((a, b) => {
    const aStacked = a.base?.ref !== 'main' ? 0 : 1;
    const bStacked = b.base?.ref !== 'main' ? 0 : 1;
    if (aStacked !== bStacked) return aStacked - bStacked;
    return Date.parse(a.updated_at || a.created_at) - Date.parse(b.updated_at || b.created_at);
  }).slice(0, max);
}

async function listPulls(github, owner, repo) {
  // Only open PRs are needed; avoid fetching hundreds of closed PRs.
  return github.paginate(github.rest.pulls.list, { owner, repo, state: 'open', per_page: 100 });
}

async function auditQueue({ github, context, core }) {
  const { owner, repo } = context.repo;
  const pulls = await listPulls(github, owner, repo);
  const result = calculateBudget(pulls);
  const lines = [
    '## Akula PR queue budget',
    `- Open: **${result.openCount}** (Draft: ${result.draftCount}; non-main base: ${result.stackedCount})`,
    `- Fixed admission maximum: **${result.ceiling}**; closing/merging a PR frees one slot`,
    `- Open above ceiling: **${result.overflow}**`,
    '',
    '### Existing PRs for manual triage (not automatically closed)',
    ...oldestCandidates(pulls).map(pr =>
      `- #${pr.number}: ${pr.base?.ref === 'main' ? 'main' : 'stacked'}; ${pr.draft ? 'Draft' : 'review'}; last update ${String(pr.updated_at).slice(0, 10)}; https://github.com/${owner}/${repo}/pull/${pr.number}`),
    '',
    'Close or merge superseded PRs only after reviewing dependencies and required checks. Never silently delete unmerged branches.',
  ];
  await core.summary.addRaw(lines.join('\n')).write();
  core.info(`PR budget open=${result.openCount} fixed_ceiling=${result.ceiling}`);
  return result;
}

async function enforceAdmission({ github, context, core }) {
  if (!['opened', 'reopened'].includes(context.payload?.action)) {
    throw new Error('Only PR opened/reopened events are admissible');
  }
  const { owner, repo } = context.repo;
  const number = context.payload.pull_request?.number;
  if (!Number.isSafeInteger(number) || number <= 0) throw new Error('Invalid event PR number');
  const { data: incoming } = await github.rest.pulls.get({ owner, repo, pull_number: number });
  if (incoming.state !== 'open') return { admitted: false, reason: 'already_closed' };

  let current = calculateBudget(await listPulls(github, owner, repo));
  if (!current.overflow) {
    core.info(`PR #${number} admitted: open=${current.openCount}, cap=${current.ceiling}`);
    return { admitted: true, current };
  }
  // A second read prevents closing on an outdated snapshot after parallel merges.
  current = calculateBudget(await listPulls(github, owner, repo));
  const { data: latest } = await github.rest.pulls.get({ owner, repo, pull_number: number });
  if (latest.state !== 'open') return { admitted: false, reason: 'already_closed' };
  if (!current.overflow) return { admitted: true, current };

  const note = [
    '**Akula PR admission budget: this new/reopened PR exceeds the queue ceiling.**',
    '',
    `Current queue: ${current.openCount} open; fixed maximum: ${current.ceiling}.`,
    'The maximum does not shrink. Closing or merging any PR releases one slot.',
    'To resume, merge or close a PR after checking its dependencies, then reopen when the total fits the fixed maximum.',
    'No commit or branch is deleted by this admission rule. Urgent fixes need a slot too; retire lower-priority work first.',
    '',
    'Policy: docs/engineering/AKULA-PR-QUEUE-BUDGET.md',
  ].join('\n');
  try {
    await github.rest.issues.createComment({ owner, repo, issue_number: number, body: note });
  } catch (error) {
    // Keep the hard ceiling even if PR comments are unavailable.
    core.warning(`PR #${number}: comment unavailable (${error.message})`);
  }
  await github.rest.pulls.update({ owner, repo, pull_number: number, state: 'closed' });
  core.warning(`PR #${number} closed by PR budget: open=${current.openCount}, cap=${current.ceiling}`);
  return { admitted: false, reason: 'over_ceiling', current };
}

module.exports = {
  MAX_OPEN_PRS,
  calculateBudget, oldestCandidates, listPulls, auditQueue, enforceAdmission,
};
