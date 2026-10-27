'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {topBlock, prTrigger, policyErrors, allowedWindowsBetaMigration, allowedLinuxBetaMigration, allowedTextDeltaMigration, changedWorkflows, audit} = require('./ci-pr-concurrency.cjs');
const group = '  group: $' + '{{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}';
const cancel = "  cancel-in-progress: $" + "{{ github.event_name == 'pull_request' }}";
const good = ['name: Optional lab','on:','  pull_request:','    paths:',"      - 'modules/test/**'",'  workflow_dispatch:','concurrency:',group,cancel,'jobs:','  test:','    runs-on: ubuntu-latest','    steps:','      - run: true'].join('\n')+'\n';

test('good scoped cancellation, including quoted on keys',()=>{
  assert.ok(prTrigger(good));
  assert.deepEqual(policyErrors(good,'test.yml'),[]);
  assert.equal(topBlock(good,'concurrency').length,3);
  for(const key of ['"on":',"'on':"]) assert.deepEqual(policyErrors(good.replace('on:',key),'test.yml'),[]);
});

test('nonstandard YAML event indentation is guarded without mistaking nested inputs',()=>{
  const indented=good.replace(
    "  pull_request:\n    paths:\n      - 'modules/test/**'\n  workflow_dispatch:",
    "    pull_request:\n      paths:\n        - 'modules/test/**'\n    workflow_dispatch:"
  );
  assert.equal(prTrigger(indented),true,'four-space PR event cannot evade the audit');
  assert.deepEqual(policyErrors(indented,'indented.yml'),[]);
  const manual="on:\n  workflow_dispatch:\n    inputs:\n      pull_request:\n        description: nested form input\njobs:\n  example:\n    runs-on: ubuntu-latest\n";
  assert.equal(prTrigger(manual),false,'nested manual input is not a GitHub event');
});

test('reject cross-workflow, cross-PR and missing cancellation protection',()=>{
  for(const replacement of ['  group: shared', '  group: $'+'{{ github.ref }}', '  group: $'+'{{ github.event.pull_request.number || github.ref }}']){
    assert.match(policyErrors(good.replace(group,replacement),'lab.yml').join(','),/group must isolate/);
  }
  for(const replacement of ['  cancel-in-progress: true','  cancel-in-progress: false', '  cancel-in-progress: $'+'{{ github.event_name == "push" }}']){
    assert.match(policyErrors(good.replace(cancel,replacement),'lab.yml').join(','),/PR triggers only/);
  }
  assert.match(policyErrors(good.replace('concurrency:\n'+group+'\n'+cancel,''),'lab.yml').join(','),/missing workflow-level/);
  assert.match(policyErrors(good.replace(cancel,cancel+'\n  queue: max'),'lab.yml').join(','),/queue: max/);
});

test('pull_request_target requires its own cancellation condition',()=>{
  const targeted=good.replace('  pull_request:', '  pull_request_target:');
  assert.ok(prTrigger(targeted));
  assert.match(policyErrors(targeted,'target.yml').join(','),/PR triggers only/);
  const targetCancel="  cancel-in-progress: $"+ "{{ github.event_name == 'pull_request_target' }}";
  assert.deepEqual(policyErrors(targeted.replace(cancel,targetCancel),'target.yml'),[]);
});

test('comments cannot masquerade as an Actions expression',()=>{
  const bad=good.replace(group, '  group: shared # github.workflow github.event.pull_request.number github.ref');
  assert.match(policyErrors(bad,'comment.yml').join(','),/group must isolate/);
});

test('manual or reusable workflows do not require PR cancellation',()=>{
  for(const trig of ['on:\n  workflow_dispatch:', 'on:\n  workflow_call:', 'on: [push, workflow_dispatch]']){
    const manual=good.replace("on:\n  pull_request:\n    paths:\n      - 'modules/test/**'\n  workflow_dispatch:",trig);
    assert.equal(prTrigger(manual),false);
    assert.deepEqual(policyErrors(manual,'manual.yml'),[]);
  }
});

test('new PR workflows are guarded, and previously protected edited ones cannot remove guard',()=>{
  const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'akula-queue-'));
  const git=(...args)=>cp.execFileSync('git',args,{cwd,encoding:'utf8'}).trim();
  const write=(name,s)=>fs.writeFileSync(path.join(cwd,'.github/workflows',name),s);
  try {
    git('init','-q'); git('config','user.name','CI'); git('config','user.email','ci@example.invalid');
    fs.mkdirSync(path.join(cwd,'.github/workflows'),{recursive:true});
    write('old.yml',good); git('add','.'); git('commit','-qm','base');
    const base=git('rev-parse','HEAD');
    write('added.yml',good.replace(cancel,'  cancel-in-progress: true'));
    git('add','.');git('commit','-qm','bad new workflow');
    assert.deepEqual(changedWorkflows(base,'HEAD',cwd),[{status:'A',path:'.github/workflows/added.yml'}]);
    assert.match(audit(base,'HEAD',cwd).errors.join(','),/PR triggers only/);
    write('added.yml',good); git('add','.');git('commit','-qm','fix new workflow');
    assert.deepEqual(audit(base,'HEAD',cwd).errors,[]);
    write('old.yml',good.replace('concurrency:\n'+group+'\n'+cancel+'\n',''));
    git('add','.');git('commit','-qm','remove old protection');
    assert.match(audit(base,'HEAD',cwd).errors.join(','),/removed existing PR concurrency/);
    write('old.yml',good.replace(cancel,'  cancel-in-progress: false'));
    git('add','.');git('commit','-qm','disable existing cancellation');
    assert.match(audit(base,'HEAD',cwd).errors.join(','),/removed existing PR cancellation/);
    fs.unlinkSync(path.join(cwd,'.github/workflows/old.yml'));
    git('add','-A');git('commit','-qm','remove formerly protected PR workflow');
    assert.deepEqual(
      changedWorkflows(base,'HEAD',cwd).filter(record => record.status === 'D'),
      [{status:'D',path:'.github/workflows/old.yml'}]
    );
    assert.match(audit(base,'HEAD',cwd).errors.join(','),/deleted previously protected PR workflow/);
  } finally {fs.rmSync(cwd,{recursive:true,force:true});}
});


test('only fully routed Windows beta may lose a protected PR trigger',()=>{
  const paths=[
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
  const expression=s=>String.fromCharCode(36,123,123)+' '+s+' '+String.fromCharCode(125,125);
  const oldHeader='on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n'
    +paths.map(p=>"      - '"+p+"'\n").join('')+'\n  workflow_dispatch:\n';
  const newHeader='on:\n  workflow_call:\n'
    +'  # Source ownership is enforced by the required Main PR gate.\n'
    +'  workflow_dispatch:\n';
  const old=oldHeader+'    inputs:\n      ref:\n        required: true\n        default: main\njobs:\n  windows-beta:\n    runs-on: windows-latest\n';
  const candidate=old.replace(oldHeader,newHeader);
  const route='const OWNED_SUITES=Object.freeze({\n  "owned_windows_beta": '
    +JSON.stringify(paths,null,2).replace(/\n/g,'\n  ')
    +'\n});\nfunction ownedRoutes(paths){\n  const changed=new Set(paths);\n'
    +'  return {};\n}\nconst flags={...ownedRoutes(paths)};\n'
    +'module.exports={OWNED_SUITES,ownedRoutes,flags};\n';
  const app=[
    '  classify:',
    '      owned_windows_beta: '+expression('steps.scope.outputs.owned_windows_beta'),
    '      node .github/scripts/ci-shared-manifest-route.cjs --base="$BASE_SHA" --head=HEAD >> "$GITHUB_OUTPUT"',
    '  required-tests:',
    '  shared-labs:',
    '    if: '+expression("needs.classify.outputs.owned_windows_beta == 'true'"),
    '    uses: ./.github/workflows/ci-shared-manifest-labs.yml',
    '      owned_windows_beta: '+expression("needs.classify.outputs.owned_windows_beta == 'true'"),
    '  checks:',
    '    name: Main PR gate',
    '    if: '+expression("always() && github.event_name == 'pull_request'"),
    '    needs: [classify, required-tests, native-frames, shared-labs]',
    '          SHARED_RISKY: '+expression("needs.classify.outputs.owned_windows_beta == 'true'"),
    '          ROUTED_SHARED_RESULT: '+expression('needs.shared-labs.result'),
    '          if [ "$SHARED_RISKY" = true ]; then',
    '            if [ "$ROUTED_SHARED_RESULT" != success ]; then',
  ].join('\n')+'\n';
  const shared=[
    '      owned_windows_beta:',
    '        required: true',
    '        type: boolean',
    '  route-windows-beta:',
    '    name: Impacted shared manifest / windows-beta',
    '    if: '+expression('inputs.lock || inputs.owned_windows_beta'),
    '    uses: ./.github/workflows/windows-beta.yml',
    '    needs: [route-other, route-windows-beta]',
    '          const cases=[["route-windows-beta",["lock","owned_windows_beta"]]];',
  ].join('\n')+'\n';
  const fixture={
    '.github/scripts/ci-shared-manifest-route.cjs':route,
    '.github/workflows/app-checks.yml':app,
    '.github/workflows/ci-shared-manifest-labs.yml':shared,
  };
  const read=path=>fixture[path];
  assert.equal(allowedWindowsBetaMigration(old,candidate,read),true,'exactly migrated route');
  const bad=[
    ['missing source owner','.github/scripts/ci-shared-manifest-route.cjs',route.replace('    "scripts/test-windows-beta.ps1",\n','')],
    ['missing routing condition','.github/workflows/ci-shared-manifest-labs.yml',shared.replace('inputs.lock || inputs.owned_windows_beta','inputs.lock')],
    ['missing boundary dependency','.github/workflows/ci-shared-manifest-labs.yml',shared.replace('route-other, route-windows-beta','route-other')],
    ['missing boundary flag','.github/workflows/ci-shared-manifest-labs.yml',shared.replace('["lock","owned_windows_beta"]','["lock"]')],
    ['missing Main gate risk','.github/workflows/app-checks.yml',app.replace("SHARED_RISKY: "+expression("needs.classify.outputs.owned_windows_beta == 'true'"),"SHARED_RISKY: "+expression('false'))],
    ['missing Main gate dependency','.github/workflows/app-checks.yml',app.replace('needs: [classify, required-tests, native-frames, shared-labs]','needs: [classify, required-tests, native-frames]')],
    ['missing fail-closed check','.github/workflows/app-checks.yml',app.replace('if [ "$ROUTED_SHARED_RESULT" != success ]; then','echo passing')],
  ];
  for(const [name,path,content] of bad){
    assert.equal(allowedWindowsBetaMigration(old,candidate,key=>key===path?content:read(key)),false,name);
  }
  assert.equal(allowedWindowsBetaMigration(old,candidate.replace('runs-on: windows-latest','runs-on: ubuntu-latest'),read),false,'installer body must not change');
  assert.equal(allowedWindowsBetaMigration(old,candidate.replace('workflow_dispatch:','push:'),read),false,'manual trigger must stay');
  assert.equal(allowedWindowsBetaMigration(old.replace("      - 'scripts/test-windows-beta.ps1'\n",''),candidate,read),false,'original paths must be exact');
  assert.equal(allowedWindowsBetaMigration(old,candidate,()=>{throw Error('missing route');}),false,'missing evidence fails closed');
});

test('malformed git revision fails closed',()=>{
  assert.throws(()=>changedWorkflows('','HEAD'),/Untrusted Git/);
  assert.throws(()=>changedWorkflows('--bad','HEAD'),/Untrusted Git/);
  assert.throws(()=>changedWorkflows('not-a-valid-ref','HEAD'));
});


test('cancellation expression cannot widen or disable PR-only coalescing',()=>{
  for(const expr of [
    "github.event_name == 'pull_request' || true",
    "true || github.event_name == 'pull_request'",
    "github.event_name == 'pull_request' && false",
    "github.event_name == 'pull_request' || github.event_name != 'schedule'",
    "github.event_name == 'pull_request' || github.event_name == 'workflow_dispatch'",
  ]){
    const candidate=good.replace(cancel,'  cancel-in-progress: $'+'{{ '+expr+' }}');
    assert.match(policyErrors(candidate,'unsafe.yml').join(','),/PR triggers only/,expr);
  }
  const both=good.replace('  workflow_dispatch:','  pull_request_target:\n  workflow_dispatch:');
  const combined='  cancel-in-progress: $'+"{{ github.event_name == 'pull_request' || github.event_name == 'pull_request_target' }}";
  assert.deepEqual(policyErrors(both.replace(cancel,combined),'both.yml'),[]);
  assert.match(policyErrors(both,'both.yml').join(','),/PR triggers only/);
});


test('only byte-exact Linux beta migration may remove a protected PR trigger',()=>{
  const owned=[
    '.github/workflows/linux-beta.yml',
    '.github/scripts/linux-beta-identity.test.cjs',
    'scripts/linux-beta-smoke.sh',
    'aquilum-app/native/packaging/akula-linux-nfpm.yaml',
    'aquilum-app/native/packaging/Akula.desktop',
    'aquilum-app/native/Cargo.toml',
  ];
  const newHeader='on:\n  workflow_call:\n'
    +'  # PR source owners are enforced by the required Main PR gate.\n'
    +'\npermissions:\n';
  const oldHeader='on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n'
    +owned.map(p=>"      - '"+p+"'\n").join('')
    +'\npermissions:\n';
  const candidate=fs.readFileSync(path.join(__dirname,'../workflows/linux-beta.yml'),'utf8');
  assert.equal(candidate.split(newHeader).length,2,'new Linux trigger is exact');
  assert.doesNotMatch(candidate,/^  pull_request:/m);
  const old=candidate.replace(newHeader,oldHeader);
  const sources=[
    '.github/scripts/ci-shared-manifest-route.cjs',
    '.github/workflows/app-checks.yml',
    '.github/workflows/ci-shared-manifest-labs.yml'
  ];
  const contents=Object.fromEntries(sources.map(file=>[file,
    fs.readFileSync(path.join(__dirname,'../..',file),'utf8')]));
  const read=file=>contents[file];
  assert.equal(allowedLinuxBetaMigration(old,candidate,read),true,
    'six exact owners and required gate prove a safe Linux move');
  const route=contents[sources[0]], app=contents[sources[1]], shared=contents[sources[2]];
  const invalid=[
    ['missing owner',sources[0],route.replace('    "scripts/linux-beta-smoke.sh",\n','')],
    ['missing owned helper',sources[0],route.replace('...ownedRoutes(paths)','false')],
    ['missing called route',sources[2],shared.replace('inputs.lock || inputs.owned_linux_beta','inputs.lock')],
    ['missing boundary flag',sources[2],shared.replace('"route-linux-beta",["lock","owned_linux_beta"]','"route-linux-beta",["lock"]')],
    ['missing route dependency',sources[2],shared.replace(', route-linux-beta,',', ')],
    ['missing Main gate dependency',sources[1],app.replace(
      'needs: [classify, required-tests, native-frames, shared-labs]',
      'needs: [classify, required-tests, native-frames]')],
    ['missing Main gate risk',sources[1],app.replace(
      'if [ "$SHARED_RISKY" = true ]; then','echo unsafe')],
    ['missing output',sources[1],app.replace(
      'owned_linux_beta: $'+'{{ steps.scope.outputs.owned_linux_beta }}',
      'owned_linux_beta: false')],
  ];
  for(const [label,key,modified] of invalid){
    assert.notEqual(modified,contents[key],label+' fixture must change');
    assert.equal(allowedLinuxBetaMigration(old,candidate,file=>file===key?modified:read(file)),
      false,label+' must fail closed');
  }
  assert.equal(allowedLinuxBetaMigration(old,candidate.replace('runs-on: ubuntu-22.04','runs-on: ubuntu-24.04'),read),
    false,'installer body changes are forbidden');
  assert.equal(allowedLinuxBetaMigration(old,candidate.replace('  workflow_call:\n','  workflow_call:\n  workflow_dispatch:\n'),read),
    false,'new manual release path is forbidden');
  assert.equal(allowedLinuxBetaMigration(old.replace('branches: [main]','branches: [develop]'),candidate,read),
    false,'target branches must remain exact');
  assert.equal(allowedLinuxBetaMigration(old.replace("      - 'scripts/linux-beta-smoke.sh'\n",''),candidate,read),
    false,'missing previous source must not lose its PR test');
  assert.equal(allowedLinuxBetaMigration(old,candidate,()=>{throw Error('no evidence');}),
    false,'missing evidence always fails closed');
});



test('B1b2b protected PR trigger migration requires exact old owners and fail-closed gate',()=>{
  const {allowedB1b2bMigration}=require('./ci-pr-concurrency.cjs');
  const name='.github/workflows/core-server-bootstrap-b1b2b-ubuntu.yml';
  const read=p=>fs.readFileSync(path.join(__dirname,'../..',p),'utf8');
  const after=read(name);
  const oldHeader="on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n      - \"aquilum-app/core/examples/akula_b1b2b_release_fixture.rs\"\n      - \"scripts/ci/akula-b1b2b-ubuntu-vm.sh\"\n      - \"scripts/ci/akula-b1b2b-guest-lab.sh\"\n      - \"scripts/ci/akula-b1b2b-node-wrapper.sh\"\n      - \"docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B2B.md\"\n      - \"docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B2B1.md\"\n      - \"docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B2B2.md\"\n      - \".github/workflows/core-server-bootstrap-b1b2b-ubuntu.yml\"\n\n      - \"aquilum-app/core/src/server_release_lab.rs\"\n      - \"aquilum-app/core/src/bin/akula_node_lab.rs\"\n  workflow_dispatch:\n";
  const newHeader="on:\n  workflow_call:\n  workflow_dispatch:\n";
  assert.equal(after.split(newHeader).length,2);
  const before=after.replace(newHeader,oldHeader);
  assert.equal(allowedB1b2bMigration(before,after,read),true);
  assert.equal(allowedB1b2bMigration(before,after.replace('workflow_dispatch:','push:'),read),false);
  assert.equal(allowedB1b2bMigration(before,after.replace('cargo build --locked','cargo build --offline'),read),false);
  assert.equal(allowedB1b2bMigration(before.replace('branches: [main]','branches: [develop]'),after,read),false);
  assert.equal(allowedB1b2bMigration(before.replace('scripts/ci/akula-b1b2b-guest-lab.sh','scripts/ci/wrong-guest.sh'),after,read),false);
  const route='.github/scripts/ci-shared-manifest-route.cjs';
  const app='.github/workflows/app-checks.yml';
  const shared='.github/workflows/ci-shared-manifest-labs.yml';
  const altered=[
    [route,read(route).replace('    "scripts/ci/akula-b1b2b-guest-lab.sh",\n','')],
    [app,read(app).replace("needs.classify.outputs.owned_b1b2b_vm == 'true'","false")],
    [app,read(app).replace('needs: [classify, required-tests, native-frames, shared-labs]','needs: [classify, required-tests, native-frames]')],
    [app,read(app).replace('if [ "$ROUTED_SHARED_RESULT" != success ]; then','echo bypass')],
    [shared,read(shared).replace('inputs.core || inputs.owned_b1b2b_vm','inputs.core')],
    [shared,read(shared).replace('["core","owned_b1b2b_vm"]','["core"]')],
    [shared,read(shared).replace('route-b0c-ssh-native, route-b1b2b-ubuntu','route-b0c-ssh-native')],
  ];
  for(const [key,modified] of altered){
    assert.notEqual(read(key),modified,'negative mutation must change fixture: '+key);
    assert.equal(allowedB1b2bMigration(before,after,p=>p===key?modified:read(p)),false,'must fail closed for '+key);
  }
  assert.equal(allowedB1b2bMigration(before,after,()=>{throw Error('no evidence');}),false);
});


test('B1b3c protected signed-FD PR-trigger removal is byte-exact and fail-closed',()=>{
  const {allowedB1b3cMigration}=require('./ci-pr-concurrency.cjs');
  const filename='.github/workflows/core-server-bootstrap-b1b3c-signed-fd.yml';
  const read=p=>fs.readFileSync(path.join(__dirname,'../..',p),'utf8');
  const after=read(filename);
  const oldHeader="on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n\n      - \"aquilum-app/core/src/server_installer_pinned_release_lab.rs\"\n      - \"aquilum-app/core/tests/server_bootstrap_b1b3c.rs\"\n      - \"aquilum-app/core/src/bin/akula_helper_b1b3b_lab.rs\"\n      - \"aquilum-app/core/examples/akula_b1b3c_fixture.rs\"\n      - \"scripts/ci/akula-b1b3c-root-helper-lab.sh\"\n      - \".github/workflows/core-server-bootstrap-b1b3c-signed-fd.yml\"\n      - \"docs/platforms/AKULA-SERVER-BOOTSTRAP-B1B3C.md\"\n  workflow_dispatch:\n";
  const newHeader="on:\n  workflow_call:\n  workflow_dispatch:\n";
  assert.equal(after.split(newHeader).length,2);
  const before=after.replace(newHeader,oldHeader);
  assert.equal(allowedB1b3cMigration(before,after,read),true,'exact owner route');
  assert.equal(allowedB1b3cMigration(before,after.replace('workflow_dispatch:','push:'),read),false,'manual remains');
  assert.equal(allowedB1b3cMigration(before,after.replace('cargo build --locked','cargo build --offline'),read),false,'signed FD build unchanged');
  assert.equal(allowedB1b3cMigration(before.replace('branches: [main]','branches: [develop]'),after,read),false,'main only');
  assert.equal(allowedB1b3cMigration(before.replace('scripts/ci/akula-b1b3c-root-helper-lab.sh','scripts/ci/other.sh'),after,read),false,'old paths immutable');
  const route='.github/scripts/ci-shared-manifest-route.cjs';
  const app='.github/workflows/app-checks.yml';
  const shared='.github/workflows/ci-shared-manifest-labs.yml';
  const mutants=[
    [route,read(route).replace('    "scripts/ci/akula-b1b3c-root-helper-lab.sh",\n','')],
    [app,read(app).replace("needs.classify.outputs.owned_b1b3c_fd == 'true'","false")],
    [app,read(app).replace('needs: [classify, required-tests, native-frames, shared-labs]','needs: [classify, required-tests, native-frames]')],
    [app,read(app).replace('if [ "$ROUTED_SHARED_RESULT" != success ]; then','echo bypass')],
    [shared,read(shared).replace('inputs.core || inputs.lib || inputs.owned_b1b3c_fd','inputs.core || inputs.lib')],
    [shared,read(shared).replace('["core","lib","owned_b1b3c_fd"]','["core","lib"]')],
    [shared,read(shared).replace('route-b1b3c-fd, route-b1b4a-cas','route-b1b4a-cas')],
  ];
  for(const [file,changed] of mutants){
    assert.notEqual(read(file),changed,'mutation must modify fixture');
    assert.equal(allowedB1b3cMigration(before,after,p=>p===file?changed:read(p)),false,file);
  }
  assert.equal(allowedB1b3cMigration(before,after,()=>{throw Error('missing route');}),false);
});


test('B1c0 protected PR trigger requires exact root-CAS route without bypass',()=>{
  const {allowedB1c0Migration}=require('./ci-pr-concurrency.cjs');
  const wf='.github/workflows/core-server-bootstrap-b1c0-fixed-node.yml';
  const read=p=>fs.readFileSync(path.join(__dirname,'../..',p),'utf8');
  const after=read(wf);
  const oldHeader="on:\n  workflow_call:\n  pull_request:\n    branches: [main]\n    paths:\n\n      - \"aquilum-app/core/src/server_installer_pinned_release_lab.rs\"\n      - \"aquilum-app/core/src/server_installer_fixed_node_lab.rs\"\n      - \"aquilum-app/core/tests/server_bootstrap_b1c0.rs\"\n      - \".github/workflows/core-server-bootstrap-b1c0-fixed-node.yml\"\n      - \"docs/platforms/AKULA-SERVER-BOOTSTRAP-B1C0.md\"\n  workflow_dispatch:\n";
  const newHeader="on:\n  workflow_call:\n  workflow_dispatch:\n";
  assert.equal(after.split(newHeader).length,2);
  const before=after.replace(newHeader,oldHeader);
  assert.equal(allowedB1c0Migration(before,after,read),true,'strict mandatory B1c0 owner route');
  assert.equal(allowedB1c0Migration(before,after.replace('workflow_dispatch:','push:'),read),false);
  assert.equal(allowedB1c0Migration(before,after.replace('cargo test --locked','cargo test --offline'),read),false);
  assert.equal(allowedB1c0Migration(before.replace('branches: [main]','branches: [other]'),after,read),false);
  assert.equal(allowedB1c0Migration(before.replace('docs/platforms/AKULA-SERVER-BOOTSTRAP-B1C0.md','docs/platforms/WRONG.md'),after,read),false);
  const route='.github/scripts/ci-shared-manifest-route.cjs';
  const app='.github/workflows/app-checks.yml';
  const shared='.github/workflows/ci-shared-manifest-labs.yml';
  const corrupted=[
    [route,read(route).replace('    "aquilum-app/core/src/server_installer_pinned_release_lab.rs",\n','')],
    [app,read(app).replace("needs.classify.outputs.owned_b1c0_node == 'true'","false")],
    [app,read(app).replace('needs: [classify, required-tests, native-frames, shared-labs]','needs: [classify, required-tests, native-frames]')],
    [app,read(app).replace('if [ "$ROUTED_SHARED_RESULT" != success ]; then','echo bypass')],
    [shared,read(shared).replace('inputs.core || inputs.lib || inputs.owned_b1c0_node','inputs.core || inputs.lib')],
    [shared,read(shared).replace('["core","lib","owned_b1c0_node"]','["core","lib"]')],
    [shared,read(shared).replace('route-b1c0-node, route-b1d0-opt','route-b1d0-opt')],
  ];
  for(const [source,mutated] of corrupted){
    assert.notEqual(read(source),mutated,'negative mutation must modify '+source);
    assert.equal(allowedB1c0Migration(before,after,p=>p===source?mutated:read(p)),false,source);
  }
  assert.equal(allowedB1c0Migration(before,after,()=>{throw Error('missing route');}),false);
});

test('Text Delta PR owner may migrate only to a proven required Main gate',()=>{
  const name='.github/workflows/text-delta-codec-isolated.yml';
  const read=p=>fs.readFileSync(path.join(__dirname,'../..',p),'utf8');
  const after=read(name);
  const replacement="on:\n"
    +"  # PR source owners run in the required Application checks / Main PR gate.\n"
    +"  workflow_dispatch:\n";
  const original="on:\n  pull_request:\n    paths:\n"
    +"      - 'modules/text-delta-codec/**'\n"
    +"      - '.github/workflows/text-delta-codec-isolated.yml'\n"
    +"  workflow_dispatch:\n";
  assert.equal(after.split(replacement).length,2,'exact migration header');
  const before=after.replace(replacement,original);
  assert.equal(allowedTextDeltaMigration(before,after,read),true,'fully protected route');
  assert.equal(allowedTextDeltaMigration(before,after.replace('workflow_dispatch:','push:'),read),false);
  assert.equal(allowedTextDeltaMigration(before.replace('modules/text-delta-codec/**','modules/other/**'),after,read),false);
  assert.equal(allowedTextDeltaMigration(before,after,()=>{throw Error('missing route');}),false);
  const appPath='.github/workflows/app-checks.yml';
  const app=read(appPath);
  for(const [label,from,to] of [
    ['missing Main dependency','needs: [classify, required-tests, native-frames, shared-labs]','needs: [classify, native-frames, shared-labs]'],
    ['missing Cargo step','cargo test --manifest-path modules/text-delta-codec/Cargo.toml -- --nocapture','echo bypass'],
    ['missing conditional',"if: needs.classify.outputs.text_delta_module_required == 'true'",'if: false']
  ]){
    assert.ok(app.includes(from),label+' fixture');
    assert.equal(allowedTextDeltaMigration(before,after,p=>p===appPath?app.replace(from,to):read(p)),false,label);
  }
});
