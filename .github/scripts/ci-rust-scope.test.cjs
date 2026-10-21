'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const cp=require('node:child_process');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {classify,changed}=require('./ci-rust-scope.cjs');

test('pure documentation and CI telemetry do not compile Rust',()=>{
  assert.equal(classify(['README.md','docs/engineering-budgets.md',
    '.github/scripts/ci-run-budget.cjs']), 'none');
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
    'aquilum-app/native/vendor/ratex-parser/src/parser.rs',
    '.github/workflows/app-checks.yml'
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
  for(const file of shared){
    assert.equal(matches(b1a,file),false,'shared changes use required gate: '+file);
    assert.equal(matches(b1b0,file),false,'shared changes use required gate: '+file);
  }
  for(const file of ['modules/bootstrap-b1a/src/lib.rs',
      'aquilum-app/core/src/server_release_lab.rs']){
    assert.equal(matches(b1a,file),true,'B1a owner path: '+file);
    assert.equal(matches(b1b0,file),true,'B1b0 dependent path: '+file);
  }
  assert.equal(matches(b1b0,'modules/bootstrap-b1b0/src/server_staging_lab.rs'),true);
  assert.equal(matches(b1a,'aquilum-app/core/tests/server_bootstrap_b1a.rs'),true);
  assert.equal(matches(b1b0,'aquilum-app/core/tests/server_bootstrap_b1b0.rs'),true);
  const gate=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.equal(parse(gate).t,'global','required application gate must remain unfiltered');
  assert.match(gate,/b1_shared_compat: \$\{\{ steps\.scope\.outputs\.b1_shared_compat \}\}/);
  assert.match(gate,/--features server-bootstrap-b1b-custody-lab/);
  assert.match(gate,/--test server_bootstrap_b1a --test server_bootstrap_b1b0/);
});
