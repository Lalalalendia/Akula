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
    assert.equal(classify(changed(base,'HEAD')),'sync');
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
test('missing Git refs do not silently skip mandatory checks',()=>{
  assert.throws(()=>changed('not-a-git-sha','HEAD'));
});
