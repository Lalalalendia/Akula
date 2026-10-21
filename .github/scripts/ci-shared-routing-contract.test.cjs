'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const core=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
const routeEntries=[["host-custody","sync-host-custody.yml",["core"]],["b1b2b-ubuntu","core-server-bootstrap-b1b2b-ubuntu.yml",["core"]],["b1c1-vm","core-server-bootstrap-b1c1-real-elf-vm.yml",["core"]],["b1b2a-systemd","core-server-bootstrap-b1b2a-systemd.yml",["core"]],["b1b3b-helper","core-server-bootstrap-b1b3b-root-helper.yml",["core"]],["b1b3c-fd","core-server-bootstrap-b1b3c-signed-fd.yml",["core","lib"]],["b1b4a-cas","core-server-bootstrap-b1b4a-cas-fence.yml",["core","lib"]],["b1b4b-witness","core-server-bootstrap-b1b4b-os-witness.yml",["core","lib"]],["b1b4b1-ipc","core-server-bootstrap-b1b4b1-witness-ipc.yml",["core"]],["b1c0-node","core-server-bootstrap-b1c0-fixed-node.yml",["core","lib"]],["b1d0-opt","core-server-bootstrap-b1d0-opt.yml",["core","lib"]],["b0b-ssh","core-server-bootstrap-b0b.yml",["core","lib"]],["b1b3a-ipc","core-server-bootstrap-b1b3a-ipc.yml",["core","lib"]],["p2p-offer","p2p-offer-lab.yml",["core","lib"]],["s3-minio","sync-s3-minio.yml",["core","lock"]],["s3-platform","sync-s3-platform.yml",["core","lock"]],["pinned-read","sync-pinned-read.yml",["core","lock"]],["s3-reservation","sync-reserved-s3-transaction.yml",["core","lock"]],["sync-protection","sync-protection.yml",["core","lock"]],["sync-transaction","sync-guarded-transaction.yml",["core","lock"]],["text-delta","sync-text-delta-rust.yml",["core","lock"]],["appimage-beta","appimage-beta.yml",["lock"]],["windows-beta","windows-beta.yml",["lock"]],["vello-half-tie","native-vello-half-tie-regression.yml",["root","lock"]],["linux-beta","linux-beta.yml",["lock"]]];
test('migrated security workflows retain source-specific triggers and manual invocation',()=>{
  assert.equal(routeEntries.length,25);
  for(const [id,file,keys] of routeEntries){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',file),'utf8');
    assert.match(body,/^on:\n  workflow_call:\n  pull_request:/m,file+' stays callable and reacts to owned paths');
    const header=body.split(/\n(?:jobs:|permissions:|concurrency:|env:)/)[0];
    for(const p of ['aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock','aquilum-app/core/src/lib.rs','aquilum-app/Cargo.toml']){
      assert.ok(!header.includes("- '"+p+"'")&&!header.includes('- "'+p+'"'),file+': no broad trigger '+p);
    }
    assert.match(body,new RegExp(file.replace(/\./g,'\\.'),'m'),file+' is triggered by edits to itself');
    assert.match(core,new RegExp('  route-'+id+':\\n'),'Main required gate must call '+id);
    assert.match(core,new RegExp('uses: \\.\\/\\.github\\/workflows\\/'+file.replace(/\./g,'\\.')+'\\n'));
    for(const key of keys)assert.ok(core.includes("needs.classify.outputs.shared_"+({core:'core_cargo',lib:'core_lib',lock:'lock',root:'root_cargo'}[key])+"_risky"),'route retains '+key);
  }
});
test('the sole protected Main PR gate depends on both fail-closed aggregators',()=>{
  assert.match(core,/needs: \[classify, required-tests, native-frames, routed-shared-gate-1, routed-shared-gate-2\]/);
  assert.match(core,/SHARED_GATE_1: \$\{\{ needs\.routed-shared-gate-1\.result \}\}/);
  assert.match(core,/SHARED_GATE_2: \$\{\{ needs\.routed-shared-gate-2\.result \}\}/);
  assert.match(core,/Required shared-manifest security\/OS regressions did not pass/);
  assert.match(core,/node --test .github\/scripts\/ci-shared-manifest-route.test.cjs/);
  assert.match(core,/ci-shared-manifest-route.cjs --base="\$BASE_SHA" --head=HEAD/);
});
