// Prevent silently publishing a stale unsigned helper after a change to its
// trust/owner/IPC verifier. The integrated branch gate runs this even if a
// broken candidate paths filter would suppress the candidate workflow.
'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const workflow = fs.readFileSync(
  path.join(__dirname, '../workflows/akula-root-helper-candidate.yml'), 'utf8'
);
const scope = workflow.match(/^    paths:\n((?:      (?:#.*|- '[^']+')\n)+)/m);
assert.ok(scope, 'candidate must retain a push.paths allowlist');
const paths = [...scope[1].matchAll(/^      - '([^']+)'$/gm)].map(m => m[1]);
assert.equal(paths.some(p => p.startsWith('!')), false, 'no silent negative path overrides');
function rebuildsFor(file) {
  return paths.some(p => p === file ||
    (p.endsWith('/**') && file.startsWith(p.slice(0, -3) + '/')));
}

test('every direct root-helper trust, release and IPC implementation triggers rebuild', () => {
  for (const file of [
    'aquilum-app/core/src/bin/akula_root_b1d0_helper.rs',
    'aquilum-app/core/src/server_root_v3_trust.rs',
    'aquilum-app/core/src/server_root_v3_trust/root_helper_entry.rs',
    'aquilum-app/core/src/server_root_v3_trust/bootstrap_helper.rs',
    'aquilum-app/core/src/server_root_v3_trust/bootstrap_helper/running.rs',
    'aquilum-app/core/src/server_root_v3_trust/owner_ipc.rs',
    'aquilum-app/core/src/server_root_v3_trust/owner_ipc/enrollment.rs',
    'aquilum-app/core/src/server_root_v3_trust/owner_ipc/enrollment/shadow.rs',
    'aquilum-app/core/src/server_root_v3_trust/owner_frame.rs',
    'aquilum-app/core/src/server_root_v3_guard.rs',
    'aquilum-app/core/src/server_root_v3_guard/tests.rs',
    'aquilum-app/core/src/server_first_root_stage.rs',
    'aquilum-app/core/src/server_release_lab.rs',
    'aquilum-app/core/src/server_release_lab/review.rs',
    'aquilum-app/core/src/server_owner_approval_lab.rs',
    'aquilum-app/core/src/server_owner_approval_wire.rs',
    'aquilum-app/core/src/server_owner_approval_wire/decode.rs',
    'aquilum-app/core/src/server_installer_pinned_release_lab.rs',
    'aquilum-app/core/src/server_installer_ipc_lab.rs',
    'aquilum-app/core/src/server_installer_durable_node.rs',
    'aquilum-app/core/src/server_installer_durable_node/provisioned_gate.rs',
    'aquilum-app/core/src/lib.rs',
    'aquilum-app/core/Cargo.toml',
    'aquilum-app/Cargo.lock',
    'modules/bootstrap-b0/src/lib.rs',
    '.github/scripts/b1-e2e07-candidate-routing.test.cjs',
    '.github/workflows/akula-root-helper-candidate.yml',
  ]) assert.ok(rebuildsFor(file), 'missing protected candidate input: ' + file);
});

test('unrelated app, docs and CI edits do not fan out to candidate release build', () => {
  for (const file of [
    'aquilum-app/core/src/search/worker.rs',
    'aquilum-app/native/src/app/settings/server.rs',
    'docs/platforms/node-v1/B1-E2E-07-INTEGRATED-INSTALL.md',
    '.github/workflows/app-checks.yml',
  ]) assert.equal(rebuildsFor(file), false, 'spurious candidate rebuild: ' + file);
});

test('candidate is explicitly opt-in, unsigned and refuses installation', () => {
  assert.match(workflow, /^  workflow_dispatch:\s*$/m);
  assert.match(workflow, /^permissions:\n  contents: read$/m);
  assert.match(workflow, /UNSIGNED-NOT-FOR-INSTALL/);
  assert.match(workflow, /release_signature=ABSENT/);
  assert.match(workflow, /privileged_install_dispatch=DISABLED/);
  assert.match(workflow, /root_service_activation=DISABLED/);
});
