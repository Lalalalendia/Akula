'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const core=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
const shared=fs.readFileSync(path.join(__dirname,'../workflows/ci-shared-manifest-labs.yml'),'utf8');
const routeEntries=[["host-custody","sync-host-custody.yml",["core"]],["b1b2b-ubuntu","core-server-bootstrap-b1b2b-ubuntu.yml",["core"]],["b1c1-vm","core-server-bootstrap-b1c1-real-elf-vm.yml",["core"]],["b1b2a-systemd","core-server-bootstrap-b1b2a-systemd.yml",["core"]],["b1b3b-helper","core-server-bootstrap-b1b3b-root-helper.yml",["core"]],["b1b3c-fd","core-server-bootstrap-b1b3c-signed-fd.yml",["core","lib"]],["b1b4a-cas","core-server-bootstrap-b1b4a-cas-fence.yml",["core","lib"]],["b1b4b-witness","core-server-bootstrap-b1b4b-os-witness.yml",["core","lib"]],["b1b4b1-ipc","core-server-bootstrap-b1b4b1-witness-ipc.yml",["core"]],["b1c0-node","core-server-bootstrap-b1c0-fixed-node.yml",["core","lib"]],["b1d0-opt","core-server-bootstrap-b1d0-opt.yml",["core","lib"]],["b0b-ssh","core-server-bootstrap-b0b.yml",["core","lib"]],["b1b3a-ipc","core-server-bootstrap-b1b3a-ipc.yml",["core","lib"]],["p2p-offer","p2p-offer-lab.yml",["core","lib"]],["s3-minio","sync-s3-minio.yml",["core","lock"]],["s3-platform","sync-s3-platform.yml",["core","lock"]],["pinned-read","sync-pinned-read.yml",["core","lock"]],["s3-reservation","sync-reserved-s3-transaction.yml",["core","lock"]],["sync-protection","sync-protection.yml",["core","lock"]],["sync-transaction","sync-guarded-transaction.yml",["core","lock"]],["text-delta","sync-text-delta-rust.yml",["core","lock"]],["appimage-beta","appimage-beta.yml",["lock"]],["windows-beta","windows-beta.yml",["lock"]],["vello-half-tie","native-vello-half-tie-regression.yml",["root","lock"]],["linux-beta","linux-beta.yml",["lock"]],["installed-owner","sync-installed-os-owner-witness.yml",["core"]]];

test('26 owned workflows remain direct for own sources, called only by impacted shared-manifest route',()=>{
  assert.equal(routeEntries.length,26);
  assert.match(shared,/^on:\n  workflow_call:/m);
  for(const [id,file,keys] of routeEntries){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',file),'utf8');
    assert.match(body,/^on:\n  workflow_call:\n  pull_request:/m);
    const header=body.split(/\n(?:jobs:|permissions:|concurrency:|env:)/)[0];
    for(const p of ['aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock','aquilum-app/core/src/lib.rs','aquilum-app/Cargo.toml']){
      assert.ok(!header.includes("- '"+p+"'")&&!header.includes('- "'+p+'"'),file+' has no broad trigger '+p);
    }
    const group=body.match(/^  group: (.*)$/m)?.[1];
    if(group)assert.ok(group.includes('github.workflow'),file+' direct/called concurrency separated');
    assert.match(body,new RegExp(file.replace(/\./g,'\\.'),'m'),file+' watches itself');
    assert.match(shared,new RegExp('  route-'+id+':\\n'),file+' included');
    assert.ok(shared.includes('uses: ./.github/workflows/'+file),file+' callable');
    for(const key of keys)assert.ok(shared.includes('inputs.'+key),'input '+key+' used by router');
  }
});
test('one skipped caller on harmless edits, with old Main PR gate failing closed on risky diffs',()=>{
  assert.match(core,/  shared-labs:\n/);
  assert.match(core,/uses: \.\/\.github\/workflows\/ci-shared-manifest-labs.yml/);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
  assert.match(core,/ROUTED_SHARED_RESULT: \$\{\{ needs\.shared-labs\.result \}\}/);
  assert.match(core,/Required shared-manifest security\/OS regressions did not pass/);
  assert.match(shared,/  boundary-gate:\n/);
  assert.match(shared,/test "\$A" = success/);
  assert.match(shared,/test "\$B" = success/);
  assert.match(core,/ci-shared-manifest-route.cjs --base="\$BASE_SHA" --head=HEAD/);
});
