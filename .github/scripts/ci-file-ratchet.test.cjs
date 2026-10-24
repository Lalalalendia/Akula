'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path'),cp=require('node:child_process');
const ratchet=require('./ci-file-ratchet.cjs');
function fixture(fn){const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'akula-ratchet-'));const git=(...a)=>cp.execFileSync('git',a,{cwd,encoding:'utf8'}).trim();const write=(p,s)=>{const file=path.join(cwd,p);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,s)};const commit=()=>{git('add','-A');git('commit','-qm','test');return git('rev-parse','HEAD')};try{git('init','-q');git('config','user.name','CI');git('config','user.email','ci@example.invalid');write('aquilum-app/core/src/old.rs','fn a(){}\n'.repeat(3));const base=commit();fn({cwd,git,write,commit,base})}finally{fs.rmSync(cwd,{recursive:true,force:true})}}
test('existing source size cannot grow even by one line',()=>fixture(x=>{x.write('aquilum-app/core/src/old.rs','fn a(){}\n'.repeat(4));assert.match(ratchet.audit(x.cwd,x.base,x.commit()).errors.join(' '),/SOURCE SIZE/)}));
test('shrinking file tightens next baseline',()=>fixture(x=>{x.write('aquilum-app/core/src/old.rs','fn a(){}\n'.repeat(2));const low=x.commit();assert.equal(ratchet.audit(x.cwd,x.base,low).errors.length,0);x.write('aquilum-app/core/src/old.rs','fn a(){}\n'.repeat(3));assert.match(ratchet.audit(x.cwd,low,x.commit()).errors.join(' '),/SOURCE SIZE/)}));
test('new source >800 lines is refused',()=>fixture(x=>{x.write('aquilum-app/core/src/new.rs','fn a(){}\n'.repeat(801));assert.match(ratchet.audit(x.cwd,x.base,x.commit()).errors.join(' '),/NEW SOURCE/)}));
test('new broad workflow subscription refused',()=>fixture(x=>{x.write('.github/workflows/new.yml',"on:\n  pull_request:\n    paths:\n      - 'aquilum-app/core/src/**'\n");assert.match(ratchet.audit(x.cwd,x.base,x.commit()).errors.join(' '),/FANOUT GROWTH/)}));
test('new global PR workflow refused',()=>fixture(x=>{x.write('.github/workflows/global.yml','on:\n  pull_request:\n');assert.match(ratchet.audit(x.cwd,x.base,x.commit()).errors.join(' '),/NEW GLOBAL/)}));
test('negative ordered path rule removes ownership',()=>{const p=ratchet.parse("on:\n  pull_request:\n    paths:\n      - 'src/**'\n      - '!src/skip.rs'\n");assert.equal(ratchet.matches(p,'src/foo.rs'),true);assert.equal(ratchet.matches(p,'src/skip.rs'),false)});
test('new leaf over six checks is refused',()=>fixture(x=>{x.write('modules/c/src/file.rs','fn a(){}\n');for(let i=0;i<7;i++)x.write('.github/workflows/lab'+i+'.yml',"on:\n  pull_request:\n    paths:\n      - 'modules/c/src/**'\n");assert.match(ratchet.audit(x.cwd,x.base,x.commit()).errors.join(' '),/NEW FANOUT/)}));

test('double-star slash matches same-level files',()=>{
 const policy=ratchet.parse("on:\n  pull_request:\n    paths:\n      - 'src/**/file?.rs'\n");
 assert.equal(ratchet.matches(policy,'src/file1.rs'),true);
 assert.equal(ratchet.matches(policy,'src/sub/file2.rs'),true);
 assert.equal(ratchet.matches(policy,'other/file1.rs'),false);
});
test('paths-ignore and inline unfiltered PR work',()=>{
 const ignore=ratchet.parse("on:\n  pull_request:\n    paths-ignore:\n      - 'docs/**'\n");
 assert.equal(ratchet.matches(ignore,'src/actual.rs'),true);
 assert.equal(ratchet.matches(ignore,'docs/readme.md'),false);
 assert.equal(ratchet.parse("on: [push, pull_request]\n").t,'global');
 assert.equal(ratchet.parse("on:\n  pull_request:\n    branches: [main]\n").t,'global');
 assert.throws(()=>ratchet.parse("on:\n  pull_request:\n    paths: []\n"),/Missing PR paths/);
});
test('workflow-only broadened paths-ignore fails closed',()=>fixture(x=>{
 x.write('.github/workflows/lab.yml',"on:\n  pull_request:\n    paths-ignore:\n      - 'docs/**'\n");
 const head=x.commit();
 assert.match(ratchet.audit(x.cwd,x.base,head).errors.join(' '),/FANOUT GROWTH/);
}));
test('removing a conditional subscription is permitted',()=>fixture(x=>{
 x.write('.github/workflows/lab.yml',"on:\n  pull_request:\n    paths:\n      - 'aquilum-app/core/src/**'\n");
 const mid=x.commit();
 fs.unlinkSync(path.join(x.cwd,'.github/workflows/lab.yml'));
 const head=x.commit();
 assert.equal(ratchet.audit(x.cwd,mid,head).errors.length,0);
}));
test('replacement workflow cannot silently acquire previous consumer',()=>fixture(x=>{
 x.write('.github/workflows/old.yml',"on:\n  pull_request:\n    paths:\n      - 'aquilum-app/core/src/**'\n");
 const baseline=x.commit();
 fs.unlinkSync(path.join(x.cwd,'.github/workflows/old.yml'));
 x.write('.github/workflows/other.yml',"on:\n  pull_request:\n    paths:\n      - 'aquilum-app/core/src/**'\n");
 const head=x.commit();
 assert.match(ratchet.audit(x.cwd,baseline,head).errors.join(' '),/NEW WORKFLOW CONSUMER/);
}));
test('renamed Rust file may not increase its line count',()=>fixture(x=>{
 const before='aquilum-app/core/src/old.rs',after='aquilum-app/core/src/moved.rs';
 fs.renameSync(path.join(x.cwd,before),path.join(x.cwd,after));
 x.write(after,'fn a(){}\n'.repeat(4));
 assert.match(ratchet.audit(x.cwd,x.base,x.commit()).errors.join(' '),/SOURCE SIZE/);
}));
test('test-only Rust fixture is excluded',()=>fixture(x=>{
 x.write('aquilum-app/core/tests/regression.rs','fn run(){}\n'.repeat(900));
 assert.equal(ratchet.audit(x.cwd,x.base,x.commit()).errors.length,0);
}));


test('Vello profiling workflows do not fan out on generic renderer/pass or DXGI edits',()=>{
  const names=[
    'native-damage-profile.yml','native-vello-single-region-gate.yml',
    'native-vello-natural-stages.yml','native-vello-dual-route.yml',
    'native-vello-worker-crossover.yml','native-vello-single-region-rss.yml',
    'native-vello-release-profile.yml'
  ];
  const entries=names.map(n=>{
    const body=fs.readFileSync(path.join(__dirname,'../workflows',n),'utf8');
    assert.match(body,/^  workflow_dispatch:\s*$/m,'manual profiler preserved: '+n);
    return [n,ratchet.parse(body)];
  });
  const consumers=p=>entries.filter(([,policy])=>ratchet.matches(policy,p)).map(([name])=>name);
  for(const [name,policy] of entries)assert.equal(ratchet.matches(policy,'.github/workflows/'+name),true,
    'changed diagnostic workflows still self-test: '+name);
  for(const source of [
    'aquilum-app/native/src/render/renderer/pass.rs',
    'aquilum-app/native/src/output/dxgi.rs',
    'aquilum-app/native/src/output/transfer.rs',
    'aquilum-app/native/src/render/scroll.rs'
  ])assert.deepEqual(consumers(source),[],'unrelated perf fanout on '+source);
  assert.deepEqual(consumers('aquilum-app/native/src/render/damage/mod.rs'),['native-damage-profile.yml']);
  assert.deepEqual(consumers('aquilum-app/native/src/render/renderer/mod.rs'),['native-vello-single-region-gate.yml']);
  assert.deepEqual(consumers('aquilum-app/native/src/app/tests/perf/worker_router.rs'),
    ['native-vello-single-region-gate.yml','native-vello-dual-route.yml','native-vello-single-region-rss.yml']);
  assert.deepEqual(consumers('aquilum-app/native/src/app/tests/perf/vello_release.rs'),['native-vello-release-profile.yml']);
  const gate=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(gate,/name: Native frame fidelity/);
  assert.match(gate,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});
