'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const core=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
const shared=fs.readFileSync(path.join(__dirname,'../workflows/ci-shared-manifest-labs.yml'),'utf8');
const routeEntries=[["host-custody","sync-host-custody.yml",["core"]],["b1b2b-ubuntu","core-server-bootstrap-b1b2b-ubuntu.yml",["core"]],["b1c1-vm","core-server-bootstrap-b1c1-real-elf-vm.yml",["core"]],["b1b2a-systemd","core-server-bootstrap-b1b2a-systemd.yml",["core"]],["b1b3b-helper","core-server-bootstrap-b1b3b-root-helper.yml",["core"]],["b1b3c-fd","core-server-bootstrap-b1b3c-signed-fd.yml",["core","lib"]],["b1b4a-cas","core-server-bootstrap-b1b4a-cas-fence.yml",["core","lib"]],["b1b4b-witness","core-server-bootstrap-b1b4b-os-witness.yml",["core","lib"]],["b1b4b1-ipc","core-server-bootstrap-b1b4b1-witness-ipc.yml",["core"]],["b1c0-node","core-server-bootstrap-b1c0-fixed-node.yml",["core","lib"]],["b1d0-opt","core-server-bootstrap-b1d0-opt.yml",["core","lib"]],["b0b-ssh","core-server-bootstrap-b0b.yml",["core","lib"]],["b1b3a-ipc","core-server-bootstrap-b1b3a-ipc.yml",["core","lib"]],["p2p-offer","p2p-offer-lab.yml",["core","lib"]],["s3-minio","sync-s3-minio.yml",["core","lock"]],["s3-platform","sync-s3-platform.yml",["core","lock"]],["pinned-read","sync-pinned-read.yml",["core","lock"]],["s3-reservation","sync-reserved-s3-transaction.yml",["core","lock"]],["sync-protection","sync-protection.yml",["core","lock"]],["sync-transaction","sync-guarded-transaction.yml",["core","lock"]],["text-delta","sync-text-delta-rust.yml",["core","lock"]],["appimage-beta","appimage-beta.yml",["lock"]],["windows-beta","windows-beta.yml",["lock"]],["vello-half-tie","native-vello-half-tie-regression.yml",["root","lock"]],["linux-beta","linux-beta.yml",["lock"]],["installed-owner","sync-installed-os-owner-witness.yml",["core","owner"]]];

test('26 owned workflows remain direct for own sources, called only by impacted shared-manifest route',()=>{
  assert.equal(routeEntries.length,26);
  assert.match(shared,/^on:\n  workflow_call:/m);
  for(const [id,file,keys] of routeEntries){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',file),'utf8');
    if(id==='installed-owner'){
      assert.match(body,/^on:\n  workflow_call:\n  workflow_dispatch:/m,'owner must be called, not independently started');
      assert.doesNotMatch(body,/^  pull_request:/m,'no untracked direct OS owner gate');
      const sources=require('./ci-shared-manifest-route.cjs');
      for(const file of ["aquilum-app/core/src/sync/host_install_unix_root.rs","aquilum-app/core/src/sync/host_install_witness.rs","aquilum-app/core/src/sync/mod.rs","aquilum-app/native/Cargo.toml",".github/workflows/sync-installed-os-owner-witness.yml","docs/sync/adr-0038-unix-root-owned-install-receipt.md"]){
        assert.equal(sources.assess({paths:[file]}).installed_owner_owned,true,file);
      }
    } else assert.match(body,/^on:\n  workflow_call:\n  pull_request:/m);
    const header=body.split(/\n(?:jobs:|permissions:|concurrency:|env:)/)[0];
    for(const p of ['aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock','aquilum-app/core/src/lib.rs','aquilum-app/Cargo.toml']){
      assert.ok(!header.includes("- '"+p+"'")&&!header.includes('- "'+p+'"'),file+' has no broad trigger '+p);
    }
    const group=body.match(/^  group: (.*)$/m)?.[1];
    if(group)assert.ok(group.includes('github.workflow'),file+' direct/called concurrency separated');
    if(id!=='installed-owner')assert.match(body,new RegExp(file.replace(/\./g,'\\.'),'m'),file+' watches itself');
    assert.match(shared,new RegExp('  route-'+id+':\\n'),file+' included');
    assert.ok(shared.includes('uses: ./.github/workflows/'+file),file+' callable');
    for(const key of keys)assert.ok(shared.includes('inputs.'+key),'input '+key+' used by router');
  }
});
test('one skipped caller on harmless edits, with old Main PR gate failing closed on risky diffs',()=>{
  assert.match(core,/  shared-labs:\n/);
  assert.match(core,/installed_owner_owned/);
  assert.match(shared,/inputs\.core \|\| inputs\.owner/);
  assert.match(core,/uses: \.\/\.github\/workflows\/ci-shared-manifest-labs.yml/);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
  assert.match(core,/ROUTED_SHARED_RESULT: \$\{\{ needs\.shared-labs\.result \}\}/);
  assert.match(core,/Required shared-manifest security\/OS regressions did not pass/);
  assert.match(shared,/  boundary-gate:\n/);
  assert.match(shared,/test "\$A" = success/);
  assert.match(shared,/test "\$B" = success/);
  assert.match(core,/ci-shared-manifest-route.cjs --base="\$BASE_SHA" --head=HEAD/);
});

test('ordinary Core PRs schedule no independent perf workflows; opt-in routes remain',()=>{
  const perf=fs.readFileSync(path.join(__dirname,'../workflows/core-perf.yml'),'utf8');
  const compare=fs.readFileSync(path.join(__dirname,'../workflows/core-perf-compare.yml'),'utf8');
  assert.match(perf,/^on:\n  workflow_call:/m);
  assert.match(perf,/types: \[labeled, edited\]/);
  assert.doesNotMatch(perf,/types: \[opened, synchronize/);
  assert.match(perf,/^  workflow_dispatch:/m,'manual baseline remains');
  assert.match(perf,/^  paired:\n/m,'paired comparison is called from one baseline workflow');
  assert.match(perf,/uses: \.\/\.github\/workflows\/core-perf-compare\.yml/);
  assert.match(compare,/^on:\n  workflow_call:/m);
  assert.doesNotMatch(compare,/^  pull_request:/m,'paired comparison has no independent PR trigger');
  assert.match(core,/perf_core_changed: \$\{\{ steps\.scope\.outputs\.perf_core_changed \}\}/);
  assert.match(core,/perf-opt-in:/);
  assert.match(core,/uses: \.\/\.github\/workflows\/core-perf\.yml/);
  assert.match(core,/needs\.classify\.outputs\.perf_core_changed == 'true'/);
  for(const line of ["aquilum-app/core/",".github/workflows/core-perf.yml",".github/workflows/core-perf-compare.yml"]){
    assert.ok(core.includes(line),'preserve previous path scope: '+line);
  }
  assert.match(core,/contains\(github\.event\.pull_request\.labels\.\*\.name, 'perf-ci'\)/);
  assert.match(perf,/startsWith\(github\.event\.pull_request\.title, 'perf:'\)/);
  assert.match(perf,/startsWith\(github\.head_ref, 'perf\/'\)/);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/,
    'performance measurements remain optional, never weaken Main PR gate');
});

test('beta packaging workflows do not build all three OS installers for any Native source edit',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const specs=[
    ['linux-beta.yml','scripts/linux-beta-smoke.sh','aquilum-app/native/packaging/akula-linux-nfpm.yaml'],
    ['windows-beta.yml','scripts/test-windows-beta.ps1','aquilum-app/native/packaging/akula-windows.json'],
    ['appimage-beta.yml','scripts/appimage-beta-smoke.sh','aquilum-app/native/packaging/akula-linux.json'],
  ];
  for(const [filename,ownedScript,ownedManifest] of specs){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',filename),'utf8');
    const policy=parse(body);
    assert.equal(policy.t,'paths',filename+' remains owned-path PR workflow');
    assert.equal(matches(policy,'aquilum-app/native/src/app/tests/graph.rs'),false,filename);
    assert.equal(matches(policy,'aquilum-app/native/src/updater/macos_transaction.rs'),false,filename);
    assert.equal(matches(policy,'aquilum-app/native/src/main.rs'),false,filename);
    for(const owned of [
      '.github/workflows/'+filename,
      'aquilum-app/native/Cargo.toml',ownedScript,ownedManifest,
    ])assert.equal(matches(policy,owned),true,filename+' retains '+owned);
    assert.match(body,/^  workflow_call:\s*$/m,filename+' still required via shared manifest');
    assert.match(body,/^  workflow_dispatch:\s*$/m,filename+' supports manual release candidate');
    assert.match(body,/^  pull_request:\s*$/m,filename+' watches packaging');
    assert.match(body,/^  [a-z-]+:\n    /m,filename+' retains job');
  }
  const main=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(main,/Native frame fidelity/);
  assert.match(main,/Run Native package tests for native-source changes/);
  assert.match(main,/workspace\|native\)\n\s+if \[ "\$FRAMES_RESULT" != success \]/,
    'a Native-only source PR is still blocked on full Native rendering tests');
});
