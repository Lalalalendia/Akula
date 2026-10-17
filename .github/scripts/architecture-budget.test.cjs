'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const cp=require('node:child_process');
const {own,metrics,dependencies,compare,assess,report}=require('./architecture-budget.cjs');
test('filters vendored, generated and test source from production budgets',()=>{
  assert.equal(own('aquilum-app/core/src/sync/read.rs'),true);
  for(const p of ['aquilum-app/native/vendor/lib.rs','aquilum-app/core/src/sync/fault_tests.rs',
    'aquilum-app/native/src/ui/generated.rs','aquilum-app/core/tests/file.rs',
    '.github/bench/bench.rs']) assert.equal(own(p),false,p);
});
test('measures distinct outbound modules and reexports',()=>{
  const m=metrics('use crate::sync::A;\nuse crate::sync::B;\npub use crate::search::C;\npub mod foo;\n// crate::ignored::X\n');
  assert.equal(m.fanout,2);
  assert.equal(m.exports,1);
  assert.equal(m.children,1);
  assert.equal(m.lines,5);
});
test('preserves existing large files without warning',()=>{
  const base='fn a() {}\n'.repeat(850);
  assert.deepEqual(compare('a.rs',base,base).notes,[]);
  assert.equal(compare('a.rs',base,base+'fn b() {}\n'.repeat(151)).notes.length,1);
  assert.equal(compare('a.rs',null,'fn b() {}\n'.repeat(601)).notes.length,1);
});
test('warns on fanout growth, not fanout alone',()=>{
  const src=n=>Array.from({length:n},(_,i)=>'use crate::mod'+i+'::A;').join('\n');
  assert.equal(compare('a.rs',src(9),src(9)).notes.length,0);
  assert.equal(compare('a.rs',src(8),src(10)).notes.length,1);
});
test('Cargo dependency counts exclude dev dependencies and deduplicate target sections',()=>{
  const s='[dependencies]\nserde=\"1\"\n[dev-dependencies]\ntempfile=\"3\"\n[target.\x27cfg(unix)\x27.dependencies]\nrustix=\"0.38\"\n';
  assert.deepEqual([...dependencies(s)].sort(),['rustix','serde']);
});
test('growth warnings do not fail or penalize deleted source',()=>{
  const rows=[compare('x.rs','fn x(){}\n'.repeat(900),null)];
  const diff=assess(rows,[]);
  assert.equal(diff.netLines,-900);
  assert.equal(diff.notes.length,0);
  assert.match(report(diff),/advisory only/);
  const newFiles=Array.from({length:11},(_,i)=>compare('x'+i+'.rs',null,'fn x(){}\n'.repeat(50)));
  const large=assess(newFiles,['a','b','c']);
  assert.equal(large.notes.length,3);
});
test('CLI compares base and head of a real minimal git repository',()=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'akula-budget-'));
  function git(args){return cp.execFileSync('git',args,{cwd:root,encoding:'utf8'}).trim();}
  try {
    git(['init','-q']);
    git(['config','user.name','CI']);
    git(['config','user.email','ci@example.invalid']);
    const file=path.join(root,'aquilum-app/core/src/sync/file.rs');
    fs.mkdirSync(path.dirname(file),{recursive:true});
    fs.writeFileSync(file,'fn hello() {}\n');
    git(['add','.']);git(['commit','-qm','base']);
    const base=git(['rev-parse','HEAD']);
    fs.appendFileSync(file,'fn world() {}\n');
    git(['add','.']);git(['commit','-qm','change']);
    const stdout=cp.execFileSync('node',[path.join(__dirname,'architecture-budget.cjs'),
      '--base='+base,'--head=HEAD'],{cwd:root,encoding:'utf8'});
    assert.match(stdout,/Changed owned Rust files: 1/);
    assert.match(stdout,/1 -> 2/);
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});
