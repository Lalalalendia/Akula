'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {topBlock, prTrigger, policyErrors, changedWorkflows, audit} = require('./ci-pr-concurrency.cjs');
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
