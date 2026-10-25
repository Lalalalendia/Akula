'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const cp=require('node:child_process');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {classify,changed,safeManifestOnly,updaterSecurityAffected}=require('./ci-rust-scope.cjs');

test('pure documentation and CI telemetry do not compile Rust',()=>{
  assert.equal(classify(['README.md','docs/engineering-budgets.md',
    '.github/scripts/ci-run-budget.cjs']), 'none');
});

test('CI-only workflow and routing script changes do not compile unrelated Rust',()=>{
  for(const onlyCi of [
    ['.github/workflows/app-checks.yml'],
    ['.github/workflows/app-checks.yml','.github/scripts/ci-rust-scope.cjs'],
    ['.github/workflows/ci-shared-manifest-labs.yml','.github/scripts/ci-shared-routing-contract.test.cjs'],
    ['.github/workflows/app-checks.yml','docs/ci.md']
  ]) assert.equal(classify(onlyCi),'none',onlyCi.join(','));
  // Cargo/Core changes must still get the full dependency graph even alongside CI changes.
  for(const rust of ['aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock',
    'aquilum-app/core/src/files/gate/write.rs']){
    assert.equal(classify([rust,'.github/workflows/app-checks.yml']),'workspace',rust);
  }
  assert.equal(classify(['aquilum-app/native/src/main.rs',
    '.github/workflows/app-checks.yml']),'native');
  assert.equal(classify(['.github/workflows/app-checks.yml',
    'aquilum-app/core/src/sync/guarded_note_write.rs']),'sync');
});
test('Native source-only PRs run native package tests, never re-run Core unit suites',()=>{
  for(const paths of [
    ['aquilum-app/native/src/main.rs'],
    ['aquilum-app/native/src/app/tests/graph.rs','README.md'],
    ['aquilum-app/native/tests/desktop_integration.rs'],
    ['aquilum-app/native/src/updater/macos_transaction.rs',
     '.github/workflows/app-checks.yml'],
    ['aquilum-app/native/src/ui/editor.rs','aquilum-app/native/src/app.rs'],
  ]) assert.equal(classify(paths),'native',paths.join(','));
  for(const risky of [
    'aquilum-app/native/Cargo.toml',
    'aquilum-app/Cargo.lock',
    'aquilum-app/Cargo.toml',
    'aquilum-app/native/build.rs',
    'aquilum-app/native/vendor/krilla/src/lib.rs',
    'aquilum-app/core/src/lib.rs',
    'aquilum-app/core/src/sync/journal.rs',
  ]) assert.equal(classify(['aquilum-app/native/src/main.rs',risky]),'workspace',risky);
});

test('only sync implementation and its contract tests use focused lane',()=>{
  assert.equal(classify(['aquilum-app/core/src/sync/pinned_windows.rs',
    'aquilum-app/core/src/sync/journal.rs',
    'aquilum-app/core/tests/sync_contract.rs','docs/adr.md']), 'sync');
});
test('any native, core outside sync, Cargo config or gate change requires all tests',()=>{
  for(const name of [
    'aquilum-app/core/src/documents/hub.rs',
    'aquilum-app/native/src/ui/editor.rs',
    'aquilum-app/Cargo.lock',
    'aquilum-app/core/Cargo.toml',
    'aquilum-app/native/vendor/ratex-parser/src/parser.rs'
  ]) assert.equal(classify([name,'aquilum-app/core/src/sync/identity.rs']),'workspace',name);
});
test('deleted/renamed files also select the appropriate scope using real Git',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'akula-ci-scope-'));
  const git=(...args)=>cp.execFileSync('git',args,{cwd:dir,encoding:'utf8'}).trim();
  try {
    git('init','-q'); git('config','user.name','CI');
    git('config','user.email','ci@example.invalid');
    const from=path.join(dir,'aquilum-app/core/src/sync/moved.rs');
    fs.mkdirSync(path.dirname(from),{recursive:true});
    fs.writeFileSync(from,'pub fn original() {}');
    git('add','.');git('commit','-qm','base');
    const base=git('rev-parse','HEAD');
    const to=path.join(dir,'aquilum-app/core/src/sync/renamed.rs');
    fs.renameSync(from,to);
    git('add','-A');git('commit','-qm','rename');
    assert.equal(classify(changed(base,'HEAD',dir)),'sync');
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
test('missing Git refs do not silently skip mandatory checks',()=>{
  assert.throws(()=>changed('not-a-git-sha','HEAD'));
});

const {assessModuleImpact}=require('./ci-module-impact.cjs');

test('B0 scope candidate is advisory, never a reason to skip required checks',()=>{
  for(const path of ['aquilum-app/core/src/server_bootstrap_lab.rs',
      'aquilum-app/core/tests/server_bootstrap_b0.rs']){
    const result=assessModuleImpact([path]);
    assert.equal(result.candidate,'server-bootstrap-b0');
    assert.equal(result.requiredScope,'workspace');
    assert.equal(result.maySkipRequiredChecks,false);
  }
  const workflow=assessModuleImpact(['.github/workflows/core-server-bootstrap-b0.yml']);
  assert.equal(workflow.candidate,'server-bootstrap-b0');
  assert.equal(workflow.maySkipRequiredChecks,false);
});

test('shared manifests, lock, lib, mixed and unknown files block focused B0 routing',()=>{
  for(const path of ['aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock',
      'aquilum-app/core/src/lib.rs','aquilum-app/core/src/sync/dag.rs',
      'aquilum-app/native/src/main.rs']){
    const result=assessModuleImpact(['aquilum-app/core/src/server_bootstrap_lab.rs',path]);
    assert.equal(result.candidate,null,path);
    assert.equal(result.requiredScope,'workspace');
    assert.equal(result.maySkipRequiredChecks,false);
  }
  assert.equal(assessModuleImpact(['aquilum-app/core/src/notion_api_probe.rs']).candidate,null);
  assert.equal(assessModuleImpact(['docs/unrelated.md']).requiredScope,'none');
});

test('empty/malformed input and moved B0-to-unknown file fail closed',()=>{
  for(const input of [[],null,[''],['/tmp/b0.rs'],['../escape']]){
    const result=assessModuleImpact(input);
    assert.equal(result.candidate,null);
    assert.equal(result.requiredScope,'workspace');
    assert.equal(result.maySkipRequiredChecks,false);
  }
  const renamed=assessModuleImpact([
    'aquilum-app/core/src/server_bootstrap_lab.rs',
    'aquilum-app/core/src/unowned.rs',
  ]);
  assert.equal(renamed.candidate,null);
  assert.equal(renamed.requiredScope,'workspace');
});

test('B1a/B1b0 scoped workflows retain full checks on owned code but not shared Cargo',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const workflow=name=>parse(fs.readFileSync(path.join(__dirname,'../workflows',name),'utf8'));
  const b1a=workflow('core-server-bootstrap-b1a-release.yml');
  const b1b0=workflow('core-server-bootstrap-b1b0-staging.yml');
  const shared=[
    'aquilum-app/core/Cargo.toml','aquilum-app/core/src/lib.rs',
    'aquilum-app/Cargo.lock','aquilum-app/Cargo.toml'
  ];
  shared.push('aquilum-app/core/src/server_release_lab.rs');
  for(const file of shared){
    assert.equal(matches(b1a,file),false,'shared changes use required gate: '+file);
    assert.equal(matches(b1b0,file),false,'shared changes use required gate: '+file);
  }
  for(const file of ['modules/bootstrap-b1a/src/lib.rs']){
    assert.equal(matches(b1a,file),true,'B1a owner path: '+file);
    assert.equal(matches(b1b0,file),true,'B1b0 dependent path: '+file);
  }
  assert.equal(matches(b1b0,'modules/bootstrap-b1b0/src/server_staging_lab.rs'),true);
  assert.equal(matches(b1a,'aquilum-app/core/tests/server_bootstrap_b1a.rs'),true);
  assert.equal(matches(b1b0,'aquilum-app/core/tests/server_bootstrap_b1b0.rs'),true);
  const gate=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.equal(parse(gate).t,'global','required application gate must remain unfiltered');
  assert.match(gate,/b1_shared_compat: \$\{\{ steps\.scope\.outputs\.b1_shared_compat \}\}/);
  assert.match(gate,/core\/src\/server_release_lab\.rs; then/);
  assert.match(gate,/grep -Fx 'pub use akula_server_release_b1a::\*;'/);
  assert.match(gate,/--features server-bootstrap-b1b-custody-lab/);
  assert.match(gate,/--test server_bootstrap_b1a --test server_bootstrap_b1b0/);
});

test('shared Cargo/lib changes route D1-D2 contracts to required gate without extra workflows',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const wf=name=>parse(fs.readFileSync(path.join(__dirname,'../workflows',name),'utf8'));
  const d1=wf('core-node-d1-loopback.yml');
  const d15=wf('core-node-d15-catalog.yml');
  const d2=wf('core-node-d2-scoped-read.yml');
  for(const file of ['aquilum-app/core/Cargo.toml','aquilum-app/core/src/lib.rs',
      'aquilum-app/Cargo.lock','aquilum-app/Cargo.toml']){
    for(const w of [d1,d15,d2]){
      assert.equal(matches(w,file),false,'use required gate for '+file);
    }
  }
  assert.equal(matches(d1,'aquilum-app/core/src/bin/akula_node_lab.rs'),true);
  assert.equal(matches(d2,'aquilum-app/core/src/bin/akula_node_lab.rs'),true);
  assert.equal(matches(d15,'aquilum-app/core/src/node_catalog_lab.rs'),true);
  assert.equal(matches(d2,'aquilum-app/core/src/node_catalog_lab.rs'),true);
  assert.equal(matches(d15,'modules/node-catalog/src/catalog.rs'),false);
  assert.equal(matches(d2,'modules/node-catalog/src/catalog.rs'),false);
  assert.equal(matches(d1,'modules/node-catalog/src/catalog.rs'),false);
  const isolated=fs.readFileSync(path.join(__dirname,'../workflows/node-catalog-isolated.yml'),'utf8');
  assert.equal(matches(parse(isolated),'modules/node-catalog/src/catalog.rs'),true);
  assert.match(isolated,/--features node-lab/);
  assert.match(isolated,/--test headless_node_d2/);
  const gate=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.equal(parse(gate).t,'global');
  assert.match(gate,/node_shared_compat: \$\{\{ steps\.scope\.outputs\.node_shared_compat \}\}/);
  assert.match(gate,/--features node-lab --bin akula-node-lab/);
  assert.match(gate,/--test headless_node_d1 --test headless_node_d2 --test node_catalog_lab/);
});

test('B1b1 signed ledger and approval tests are covered on shared Cargo/Core changes',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  for(const wf of ['core-server-bootstrap-b1b1-floor.yml',
      'core-server-bootstrap-b1b1b-approval.yml',
      'core-server-bootstrap-b1b1c-witness.yml']){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',wf),'utf8');
    const policy=parse(body);
    for(const shared of ['aquilum-app/core/Cargo.toml','aquilum-app/core/src/lib.rs',
        'aquilum-app/Cargo.lock','aquilum-app/Cargo.toml']){
      assert.equal(matches(policy,shared),false,wf+' shared '+shared);
    }
    assert.match(body,/workflow_dispatch:/,'retain manual lab execution');
  }
  const floor=parse(fs.readFileSync(path.join(__dirname,'../workflows/core-server-bootstrap-b1b1-floor.yml'),'utf8'));
  const approval=parse(fs.readFileSync(path.join(__dirname,'../workflows/core-server-bootstrap-b1b1b-approval.yml'),'utf8'));
  const witness=parse(fs.readFileSync(path.join(__dirname,'../workflows/core-server-bootstrap-b1b1c-witness.yml'),'utf8'));
  assert.equal(matches(floor,'aquilum-app/core/src/server_release_floor_lab.rs'),true);
  assert.equal(matches(approval,'aquilum-app/core/src/server_owner_approval_lab.rs'),true);
  assert.equal(matches(witness,'aquilum-app/core/src/server_witness_lab.rs'),true);
  const gate=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.equal(parse(gate).t,'global','mandatory gate must remain unfiltered');
  assert.match(gate,/--features server-bootstrap-b1b1c-witness-lab/);
  for(const suite of ['server_bootstrap_b1b1a','server_bootstrap_b1b1b','server_bootstrap_b1b1c'])
    assert.ok(gate.includes('--test '+suite),suite+' retained in mandatory contract');
});

test('File Gate timing-only workflow is manual or self-triggered, never source fan-out',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const w=fs.readFileSync(path.join(__dirname,
    '../workflows/filegate-history-stage-profile.yml'),'utf8');
  const policy=parse(w);
  assert.equal(matches(policy,'aquilum-app/core/Cargo.toml'),false);
  for(const file of [
    'aquilum-app/core/src/files/document.rs',
    'aquilum-app/core/src/files/gate/write.rs',
    'aquilum-app/core/src/history/service.rs',
    'aquilum-app/core/src/history/store.rs'
  ]) assert.equal(matches(policy,file),false,'optional stage profiler must not run on '+file);
  assert.equal(matches(policy,'.github/workflows/filegate-history-stage-profile.yml'),true,
    'profiler workflow edits still exercise the profiler');
  assert.match(w,/^  workflow_dispatch:/m,'manual profiling stays available');
  assert.match(w,/Six fresh Core processes per note size/);
});

test('main-only registry source archive retains immutable PR cache safety',()=>{
  const code=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  const seed=code.split('  seed-rust-cache:\n')[1];
  assert.ok(seed,'existing seed job remains');
  assert.match(seed,/if: github.event_name == 'push'/);
  assert.match(seed,/id: registry-cache-probe/);
  assert.match(seed,/uses: actions\/cache\/restore@v4/);
  assert.match(seed,/lookup-only: true/);
  assert.match(seed,/cargo fetch --locked --manifest-path aquilum-app\/Cargo.toml/);
  assert.match(seed,/uses: actions\/cache\/save@v4/);
  assert.match(seed,/key: akula-registry-source-v1-/);
  assert.match(seed,/hashFiles\('aquilum-app\/Cargo.lock'\)/);
  assert.match(seed,/registry-cache-probe.outputs.cache-hit != 'true'/);
  assert.ok(!/cache\/save@v4/.test(code.split('  seed-rust-cache:\n')[0]),
    'no PR-context writable cache action');
  assert.match(seed,/~\/\.cargo\/registry\/cache/);
  assert.match(seed,/~\/\.cargo\/registry\/index/);
  assert.match(seed,/~\/\.cargo\/git/);
  assert.equal((code.match(/\n  seed-rust-cache:\n/g)||[]).length,1,'do not add new seed jobs');
});

test('safe isolated Cargo feature routes only Core, never unrelated Native/OS tests',()=>{
  const core='aquilum-app/core/Cargo.toml',lock='aquilum-app/Cargo.lock';
  const safe={shared_any_risky:false};
  const risky={shared_any_risky:true};
  assert.equal(safeManifestOnly([core,lock],safe),true);
  assert.equal(safeManifestOnly([core],safe),true);
  assert.equal(safeManifestOnly([core,lock,'aquilum-app/core/src/node_catalog_lab.rs'],safe),false);
  assert.equal(safeManifestOnly([lock],safe),false,'standalone lock edit remains full suite');
  assert.equal(safeManifestOnly([core,lock],risky),false,'existing dependency changes remain full suite');
  assert.equal(safeManifestOnly([core,lock],null),false,'missing classification never skips coverage');
  assert.equal(safeManifestOnly([],safe),false);
});
test('real git comment-only Cargo+lock change enters focused Core mode; risky changes stay full',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'akula-manifest-mode-'));
  const git=(...args)=>cp.execFileSync('git',args,{cwd:dir,encoding:'utf8'}).trim();
  const mode=base=>cp.execFileSync(process.execPath,
    [path.join(__dirname,'ci-rust-scope.cjs'),'--base='+base,'--head=HEAD'],
    {cwd:dir,encoding:'utf8'}).trim().split(/\r?\n/).find(line=>line.startsWith('mode='));
  const nl=String.fromCharCode(10);
  try{
    git('init','-q');git('config','user.name','CI');git('config','user.email','ci@example.invalid');
    const core=path.join(dir,'aquilum-app/core/Cargo.toml');
    const lock=path.join(dir,'aquilum-app/Cargo.lock');
    fs.mkdirSync(path.dirname(core),{recursive:true});
    fs.writeFileSync(core,[
      '[package]','name = "aquilum-core"','version = "0.3.10"',
      '[features]','default = []',''
    ].join(nl));
    const before=[
      '# Cargo lock','version = 4','',
      '[[package]]','name = "aquilum-core"','version = "0.3.10"',
      'dependencies = [',' "serde",',']','',
      '[[package]]','name = "serde"','version = "1.0.0"',
      'source = "registry+https://example.invalid/index"',
      'checksum = "fixture"',''
    ].join(nl);
    fs.writeFileSync(lock,before);
    git('add','.');git('commit','-qm','baseline');
    const base=git('rev-parse','HEAD');
    fs.appendFileSync(core,['# CI comment',''].join(nl));
    fs.writeFileSync(lock,before.replace('# Cargo lock'+nl,
      '# Cargo lock'+nl+'# CI header comment'+nl));
    git('add','.');git('commit','-qm','safe-comment');
    assert.equal(mode(base),'mode=manifest');
    // The original baseline comparison must see the changed existing checksum.
    fs.writeFileSync(lock,fs.readFileSync(lock,'utf8').replace('checksum = "fixture"','checksum = "changed"'));
    git('add','.');git('commit','-qm','unsafe-dependency-change');
    assert.equal(mode(base),'mode=workspace');
  }finally{fs.rmSync(dir,{recursive:true,force:true});}
});

test('updater security acceptance triggers for owned sources, installer scripts and policy edits',()=>{
  const owned=[
    'aquilum-app/native/src/updater/macos_transaction.rs',
    'aquilum-app/native/src/updater/tar.rs',
    'aquilum-app/native/src/updater/release/signed_candidate.rs',
    'aquilum-app/native/Cargo.toml',
    'scripts/install.sh','scripts/uninstall.sh',
    'scripts/verify-macos-release.sh','scripts/test-install-scripts.sh',
    'scripts/test-macos-release-verifier.sh',
    '.github/workflows/macos-smoke.yml',
    '.github/workflows/macos-updater-integration.yml',
    '.github/workflows/app-checks.yml',
    '.github/scripts/ci-rust-scope.cjs',
  ];
  for(const p of owned)assert.equal(updaterSecurityAffected([p]),true,p);
  for(const unrelated of [
    'aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock',
    'aquilum-app/core/src/sync/guarded_note_write.rs',
    'aquilum-app/native/src/app/tests/graph.rs',
    'aquilum-app/native/src/main.rs',
    '.github/scripts/ci-file-ratchet.cjs',
  ]) assert.equal(updaterSecurityAffected([unrelated]),false,unrelated);
  assert.equal(updaterSecurityAffected(['aquilum-app/native/src/main.rs',owned[0]]),true);
  assert.equal(updaterSecurityAffected([]),false);
  assert.equal(updaterSecurityAffected(null),true,'unknown file list fails closed');
  assert.equal(updaterSecurityAffected(['']),true,'malformed path fails closed');
});
test('rename away from updater retains original path for fail-closed security coverage',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'akula-updater-rename-'));
  const git=(...args)=>cp.execFileSync('git',args,{cwd:dir,encoding:'utf8'}).trim();
  try{
    git('init','-q');git('config','user.name','CI');
    git('config','user.email','ci@example.invalid');
    const before=path.join(dir,'aquilum-app/native/src/updater/macos_transaction.rs');
    const after=path.join(dir,'aquilum-app/native/src/legacy/other.rs');
    fs.mkdirSync(path.dirname(before),{recursive:true});
    fs.writeFileSync(before,'pub fn proof() {}');
    git('add','.');git('commit','-qm','before');
    const base=git('rev-parse','HEAD');
    fs.mkdirSync(path.dirname(after),{recursive:true});
    fs.renameSync(before,after);
    git('add','-A');git('commit','-qm','rename');
    const paths=changed(base,'HEAD',dir);
    assert.ok(paths.includes('aquilum-app/native/src/updater/macos_transaction.rs'));
    assert.ok(paths.includes('aquilum-app/native/src/legacy/other.rs'));
    assert.equal(updaterSecurityAffected(paths),true);
  }finally{fs.rmSync(dir,{recursive:true,force:true});}
});
test('required macOS security suites use exact ownership and still test lock behavior',()=>{
  const yaml=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(yaml,/updater_security_required: \$\{\{ steps\.scope\.outputs\.updater_security_required \}\}/);
  assert.match(yaml,/updater_security_required == 'true'/);
  for(const name of ['Security-test macOS updater TAR parser on Linux too',
      'Test macOS updater transaction with shared installer lock and crash recovery']){
    const marker='      - name: '+name+'\n';
    const i=yaml.indexOf(marker);
    assert.ok(i>=0,name+' must remain an executable required test');
    const section=yaml.slice(i,i+270);
    assert.match(section,/if: needs\.classify\.outputs\.updater_security_required == 'true'/,
      name+' executes for owned files, not unrelated workspace changes');
  }
  assert.match(yaml,/akula_updater_txn_tests" --nocapture --test-threads=1/);
  assert.match(yaml,/txn::tests::fresh_user_install_has_one_shared_lock/);
  assert.match(yaml,/Run Core tests in both modes \(safe manifest additions\)/);
  assert.match(yaml,/--features sync-engine/);
  assert.match(yaml,/B1a\/B1b0\/B1b1 release and approval compatibility/);
  assert.match(yaml,/Node D1\/D1\.5\/D2 scoped read and loopback compatibility/);
  assert.match(yaml,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});

test('native-only lane runs all Native tests and still requires frame-fidelity Main PR gate',()=>{
  const yaml=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(yaml,/name: Run Native package tests for native-source changes/);
  assert.match(yaml,/if: needs\.classify\.outputs\.mode == 'native'/);
  assert.match(yaml,/cargo test -p aquilum-native --locked -- --skip app::tests::perf::frames::/);
  assert.match(yaml,/if: needs\.classify\.outputs\.mode == 'workspace' \|\| needs\.classify\.outputs\.mode == 'native'/,
    'Native-only still installs Linux build dependencies');
  assert.match(yaml,/mode == 'workspace' \|\| needs\.classify\.outputs\.mode == 'native'\)/,
    'Native frame-fidelity must run for Native-only changes');
  assert.match(yaml,/workspace\|native\)\n\s+if \[ "\$FRAMES_RESULT" != success \]/,
    'protected Main PR gate requires success for Native fidelity');
  assert.match(yaml,/sync\|none\|manifest\)\n\s+if \[ "\$FRAMES_RESULT" != skipped \]/,
    'scopes with unchanged Native keep separate fidelity skipped');
  assert.match(yaml,/if: needs\.classify\.outputs\.mode == 'workspace'\n\s+working-directory: aquilum-app/,
    'mixed/Core/root changes retain full workspace');
  assert.match(yaml,/cargo test --workspace --locked/);
  assert.match(yaml,/cargo test -p aquilum-core --features sync-engine --locked/);
  assert.match(yaml,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});

test('Linux first-GUI ACK real-process security moved into the required owner fast lane',()=>{
  const app=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  const beta=fs.readFileSync(path.join(__dirname,'../workflows/appimage-beta.yml'),'utf8');
  const marker='      - name: Verify Linux updater first-GUI ACK IPC without AppImage build\n';
  const start=app.indexOf(marker),end=app.indexOf('\n      - name: Restore shared main Rust cache',start);
  assert.ok(start>0&&end>start,'must be a standalone required security step before any full Rust build');
  const segment=app.slice(start,end);
  assert.match(segment,/if: needs\.classify\.outputs\.updater_security_required == 'true'/);
  assert.match(segment,/rustc --edition=2024 --test -D warnings/);
  assert.match(segment,/aquilum-app\/native\/src\/updater\/linux_first_gui_ack\.rs/);
  for(const proof of [
    'real_inherited_stream_ack_has_child_pid_nonce_and_generation_digest',
    'wrong_parent_cannot_emit_even_with_provided_nonce_and_fd',
    'test result: ok. 5 passed; 0 failed;',
    'AKULA_LINUX_FIRST_GUI_ACK_IPC_PASS',
  ]){
    assert.ok(segment.includes(proof),'required Linux ACK security proof: '+proof);
    assert.ok(beta.includes(proof),'original independent AppImage proof remains: '+proof);
  }
  assert.match(app,/needs: \[classify, required-tests, native-frames, shared-labs\]/,
    'security stage continues to block Main PR gate on failure');
});


test('every standalone Rust module has a direct PR owner and executable Cargo test',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  // Explicitly approved owned standalone packages. Adding a module without
  // a direct PR test owner is unsafe: Main PR gate otherwise sees mode=none.
  const owners={
    'bootstrap-b0':'bootstrap-b0-isolated.yml',
    'bootstrap-b1a':'core-server-bootstrap-b1a-release.yml',
    'bootstrap-b1b0':'core-server-bootstrap-b1b0-staging.yml',
    'node-catalog':'node-catalog-isolated.yml',
    'dropbox-t0':'dropbox-t0-isolated.yml',
    'dropbox-auth-lab':'dropbox-t0-isolated.yml',
    'dropbox-secret-store':'dropbox-t0-isolated.yml',
    'dropbox-connect-lab':'dropbox-t0-isolated.yml',
  };
  const moduleRoot=path.join(__dirname,'../../modules');
  const found=fs.readdirSync(moduleRoot,{withFileTypes:true})
    .filter(entry=>entry.isDirectory()&&fs.existsSync(path.join(moduleRoot,entry.name,'Cargo.toml')))
    .map(entry=>entry.name).sort();
  assert.deepEqual(found,Object.keys(owners).sort(),
    'new standalone Cargo module must have a reviewed direct CI owner');
  for(const [name,workflowName] of Object.entries(owners)){
    const manifest='modules/'+name+'/Cargo.toml';
    const workflow=fs.readFileSync(path.join(__dirname,'../workflows',workflowName),'utf8');
    const policy=parse(workflow);
    assert.equal(matches(policy,manifest),true,'missing direct PR subscription: '+manifest);
    const testLine=new RegExp('^\\s*(?:run:\\s*)?cargo test --manifest-path modules/'+name+'/Cargo\\.toml(?:\\s|$)','m');
    assert.match(workflow,testLine,'standalone crate test must run in owner workflow: '+name);
  }
});


test('ignored Search performance suites are opt-in while Core correctness remains required',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const {policyErrors}=require('./ci-pr-concurrency.cjs');
  const perWorkflow={
    'wiki-parser-perf.yml':['aquilum-app/core/src/search/wiki/parser.rs'],
    'untracked-delete-perf.yml':[
      'aquilum-app/core/src/search/sync.rs',
      'aquilum-app/core/src/search/index_document.rs',
    ],
    'search-hits-perf.yml':[
      'aquilum-app/core/src/search/hits/repository.rs',
      'aquilum-app/core/src/search/index_tests.rs',
    ],
  };
  for(const [name,sourcePaths] of Object.entries(perWorkflow)){
    const workflow='.github/workflows/'+name;
    const body=fs.readFileSync(path.join(__dirname,'../workflows',name),'utf8');
    const rule=parse(body);
    assert.equal(matches(rule,workflow),true,'self-edits must still test '+name);
    for(const source of sourcePaths){
      assert.equal(matches(rule,source),false,'no automatic expensive Search benchmark on '+source);
      assert.equal(classify([source]),'workspace','required Core tests for '+source);
    }
    assert.match(body,/^  workflow_dispatch:\s*$/m,'manual benchmark remains '+name);
    assert.deepEqual(policyErrors(body,workflow),[],'manual benchmark must not be canceled by unrelated run: '+name);
    assert.match(body,/--ignored/,'only optional timing measurements are routed: '+name);
  }
  const read=source=>fs.readFileSync(path.join(__dirname,'../../',source),'utf8');
  const parser=read('aquilum-app/core/src/search/wiki/parser.rs');
  assert.match(parser,/#\[test\]\s+fn fast_path_is_identical_to_the_full_markdown_scanner/);
  const sync=read('aquilum-app/core/src/search/sync.rs');
  assert.match(sync,/#\[test\]\s+fn deleting_an_untracked_note_does_not_acquire_writer_or_commit/);
  assert.match(sync,/#\[test\]\s+fn orphaned_index_document_is_still_deleted_without_sqlite_metadata/);
  const document=read('aquilum-app/core/src/search/index_document.rs');
  assert.match(document,/#\[test\]\s+fn a_crlf_note_is_indexed_as_lf/);
  const indexing=read('aquilum-app/core/src/search/index_tests.rs');
  assert.match(indexing,/#\[test\]\s+fn disk_index_searches_title_and_body_by_case_insensitive_prefix/);
  const app=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(app,/Run full Rust workspace and sync-engine tests/);
  assert.match(app,/cargo test --workspace --locked/);
});
