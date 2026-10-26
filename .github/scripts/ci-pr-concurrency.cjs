'use strict';

const cp = require('node:child_process');
const fs = require('node:fs');

// Fail closed for NEW GitHub Actions PR workflows. Existing jobs retain their
// tested required checks/routing; the policy adds no new Actions runners.
const WORKFLOW = /^\.github\/workflows\/[^/\0]+\.ya?ml$/;

function topBlock(yaml, name) {
  const lines = String(yaml).replace(/\r\n/g, '\n').split('\n');
  const re = new RegExp('^(?:' + name + '|"' + name + '"|\\x27' + name + '\\x27):(?:\\s|$)');
  const i = lines.findIndex(line => re.test(line));
  if (i < 0) return [];
  const out = [lines[i]];
  for (let j = i + 1; j < lines.length; j++) {
    if (/^[^\s#][^:]*:\s*/.test(lines[j])) break;
    out.push(lines[j]);
  }
  return out;
}

function prEvents(yaml) {
  const on = topBlock(yaml, 'on');
  if (on.length === 0) return [];
  const inline = on[0].replace(/^(['"]?on['"]?):\s*/, '');
  // Track ancestor indentation rather than assuming all event keys use
  // the same indentation. Nested workflow_dispatch inputs are not triggers.
  const rootEvents = new Set();
  const parents = [];
  for (const line of on.slice(1)) {
    const match = line.match(/^(\s+)(['"]?)([\w-]+)\2:(?:\s|$)/);
    if (!match) continue;
    const depth = match[1].length;
    while (parents.length && parents[parents.length - 1] >= depth) parents.pop();
    if (parents.length === 0) rootEvents.add(match[3]);
    parents.push(depth);
  }
  return ['pull_request', 'pull_request_target'].filter(event => {
    const inlineName = new RegExp('\\b' + event + '\\b');
    return inlineName.test(inline) || rootEvents.has(event);
  });
}
function prTrigger(yaml) { return prEvents(yaml).length > 0; }

function policyErrors(yaml, path) {
  if (!prTrigger(yaml)) return [];
  const lines = topBlock(yaml, 'concurrency');
  if (!lines.length) return [path + ': missing workflow-level concurrency'];
  const group = lines.find(line => /^  group:/.test(line)) || '';
  const cancel = lines.find(line => /^  cancel-in-progress:/.test(line)) || '';
  const clean = value => value.replace(/\s+#.*$/, '');
  const groupValue = clean(group.replace(/^  group:\s*/, ''));
  const cancelValue = clean(cancel.replace(/^  cancel-in-progress:\s*/, ''));
  const errors = [];
  // Match real Actions interpolation, not a descriptive YAML comment.
  if (!/\$\{\{[^}]*github\.workflow[^}]*\}\}/.test(groupValue) ||
      !/\$\{\{[^}]*github\.event\.pull_request\.number[^}]*github\.ref[^}]*\}\}/.test(groupValue)) {
    errors.push(path + ': group must isolate workflow and PR number, with ref fallback');
  }
  // Do not silently cancel workflow_dispatch, pushes or released artifacts.
  const condition = cancelValue.match(/^\$\{\{\s*(.*?)\s*\}\}$/)?.[1] || '';
  // Allow only explicit PR event equalities. Substring matching would accept
  // "github.event_name == 'pull_request' || true", cancelling manual/releases.
  const normalized = condition.replace(/\s+/g, '').replace(/"/g, "'");
  const expected = prEvents(yaml).map(event => "github.event_name=='" + event + "'");
  const options = expected.length === 1
    ? [expected[0]]
    : [expected.join('||'), expected.slice().reverse().join('||')];
  const actual = normalized.startsWith('(') && normalized.endsWith(')')
    ? normalized.slice(1,-1) : normalized;
  if (!options.includes(actual)) {
    errors.push(path + ': cancel-in-progress must cover PR triggers only');
  }
  if (/^  queue:\s*max\b/m.test(lines.join('\n'))) {
    errors.push(path + ': queue: max defeats obsolete-run coalescing');
  }
  return errors;
}

function allowedTextDeltaMigration(before, after, read) {
  const original = "on:\n  pull_request:\n    paths:\n"
    +"      - 'modules/text-delta-codec/**'\n"
    +"      - '.github/workflows/text-delta-codec-isolated.yml'\n"
    +"  workflow_dispatch:\n";
  const replacement = "on:\n"
    +"  # PR source owners run in the required Application checks / Main PR gate.\n"
    +"  workflow_dispatch:\n";
  if(before.split(original).length!==2 || after!==before.replace(original,replacement))
    return false;
  let app,scope,tests;
  try {
    app=read('.github/workflows/app-checks.yml');
    scope=read('.github/scripts/ci-rust-scope.cjs');
    tests=read('.github/scripts/ci-rust-scope.test.cjs');
  }catch{return false;}
  if(![app,scope,tests].every(x=>typeof x==='string'))return false;
  const mustHave=[
    "      text_delta_module_required: ${{ steps.scope.outputs.text_delta_module_required }}",
    "node .github/scripts/ci-rust-scope.cjs --base=\"$BASE_SHA\" --head=HEAD >> \"$GITHUB_OUTPUT\"",
    "if: needs.classify.outputs.mode != 'none' || needs.classify.outputs.updater_security_required == 'true' || needs.classify.outputs.text_delta_module_required == 'true'",
    "name: Require standalone Text Delta wire and hostile-input tests",
    "if: needs.classify.outputs.text_delta_module_required == 'true'",
    "cargo test --manifest-path modules/text-delta-codec/Cargo.toml -- --nocapture",
    "    needs: [classify, required-tests, native-frames, shared-labs]",
    "if [ \"$CLASSIFY_RESULT\" != success ] || [ \"$REQUIRED_RESULT\" != success ]; then"
  ];
  if(!mustHave.every(x=>app.includes(x)))return false;
  if(!scope.includes('function textDeltaModuleRequired(changedPaths) {') ||
     !scope.includes("p.startsWith('modules/text-delta-codec/')") ||
     !scope.includes("p === '.github/workflows/text-delta-codec-isolated.yml'") ||
     !scope.includes("text_delta_module_required='+textDeltaModuleRequired(paths)"))return false;
  if(!tests.includes('Text Delta parity lab never duplicates Core source PR workflow runs') ||
     !tests.includes('assert.equal(textDeltaModuleRequired([leaf]),true'))return false;
  return true;
}

function allowedWindowsBetaMigration(before, after, read) {
  // Only the byte-preserving Windows beta PR -> required callable route change.
  const owned = [
    '.github/workflows/windows-beta.yml',
    '.github/scripts/windows-beta-identity.test.cjs',
    'scripts/test-windows-beta.ps1',
    'scripts/build-windows-update-fixture.ps1',
    'scripts/test-windows-beta-upgrade.ps1',
    'scripts/windows-beta-gui-input.ps1',
    'scripts/windows-beta-candidate-manifest.ps1',
    'scripts/windows-accesskit-uia.ps1',
    'aquilum-app/native/packaging/akula-windows.json',
    'aquilum-app/native/Cargo.toml',
  ];
  const oldHeader = 'on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n'
    + owned.map(path => "      - '" + path + "'\n").join('')
    + '\n  workflow_dispatch:\n';
  const newHeader = 'on:\n  workflow_call:\n'
    + '  # Source ownership is enforced by the required Main PR gate.\n'
    + '  workflow_dispatch:\n';
  if (before.split(oldHeader).length !== 2 || after !== before.replace(oldHeader, newHeader) ||
      !after.includes('  workflow_dispatch:\n    inputs:\n      ref:\n')) return false;
  let routes, app, shared;
  try {
    routes = read('.github/scripts/ci-shared-manifest-route.cjs');
    app = read('.github/workflows/app-checks.yml');
    shared = read('.github/workflows/ci-shared-manifest-labs.yml');
  } catch { return false; }
  if (![routes, app, shared].every(x => typeof x === 'string')) return false;
  const list = routes.match(/^  "owned_windows_beta": (\[[\s\S]*?^  \])/m)?.[1];
  let actual;
  try { actual = JSON.parse(list); } catch { return false; }
  if (JSON.stringify(actual) !== JSON.stringify(owned) ||
      !routes.includes('function ownedRoutes(paths){\n  const changed=new Set(paths);') ||
      !routes.includes('...ownedRoutes(paths)') ||
      !routes.includes('module.exports={OWNED_SUITES,ownedRoutes,')) return false;
  const classification = app.split('  required-tests:\n')[0];
  const sharedJob = app.split('  shared-labs:\n')[1]?.split('  checks:\n')[0] || '';
  const mainGate = app.split('  checks:\n')[1] || '';
  if (!classification.includes('      owned_windows_beta: ${{ steps.scope.outputs.owned_windows_beta }}') ||
      !classification.includes('node .github/scripts/ci-shared-manifest-route.cjs --base="$BASE_SHA" --head=HEAD >> "$GITHUB_OUTPUT"') ||
      !sharedJob.includes("needs.classify.outputs.owned_windows_beta == 'true'") ||
      !sharedJob.includes("      owned_windows_beta: ${{ needs.classify.outputs.owned_windows_beta == 'true' }}") ||
      !sharedJob.includes('uses: ./.github/workflows/ci-shared-manifest-labs.yml') ||
      !mainGate.includes('    name: Main PR gate') ||
      !mainGate.includes("    if: ${{ always() && github.event_name == 'pull_request' }}") ||
      !mainGate.includes('    needs: [classify, required-tests, native-frames, shared-labs]') ||
      !mainGate.includes('          SHARED_RISKY: ${{') ||
      !mainGate.includes("needs.classify.outputs.owned_windows_beta == 'true'") ||
      !mainGate.includes('          ROUTED_SHARED_RESULT: ${{ needs.shared-labs.result }}') ||
      !mainGate.includes('if [ "$SHARED_RISKY" = true ]; then') ||
      !mainGate.includes('if [ "$ROUTED_SHARED_RESULT" != success ]; then')) return false;
  if (!shared.includes('      owned_windows_beta:\n        required: true\n        type: boolean') ||
      !shared.includes('  route-windows-beta:\n    name: Impacted shared manifest / windows-beta\n    if: ${{ inputs.lock || inputs.owned_windows_beta }}\n    uses: ./.github/workflows/windows-beta.yml') ||
      !/^    needs: \[[^\n]*route-windows-beta[^\n]*\]$/m.test(shared) ||
      !shared.includes('"route-windows-beta",["lock","owned_windows_beta"]')) return false;
  return true;
}

function allowedLinuxBetaMigration(before, after, read) {
  // Only the byte-preserving Linux beta PR -> required callable route change.
  const owned = [
    '.github/workflows/linux-beta.yml',
    '.github/scripts/linux-beta-identity.test.cjs',
    'scripts/linux-beta-smoke.sh',
    'aquilum-app/native/packaging/akula-linux-nfpm.yaml',
    'aquilum-app/native/packaging/Akula.desktop',
    'aquilum-app/native/Cargo.toml',
  ];
  const oldHeader = 'on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n'
    + owned.map(path => "      - '" + path + "'\n").join('')
    + '\npermissions:\n';
  const newHeader = 'on:\n  workflow_call:\n'
    + '  # PR source owners are enforced by the required Main PR gate.\n'
    + '\npermissions:\n';
  if (before.split(oldHeader).length !== 2 || after !== before.replace(oldHeader, newHeader) ||
      after.includes('  workflow_dispatch:\n')) return false;
  let routes, app, shared;
  try {
    routes = read('.github/scripts/ci-shared-manifest-route.cjs');
    app = read('.github/workflows/app-checks.yml');
    shared = read('.github/workflows/ci-shared-manifest-labs.yml');
  } catch { return false; }
  if (![routes, app, shared].every(x => typeof x === 'string')) return false;
  const list = routes.match(/^  "owned_linux_beta": (\[[\s\S]*?^  \])/m)?.[1];
  let actual;
  try { actual = JSON.parse(list); } catch { return false; }
  if (JSON.stringify(actual) !== JSON.stringify(owned) ||
      !routes.includes('function ownedRoutes(paths){\n  const changed=new Set(paths);') ||
      !routes.includes('...ownedRoutes(paths)') ||
      !routes.includes('module.exports={OWNED_SUITES,ownedRoutes,')) return false;
  const classification = app.split('  required-tests:\n')[0];
  const sharedJob = app.split('  shared-labs:\n')[1]?.split('  checks:\n')[0] || '';
  const mainGate = app.split('  checks:\n')[1] || '';
  if (!classification.includes('      owned_linux_beta: ${{ steps.scope.outputs.owned_linux_beta }}') ||
      !classification.includes('node .github/scripts/ci-shared-manifest-route.cjs --base="$BASE_SHA" --head=HEAD >> "$GITHUB_OUTPUT"') ||
      !sharedJob.includes("needs.classify.outputs.owned_linux_beta == 'true'") ||
      !sharedJob.includes("      owned_linux_beta: ${{ needs.classify.outputs.owned_linux_beta == 'true' }}") ||
      !sharedJob.includes('uses: ./.github/workflows/ci-shared-manifest-labs.yml') ||
      !mainGate.includes('    name: Main PR gate') ||
      !mainGate.includes("    if: ${{ always() && github.event_name == 'pull_request' }}") ||
      !mainGate.includes('    needs: [classify, required-tests, native-frames, shared-labs]') ||
      !mainGate.includes('          SHARED_RISKY: ${{') ||
      !mainGate.includes("needs.classify.outputs.owned_linux_beta == 'true'") ||
      !mainGate.includes('          ROUTED_SHARED_RESULT: ${{ needs.shared-labs.result }}') ||
      !mainGate.includes('if [ "$SHARED_RISKY" = true ]; then') ||
      !mainGate.includes('if [ "$ROUTED_SHARED_RESULT" != success ]; then')) return false;
  if (!shared.includes('      owned_linux_beta:\n        required: true\n        type: boolean') ||
      !shared.includes('  route-linux-beta:\n    name: Impacted shared manifest / linux-beta\n    if: ${{ inputs.lock || inputs.owned_linux_beta }}\n    uses: ./.github/workflows/linux-beta.yml') ||
      !/^    needs: \[[^\n]*route-linux-beta[^\n]*\]$/m.test(shared) ||
      !shared.includes('"route-linux-beta",["lock","owned_linux_beta"]')) return false;
  return true;
}


function allowedB1b2bMigration(before, after, read) {
  // Permit only the byte-exact ten-owner PR -> required signed Ubuntu VM route.
  const owned=[
    "aquilum-app/core/examples/akula_b1b2b_release_fixture.rs",
    "scripts/ci/akula-b1b2b-ubuntu-vm.sh",
    "scripts/ci/akula-b1b2b-guest-lab.sh",
    "scripts/ci/akula-b1b2b-node-wrapper.sh",
    "docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B2B.md",
    "docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B2B1.md",
    "docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B2B2.md",
    ".github/workflows/core-server-bootstrap-b1b2b-ubuntu.yml",
    "aquilum-app/core/src/server_release_lab.rs",
    "aquilum-app/core/src/bin/akula_node_lab.rs"
];
  const oldHeader='on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n'
    +owned.map((p,i)=>(i===8?'\n':'')+'      - "'+p+'"\n').join('')
    +'  workflow_dispatch:\n';
  const newHeader='on:\n  workflow_call:\n  workflow_dispatch:\n';
  if(before.split(oldHeader).length!==2 || after!==before.replace(oldHeader,newHeader))
    return false;
  let routes,app,shared;
  try {
    routes=read('.github/scripts/ci-shared-manifest-route.cjs');
    app=read('.github/workflows/app-checks.yml');
    shared=read('.github/workflows/ci-shared-manifest-labs.yml');
  }catch{return false;}
  if(![routes,app,shared].every(x=>typeof x==='string'))return false;
  const list=routes.match(/^  "owned_b1b2b_vm": (\[[\s\S]*?^  \]),/m)?.[1];
  let actual;
  try{actual=JSON.parse(list);}catch{return false;}
  if(JSON.stringify(actual)!==JSON.stringify(owned) ||
    !routes.includes('function ownedRoutes(paths){\n  const changed=new Set(paths);') ||
    !routes.includes('...ownedRoutes(paths)') ||
    !routes.includes('module.exports={OWNED_SUITES,ownedRoutes,'))return false;
  const classify=app.split('  required-tests:\n')[0];
  const required=app.split('  shared-labs:\n')[1]?.split('  checks:\n')[0]||'';
  const routeIf=required.split('    uses: ./.github/workflows/ci-shared-manifest-labs.yml')[0];
  const gate=app.split('  checks:\n')[1]||'';
  if(!classify.includes('      owned_b1b2b_vm: $'+'{{ steps.scope.outputs.owned_b1b2b_vm }}') ||
    !classify.includes('node .github/scripts/ci-shared-manifest-route.cjs --base="$BASE_SHA" --head=HEAD >> "$GITHUB_OUTPUT"') ||
    !routeIf.includes("needs.classify.outputs.owned_b1b2b_vm == 'true'") ||
    !required.includes('      owned_b1b2b_vm: $'+"{{ needs.classify.outputs.owned_b1b2b_vm == 'true' }}") ||
    !required.includes('uses: ./.github/workflows/ci-shared-manifest-labs.yml') ||
    !gate.includes('    name: Main PR gate') ||
    !gate.includes('    if: $'+"{{ always() && github.event_name == 'pull_request' }}") ||
    !gate.includes('    needs: [classify, required-tests, native-frames, shared-labs]') ||
    !gate.includes("needs.classify.outputs.owned_b1b2b_vm == 'true'") ||
    !gate.includes('          ROUTED_SHARED_RESULT: $'+'{{ needs.shared-labs.result }}') ||
    !gate.includes('if [ "$SHARED_RISKY" = true ]; then') ||
    !gate.includes('if [ "$ROUTED_SHARED_RESULT" != success ]; then'))return false;
  if(!shared.includes('      owned_b1b2b_vm:\n        required: true\n        type: boolean') ||
    !shared.includes('  route-b1b2b-ubuntu:\n    name: Impacted shared manifest / b1b2b-ubuntu\n    if: $'+'{{ inputs.core || inputs.owned_b1b2b_vm }}\n    uses: ./.github/workflows/core-server-bootstrap-b1b2b-ubuntu.yml') ||
    !/^    needs: \[[^\n]*route-b1b2b-ubuntu[^\n]*\]$/m.test(shared) ||
    !shared.includes('"route-b1b2b-ubuntu",["core","owned_b1b2b_vm"]'))return false;
  return true;
}


function allowedB1b3cMigration(before, after, read) {
  // Allow this protected PR-trigger removal only for byte-exact signed FD / systemd proof.
  const oldHeader="on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n\n      - \"aquilum-app/core/src/server_installer_pinned_release_lab.rs\"\n      - \"aquilum-app/core/tests/server_bootstrap_b1b3c.rs\"\n      - \"aquilum-app/core/src/bin/akula_helper_b1b3b_lab.rs\"\n      - \"aquilum-app/core/examples/akula_b1b3c_fixture.rs\"\n      - \"scripts/ci/akula-b1b3c-root-helper-lab.sh\"\n      - \".github/workflows/core-server-bootstrap-b1b3c-signed-fd.yml\"\n      - \"docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B3C.md\"\n  workflow_dispatch:\n";
  const newHeader="on:\n  workflow_call:\n  workflow_dispatch:\n";
  const owned=[
    "aquilum-app/core/src/server_installer_pinned_release_lab.rs",
    "aquilum-app/core/tests/server_bootstrap_b1b3c.rs",
    "aquilum-app/core/src/bin/akula_helper_b1b3b_lab.rs",
    "aquilum-app/core/examples/akula_b1b3c_fixture.rs",
    "scripts/ci/akula-b1b3c-root-helper-lab.sh",
    ".github/workflows/core-server-bootstrap-b1b3c-signed-fd.yml",
    "docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B3C.md"
];
  if(before.split(oldHeader).length!==2 || after!==before.replace(oldHeader,newHeader))
    return false;
  let routes,app,shared;
  try {
    routes=read('.github/scripts/ci-shared-manifest-route.cjs');
    app=read('.github/workflows/app-checks.yml');
    shared=read('.github/workflows/ci-shared-manifest-labs.yml');
  }catch{return false;}
  if(![routes,app,shared].every(x=>typeof x==='string'))return false;
  const list=routes.match(/^  "owned_b1b3c_fd": (\[[\s\S]*?^  \]),/m)?.[1];
  let actual;
  try{actual=JSON.parse(list);}catch{return false;}
  if(JSON.stringify(actual)!==JSON.stringify(owned) ||
      !routes.includes('function ownedRoutes(paths){\n  const changed=new Set(paths);') ||
      !routes.includes('...ownedRoutes(paths)') ||
      !routes.includes('module.exports={OWNED_SUITES,ownedRoutes,'))return false;
  const classify=app.split('  required-tests:\n')[0];
  const required=app.split('  shared-labs:\n')[1]?.split('  checks:\n')[0]||'';
  const requiredIf=required.split('    uses: ./.github/workflows/ci-shared-manifest-labs.yml')[0];
  const gate=app.split('  checks:\n')[1]||'';
  if(!classify.includes('      owned_b1b3c_fd: $'+'{{ steps.scope.outputs.owned_b1b3c_fd }}') ||
      !classify.includes('node .github/scripts/ci-shared-manifest-route.cjs --base="$BASE_SHA" --head=HEAD >> "$GITHUB_OUTPUT"') ||
      !requiredIf.includes("needs.classify.outputs.owned_b1b3c_fd == 'true'") ||
      !required.includes('      owned_b1b3c_fd: $'+"{{ needs.classify.outputs.owned_b1b3c_fd == 'true' }}") ||
      !required.includes('uses: ./.github/workflows/ci-shared-manifest-labs.yml') ||
      !gate.includes('    name: Main PR gate') ||
      !gate.includes('    if: $'+"{{ always() && github.event_name == 'pull_request' }}") ||
      !gate.includes('    needs: [classify, required-tests, native-frames, shared-labs]') ||
      !gate.includes("needs.classify.outputs.owned_b1b3c_fd == 'true'") ||
      !gate.includes('          ROUTED_SHARED_RESULT: $'+'{{ needs.shared-labs.result }}') ||
      !gate.includes('if [ "$SHARED_RISKY" = true ]; then') ||
      !gate.includes('if [ "$ROUTED_SHARED_RESULT" != success ]; then'))return false;
  if(!shared.includes('      owned_b1b3c_fd:\n        required: true\n        type: boolean') ||
      !shared.includes('  route-b1b3c-fd:\n    name: Impacted shared manifest / b1b3c-fd\n    if: $'+'{{ inputs.core || inputs.lib || inputs.owned_b1b3c_fd }}\n    uses: ./.github/workflows/core-server-bootstrap-b1b3c-signed-fd.yml') ||
      !/^    needs: \[[^\n]*route-b1b3c-fd[^\n]*\]$/m.test(shared) ||
      !shared.includes('"route-b1b3c-fd",["core","lib","owned_b1b3c_fd"]'))return false;
  return true;
}

function git(cwd, ...args) {
  return cp.execFileSync('git', args, { cwd, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
}
function changedWorkflows(base, head = 'HEAD', cwd = process.cwd()) {
  for (const ref of [base, head]) {
    if (typeof ref !== 'string' || !/^[\w./-]+$/.test(ref) || ref.startsWith('-')) {
      throw Error('Untrusted Git revision');
    }
  }
  const raw = git(cwd, 'diff', '--name-status', '-z', '--no-renames', base, head, '--', '.github/workflows/');
  const fields = raw.split('\0').filter(Boolean);
  if (fields.length % 2 !== 0) throw Error('Unexpected git diff name-status output');
  const records = [];
  for (let i = 0; i < fields.length; i += 2) {
    if (!/^[AMD]$/.test(fields[i])) continue;
    const path = fields[i + 1];
    if (WORKFLOW.test(path)) records.push({ status: fields[i], path });
  }
  return records;
}

function audit(base, head = 'HEAD', cwd = process.cwd()) {
  const records = changedWorkflows(base, head, cwd);
  const errors = [];
  let newPr = 0;
  let protectedModified = 0;
  for (const record of records) {
    if (record.status === 'D') {
      // A deleted or renamed protected PR workflow is as dangerous as
      // removing its trigger or cancellation from an existing file.
      const before = git(cwd, 'show', base + ':' + record.path);
      if (prTrigger(before) && topBlock(before, 'concurrency').length > 0) {
        protectedModified++;
        errors.push(record.path + ': deleted previously protected PR workflow');
      }
      continue;
    }
    const text = git(cwd, 'show', head + ':' + record.path);
    if (record.status === 'A') {
      if (prTrigger(text)) newPr++;
      errors.push(...policyErrors(text, record.path));
    } else {
      const before = git(cwd, 'show', base + ':' + record.path);
      // Modifying an existing workflow must not remove previously enforced
      // concurrency. We do not retroactively change legacy CI implementations.
      if (prTrigger(before) && topBlock(before, 'concurrency').length > 0) {
        protectedModified++;
        if (!prTrigger(text)) {
          // Permit only byte-exact owner moves backed by the mandatory route.
          // Every other removed protected trigger still fails closed.
          const read = path => git(cwd, 'show', head + ':' + path);
          const routed = (record.path === '.github/workflows/windows-beta.yml' &&
            allowedWindowsBetaMigration(before, text, read)) ||
            (record.path === '.github/workflows/linux-beta.yml' &&
            allowedLinuxBetaMigration(before, text, read)) ||
            (record.path === '.github/workflows/text-delta-codec-isolated.yml' &&
            allowedTextDeltaMigration(before, text, read)) ||
            (record.path === '.github/workflows/core-server-bootstrap-b1b2b-ubuntu.yml' &&
            allowedB1b2bMigration(before, text, read)) ||
            (record.path === '.github/workflows/core-server-bootstrap-b1b3c-signed-fd.yml' &&
            allowedB1b3cMigration(before, text, read));
          if (!routed) errors.push(record.path + ': removed existing pull_request trigger');
        } else {
          const newer = topBlock(text, 'concurrency');
          if (newer.length === 0) {
            errors.push(record.path + ': removed existing PR concurrency');
          } else {
            const prior = topBlock(before, 'concurrency');
            const priorCancel = prior.find(line => /^  cancel-in-progress:/.test(line));
            const currentCancel = newer.find(line => /^  cancel-in-progress:/.test(line));
            if (priorCancel && (!currentCancel || /^  cancel-in-progress:\s*false(?:\s|$)/.test(currentCancel))) {
              errors.push(record.path + ': removed existing PR cancellation');
            }
          }
        }
      }
    }
  }
  return { records: records.length, newPr, protectedModified, errors };
}

function main(argv) {
  const args = Object.fromEntries(argv.map(arg => {
    const match = arg.match(/^--(base|head)=([^\s]+)$/);
    if (!match) throw Error('Expected --base=SHA and --head=SHA');
    return [match[1], match[2]];
  }));
  if (!args.base) throw Error('--base required');
  const result = audit(args.base, args.head || 'HEAD');
  console.log('CI_PR_CONCURRENCY_AUDIT inspected=' + result.records +
    ' new_pr=' + result.newPr + ' protected_modified=' + result.protectedModified);
  if (result.errors.length > 0) {
    for (const error of result.errors) console.error('::error::' + error);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  try { main(process.argv.slice(2)); }
  catch (error) {
    console.error('CI queue policy could not verify workflows: ' + error.message);
    process.exitCode = 1;
  }
}

module.exports = { topBlock, prEvents, prTrigger, policyErrors, allowedWindowsBetaMigration, allowedLinuxBetaMigration, allowedTextDeltaMigration, allowedB1b2bMigration, allowedB1b3cMigration, changedWorkflows, audit, main };
