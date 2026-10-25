'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const core=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
const shared=fs.readFileSync(path.join(__dirname,'../workflows/ci-shared-manifest-labs.yml'),'utf8');
const routeEntries=[["host-custody","sync-host-custody.yml",["core","native"]],["b0c-ssh-native","core-server-bootstrap-b0c-windows-ssh.yml",["native","owned_b0c_ssh"]],["b1b2b-ubuntu","core-server-bootstrap-b1b2b-ubuntu.yml",["core"]],["b1c1-vm","core-server-bootstrap-b1c1-real-elf-vm.yml",["core"]],["b1b2a-systemd","core-server-bootstrap-b1b2a-systemd.yml",["core"]],["b1b3b-helper","core-server-bootstrap-b1b3b-root-helper.yml",["core"]],["b1b3c-fd","core-server-bootstrap-b1b3c-signed-fd.yml",["core","lib"]],["b1b4a-cas","core-server-bootstrap-b1b4a-cas-fence.yml",["core","lib"]],["b1b4b-witness","core-server-bootstrap-b1b4b-os-witness.yml",["core","lib"]],["b1b4b1-ipc","core-server-bootstrap-b1b4b1-witness-ipc.yml",["core"]],["b1c0-node","core-server-bootstrap-b1c0-fixed-node.yml",["core","lib"]],["b1d0-opt","core-server-bootstrap-b1d0-opt.yml",["core","lib"]],["b0b-ssh","core-server-bootstrap-b0b.yml",["core","lib"]],["b1b3a-ipc","core-server-bootstrap-b1b3a-ipc.yml",["core","lib"]],["p2p-offer","p2p-offer-lab.yml",["core","lib"]],["s3-minio","sync-s3-minio.yml",["core","lock","owned_s3_minio"]],["s3-platform","sync-s3-platform.yml",["core","lock","native"]],["pinned-read","sync-pinned-read.yml",["core","lock","owned_pinned_read"]],["s3-reservation","sync-reserved-s3-transaction.yml",["core","lock","owned_s3_reservation"]],["sync-protection","sync-protection.yml",["core","lock"]],["sync-transaction","sync-guarded-transaction.yml",["core","lock"]],["text-delta","sync-text-delta-rust.yml",["core","lock"]],["appimage-beta","appimage-beta.yml",["lock"]],["windows-beta","windows-beta.yml",["lock","owned_windows_beta"]],["vello-half-tie","native-vello-half-tie-regression.yml",["root","lock"]],["linux-beta","linux-beta.yml",["lock","owned_linux_beta"]],["installed-owner","sync-installed-os-owner-witness.yml",["core","owner"]],["device-roster","sty-device-roster-lab.yml",["core"]],["native-positive","sync-native-positive-host.yml",["native"]]];

test('29 owned routes preserve direct-only versus required callable owners',()=>{
  assert.equal(routeEntries.length,29);
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
    } else if(['host-custody','sync-protection','text-delta','appimage-beta','windows-beta','linux-beta'].includes(id)){
      assert.match(body,/^on:\n  workflow_call:/m);
      assert.doesNotMatch(body,/^  pull_request:/m);
    } else assert.match(body,/^on:\n  workflow_call:\n  pull_request:/m);
    const header=body.split(/\n(?:jobs:|permissions:|concurrency:|env:)/)[0];
    for(const p of ['aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock','aquilum-app/core/src/lib.rs','aquilum-app/Cargo.toml']){
      assert.ok(!header.includes("- '"+p+"'")&&!header.includes('- "'+p+'"'),file+' has no broad trigger '+p);
    }
    const group=body.match(/^  group: (.*)$/m)?.[1];
    if(group)assert.ok(group.includes('github.workflow'),file+' direct/called concurrency separated');
    if(!['installed-owner','host-custody','sync-protection','text-delta','appimage-beta','windows-beta','linux-beta'].includes(id))assert.match(body,new RegExp(file.replace(/\./g,'\\.'),'m'),file+' watches itself');
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
    assert.equal(policy.t,['appimage-beta.yml','windows-beta.yml','linux-beta.yml'].includes(filename)?'none':'paths',filename+' routed PR ownership');
    assert.equal(matches(policy,'aquilum-app/native/src/app/tests/graph.rs'),false,filename);
    assert.equal(matches(policy,'aquilum-app/native/src/updater/macos_transaction.rs'),false,filename);
    assert.equal(matches(policy,'aquilum-app/native/src/main.rs'),false,filename);
    for(const owned of [
      '.github/workflows/'+filename,
      'aquilum-app/native/Cargo.toml',ownedScript,ownedManifest,
    ])if(!['appimage-beta.yml','windows-beta.yml','linux-beta.yml'].includes(filename))assert.equal(matches(policy,owned),true,filename+' retains '+owned);
    assert.match(body,/^  workflow_call:\s*$/m,filename+' still required via shared manifest');
    if(filename==='windows-beta.yml')assert.match(body,/^  workflow_dispatch:\s*$/m,'Windows retains established manual candidate');
    else assert.doesNotMatch(body,/^  workflow_dispatch:\s*$/m,filename+' remains PR-only for unsigned release safety');
    if(!['appimage-beta.yml','windows-beta.yml','linux-beta.yml'].includes(filename))assert.match(body,/^  pull_request:\s*$/m,filename+' watches packaging');
    assert.match(body,/^  [a-z-]+:\n    /m,filename+' retains job');
  }
  const main=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(main,/Native frame fidelity/);
  assert.match(main,/Run Native package tests for native-source changes/);
  assert.match(main,/workspace\|native\)\n\s+if \[ "\$FRAMES_RESULT" != success \]/,
    'a Native-only source PR is still blocked on full Native rendering tests');
});

test('called OS/security workflows cannot mutually cancel through shared concurrency',()=>{
  const groups=new Map();
  for(const [,filename] of routeEntries){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',filename),'utf8');
    const group=body.match(/^  group: (.+)$/m)?.[1];
    if(!group)continue;
    assert.ok(!groups.has(group),filename+' collides with '+groups.get(group));
    groups.set(group,filename);
  }
  const ipc=fs.readFileSync(path.join(__dirname,'../workflows/core-server-bootstrap-b1b4b1-witness-ipc.yml'),'utf8');
  assert.match(ipc,/^  group: akula-b1b4b1-\$\{\{ github\.workflow \}\}-\$\{\{ github\.event\.pull_request\.number \|\| github\.run_id \}\}$/m);
  assert.match(ipc,/^  cancel-in-progress: true$/m);
  const core=fs.readFileSync(path.join(__dirname,'../workflows/app-checks.yml'),'utf8');
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});


test('Native Cargo metadata uses existing required S3 and host routes, never an independent duplicate',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const r=require('./ci-shared-manifest-route.cjs');
  const native=r.NATIVE;
  const changed=r.assess({paths:[native]});
  assert.equal(changed.native_manifest_risky,true,'Native feature/dependency changes keep S3');
  assert.equal(changed.installed_owner_owned,true,'Native feature/dependency changes keep host');
  const pure=r.assess({paths:[r.CORE,r.LOCK,r.NATIVE,r.WINDOWS],versionOnly:true});
  assert.equal(pure.native_manifest_risky,false,'pure bump exempts only validated dependency graph');
  assert.equal(pure.installed_owner_owned,false);
  const host=fs.readFileSync(path.join(__dirname,'../workflows/sync-host-custody.yml'),'utf8');
  assert.equal(parse(host).t,'none','host custody must have no parallel pull_request run');
  assert.match(host,/^  workflow_call:$/m,'required host callable retained');
  assert.doesNotMatch(host,/^  pull_request:$/m);
  for(const path of ['.github/workflows/sync-host-custody.yml','aquilum-app/core/src/app_core.rs']){
    assert.ok(r.OWNED_SUITES.owned_host_custody.includes(path),path+' retains source ownership');
    assert.equal(r.assess({paths:[path]}).owned_host_custody,true,path+' required route');
  }
  const s3=fs.readFileSync(path.join(__dirname,'../workflows/sync-s3-platform.yml'),'utf8');
  const s3Policy=parse(s3);
  assert.equal(matches(s3Policy,native),false,'S3 platform shared Native route is not duplicated');
  for(const path of ['.github/workflows/sync-s3-platform.yml','aquilum-app/core/src/app_core.rs'])
    assert.equal(matches(s3Policy,path),true,path+' retains standalone S3 proof');
  assert.match(s3,/^  workflow_call:$/m,'required S3 callable retained');
  assert.match(core,/native_manifest_risky: \$\{\{ steps\.scope\.outputs\.native_manifest_risky \}\}/);
  assert.match(core,/native: \$\{\{ needs\.classify\.outputs\.native_manifest_risky == 'true' \}\}/);
  assert.match(core,/SHARED_RISKY: .*native_manifest_risky/);
  assert.match(shared,/^      native:\n        required: true\n        type: boolean$/m);
  assert.match(shared,/route-host-custody:\n    name: Impacted shared manifest \/ host-custody\n    if: \$\{\{ inputs\.core \|\| inputs\.native \|\| inputs\.owned_host_custody \}\}/);
  assert.match(shared,/route-s3-platform:\n    name: Impacted shared manifest \/ s3-platform\n    if: \$\{\{ inputs\.core \|\| inputs\.lock \|\| inputs\.native \}\}/);
  assert.match(shared,/route-host-custody",\["owned_host_custody","core","native"\]/);
  assert.match(shared,/route-s3-platform",\["core","lock","native"\]/);
});


test('roster and positive-host ownership survives selective Cargo CI routing',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const manifest=require('./ci-shared-manifest-route.cjs');
  const expected=[
    ['sty-device-roster-lab.yml',manifest.CORE,'aquilum-app/core/src/sync/device_roster_lab.rs','device-roster','core'],
    ['sync-native-positive-host.yml',manifest.NATIVE,'aquilum-app/native/src/workspace.rs','native-positive','native'],
  ];
  for(const [name,broad,owned,id,input] of expected){
    const body=fs.readFileSync(path.join(__dirname,'../workflows',name),'utf8');
    const policy=parse(body);
    assert.equal(matches(policy,broad),false,name+' does not run independently on shared Cargo metadata');
    assert.equal(matches(policy,owned),true,name+' retains independently owned source trigger');
    assert.equal(matches(policy,'.github/workflows/'+name),true,name+' retains self-change trigger');
    assert.match(body,/^on:\n  workflow_call:\n  pull_request:/m,'mandatory caller and source-triggered direct lane retained');
    assert.match(body,/^  workflow_dispatch:$/m,'manual lab still available');
    assert.match(body,/^  group: [^\n]*github\.workflow/m,'direct and reusable runs have distinct concurrency groups');
    assert.match(body,/^  cancel-in-progress: \$\{\{ github\.event_name == 'pull_request' \}\}$/m,
      'manual labs are not cancelled by PR supersession');
    assert.ok(shared.includes('  route-'+id+':\n    name: Impacted shared manifest / '+id+
      '\n    if: $'+'{{ inputs.'+input+' }}\n    uses: ./.github/workflows/'+name),id+' required callable gate');
    assert.ok(shared.includes('["route-'+id+'",["'+input+'"]]'),id+' must be fail-closed');
    assert.match(shared,new RegExp('needs: \\[[^\\n]*route-'+id+'[^\\n]*\\]'),
      id+' must be part of the fail-closed boundary dependency list');
  }
  const native=manifest.assess({paths:[manifest.NATIVE]});
  assert.equal(native.native_manifest_risky,true,'real Native manifest edits must run positive host');
  const coreRisk=manifest.assess({paths:[manifest.CORE],
    coreDiff:'-version = "0.3.10"\n+version = "0.3.11"'});
  assert.equal(coreRisk.shared_core_cargo_risky,true,'unproven Core edits must run device roster');
  const exact=[manifest.CORE,manifest.LOCK,manifest.NATIVE,manifest.WINDOWS];
  const proven=manifest.assess({paths:exact,versionOnly:true});
  assert.equal(proven.native_manifest_risky,false);
  assert.equal(proven.shared_core_cargo_risky,false);
  assert.equal(proven.installed_owner_owned,false);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});

test('source path ownership remains in fail-closed shared route',()=>{const own=require('./ci-shared-manifest-route.cjs');for(const [k,v] of Object.entries(own.OWNED_SUITES)){assert.ok(v.length>0);assert.ok(shared.includes('inputs.'+k));assert.ok(shared.includes('"'+k+'"'));}});


test('Windows beta standalone and mixed source owners require exactly one signed-off CI route',()=>{
  const r=require('./ci-shared-manifest-route.cjs');
  const {parse}=require('./ci-file-ratchet.cjs');
  const body=fs.readFileSync(path.join(__dirname,'../workflows/windows-beta.yml'),'utf8');
  assert.equal(parse(body).t,'none','no parallel PR Windows installer');
  assert.match(body,/^on:\n  workflow_call:\n/m,'reusable caller stays active');
  assert.match(body,/^  workflow_dispatch:\n    inputs:\n      ref:\n/m,'manual candidate with ref stays active');
  assert.match(body,/^        default: main$/m);
  assert.doesNotMatch(body,/^  pull_request:/m);
  assert.equal(r.OWNED_SUITES.owned_windows_beta.length,10,'preserve all 10 former paths');
  const owned=['.github/workflows/windows-beta.yml',
    '.github/scripts/windows-beta-identity.test.cjs',
    'scripts/test-windows-beta.ps1','scripts/build-windows-update-fixture.ps1',
    'scripts/test-windows-beta-upgrade.ps1','scripts/windows-beta-gui-input.ps1',
    'scripts/windows-beta-candidate-manifest.ps1','scripts/windows-accesskit-uia.ps1',
    'aquilum-app/native/packaging/akula-windows.json','aquilum-app/native/Cargo.toml'];
  assert.deepEqual(r.OWNED_SUITES.owned_windows_beta,owned);
  for(const file of owned){
    assert.equal(r.assess({paths:[file]}).owned_windows_beta,true,file+' source-only routed');
    assert.equal(r.assess({paths:[file,r.LOCK]}).owned_windows_beta,true,file+' mixed Cargo routed');
  }
  assert.equal(r.assess({paths:['README.md']}).owned_windows_beta,false);
  assert.match(core,/owned_windows_beta: \$\{\{ steps\.scope\.outputs\.owned_windows_beta \}\}/);
  assert.match(core,/owned_windows_beta: \$\{\{ needs\.classify\.outputs\.owned_windows_beta == 'true' \}\}/);
  assert.match(core,/SHARED_RISKY: .*owned_windows_beta/);
  assert.match(shared,/route-windows-beta:\n    name: Impacted shared manifest \/ windows-beta\n    if: \$\{\{ inputs\.lock \|\| inputs\.owned_windows_beta \}\}/);
  assert.match(shared,/\"route-windows-beta\",\[\"lock\",\"owned_windows_beta\"\]/);
  assert.match(shared,/^      owned_windows_beta:\n        required: true\n        type: boolean$/m);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});


test('Linux beta source-only, mixed lock and workflow edits remain mandatory without duplicate PR jobs',()=>{
  const route=require('./ci-shared-manifest-route.cjs');
  const {parse}=require('./ci-file-ratchet.cjs');
  const body=fs.readFileSync(path.join(__dirname,'../workflows/linux-beta.yml'),'utf8');
  assert.equal(parse(body).t,'none','Linux packaging must not run as parallel PR workflow');
  assert.match(body,/^on:\n  workflow_call:\n/m);
  assert.doesNotMatch(body,/^  pull_request:$/m);
  assert.doesNotMatch(body,/^  workflow_dispatch:$/m,'candidate packaging must remain non-public');
  const paths=['.github/workflows/linux-beta.yml',
    '.github/scripts/linux-beta-identity.test.cjs',
    'scripts/linux-beta-smoke.sh',
    'aquilum-app/native/packaging/akula-linux-nfpm.yaml',
    'aquilum-app/native/packaging/Akula.desktop',
    'aquilum-app/native/Cargo.toml'];
  assert.deepEqual(route.OWNED_SUITES.owned_linux_beta,paths,'preserve exact six formerly direct paths');
  for(const path of paths){
    assert.equal(route.assess({paths:[path]}).owned_linux_beta,true,path+' source-only must call required route');
    assert.equal(route.assess({paths:[path,route.LOCK]}).owned_linux_beta,true,path+' mixed lock must call required route');
  }
  for(const path of ['README.md','aquilum-app/native/src/main.rs']){
    assert.equal(route.assess({paths:[path]}).owned_linux_beta,false,path+' must not package Linux');
  }
  assert.match(core,/owned_linux_beta: \$\{\{ steps\.scope\.outputs\.owned_linux_beta \}\}/);
  assert.match(core,/owned_linux_beta: \$\{\{ needs\.classify\.outputs\.owned_linux_beta == 'true' \}\}/);
  assert.match(core,/SHARED_RISKY: .*owned_linux_beta/);
  assert.match(shared,/route-linux-beta:\n    name: Impacted shared manifest \/ linux-beta\n    if: \$\{\{ inputs\.lock \|\| inputs\.owned_linux_beta \}\}/);
  assert.match(shared,/"route-linux-beta",\["lock","owned_linux_beta"\]/);
  assert.match(shared,/^      owned_linux_beta:\n        required: true\n        type: boolean$/m);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/,
    'Main PR gate remains fail closed on required Linux beta acceptance');
});


test('Pinned Read keeps 53 exact sources on both OS, main required and non-main direct only',()=>{
  const route=require('./ci-shared-manifest-route.cjs');
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const body=fs.readFileSync(path.join(__dirname,'../workflows/sync-pinned-read.yml'),'utf8');
  assert.match(body,/^on:\n  workflow_call:\n  pull_request:\n/m,'reusable plus direct non-main triggers remain');
  assert.match(body,/^    branches-ignore: \[main\]$/m,
    'main is excluded from independent direct pinned-read runs');
  assert.doesNotMatch(body,/^    branches:/m,'never accidentally restrict direct checks to main');
  assert.doesNotMatch(body,/^  workflow_dispatch:/m);
  assert.match(body,/matrix:\n        os: \[ubuntu-24\.04, windows-latest\]/m,
    'retain Linux and Windows proof');
  const direct=body.split('  pull_request:\n')[1]?.split('\npermissions:')[0]||'';
  assert.match(direct,/^    branches-ignore: \[main\]$/m);
  assert.doesNotMatch(direct,/^    paths-ignore:/m);
  const configured=[...direct.matchAll(/^\s+- "([^"]+)"$/gm)].map(x=>x[1]);
  const originalPaths=[
  "aquilum-app/core/src/plugins/vault_reader.rs",
  "aquilum-app/core/src/plugins/read_tests.rs",
  "aquilum-app/core/src/plugins/permission_broker.rs",
  "aquilum-app/core/src/sync/pinned_read.rs",
  "aquilum-app/core/src/sync/local_create.rs",
  "aquilum-app/core/src/sync/native_existing_replace_probe.rs",
  "aquilum-app/core/src/sync/native_windows_replace_probe.rs",
  "aquilum-app/core/src/sync/local_recovery.rs",
  "aquilum-app/core/src/sync/recovery_dashboard.rs",
  "aquilum-app/core/src/sync/recovery_existing_replace_tests.rs",
  "aquilum-app/core/src/sync/journal/existing_replace_attempt.rs",
  "aquilum-app/core/src/sync/recovery_review.rs",
  "aquilum-app/core/src/sync/sealed_archive.rs",
  "aquilum-app/core/src/sync/local_create_tests.rs",
  "aquilum-app/core/src/sync/journal.rs",
  "aquilum-app/core/src/sync/journal/sqlite_lifecycle.rs",
  "aquilum-app/core/src/sync/journal/publication_recovery.rs",
  "aquilum-app/core/src/sync/journal/phase_transitions.rs",
  "aquilum-app/core/src/sync/journal/tests/mod.rs",
  "aquilum-app/core/src/sync/journal/tests/recovery.rs",
  "aquilum-app/core/src/sync/core_scan.rs",
  "aquilum-app/core/src/sync/path.rs",
  "aquilum-app/core/src/search/paths.rs",
  "aquilum-app/core/src/sync/host_admission.rs",
  "aquilum-app/core/src/sync/host_admission/tests/mod.rs",
  "aquilum-app/core/src/sync/host_admission/tests/preimage.rs",
  "aquilum-app/core/src/sync/guarded_note_write.rs",
  "aquilum-app/core/src/app_core.rs",
  "aquilum-app/core/src/files/gate/write.rs",
  "aquilum-app/core/src/files/gate/moves.rs",
  "aquilum-app/core/src/files/gate/trash.rs",
  "aquilum-app/core/src/files/gate/mod.rs",
  "aquilum-app/core/src/files/trash.rs",
  "aquilum-app/core/src/history/store.rs",
  "aquilum-app/core/src/history/service.rs",
  "aquilum-app/core/src/history/recovery.rs",
  "aquilum-app/core/src/history/mod.rs",
  "aquilum-app/native/src/app/settings/apply.rs",
  "aquilum-app/native/src/app/settings/mod.rs",
  "aquilum-app/native/src/app/templates/starters.rs",
  "aquilum-app/core/src/files/gate/tests.rs",
  "aquilum-app/core/src/files/gate/interprocess_probe.rs",
  "aquilum-app/core/src/sync/filegate_lock_probe.rs",
  "aquilum-app/core/src/documents/hub.rs",
  "aquilum-app/core/src/documents/hub_tests.rs",
  "aquilum-app/core/src/documents/session.rs",
  "aquilum-app/core/src/documents/session_tests.rs",
  "aquilum-app/core/src/documents/store.rs",
  "aquilum-app/core/src/documents/store_tests.rs",
  "aquilum-app/core/src/documents/io.rs",
  "aquilum-app/core/src/sync/pinned_windows.rs",
  "aquilum-app/core/src/sync/mod.rs",
  ".github/workflows/sync-pinned-read.yml"
];
  assert.equal(originalPaths.length,53);
  assert.deepEqual(configured,originalPaths,'all former paths still trigger non-main PR acceptance');
  assert.deepEqual(route.OWNED_SUITES.owned_pinned_read,originalPaths,
    'all 53 paths also trigger mandatory main-branch OS checks');
  assert.equal(parse(body).t,'paths');
  for(const source of originalPaths){
    assert.equal(matches(parse(body),source),true,source+' direct for non-main PR');
    assert.equal(route.assess({paths:[source]}).owned_pinned_read,true,
      source+' source-only main branch coverage');
    assert.equal(route.assess({paths:[source,route.LOCK]}).owned_pinned_read,true,
      source+' mixed lock-and-source main branch coverage');
  }
  for(const unrelated of ['README.md','aquilum-app/native/src/main.rs']){
    assert.equal(route.assess({paths:[unrelated]}).owned_pinned_read,false,
      unrelated+' must not expand OS checks');
  }
  assert.match(core,/owned_pinned_read: \$\{\{ steps\.scope\.outputs\.owned_pinned_read \}\}/);
  assert.match(core,/owned_pinned_read: \$\{\{ needs\.classify\.outputs\.owned_pinned_read == 'true' \}\}/);
  assert.match(core,/SHARED_RISKY: .*owned_pinned_read/);
  assert.match(shared,/route-pinned-read:\n    name: Impacted shared manifest \/ pinned-read\n    if: \$\{\{ inputs\.core \|\| inputs\.lock \|\| inputs\.owned_pinned_read \}\}/);
  assert.match(shared,/"route-pinned-read",\["core","lock","owned_pinned_read"\]/);
  assert.match(shared,/^      owned_pinned_read:\n        required: true\n        type: boolean$/m);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
  assert.match(body,/^  group: pinned-read-.*github\.workflow/m,
    'direct/called runs retain original concurrency separation');
});


test('Reserved S3 OS acceptance preserves all 14 glob owners for main and non-main targets',()=>{
  const owned=require('./ci-shared-manifest-route.cjs');
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const body=fs.readFileSync(path.join(__dirname,'../workflows/sync-reserved-s3-transaction.yml'),'utf8');
  assert.match(body,/^on:\n  workflow_call:\n  pull_request:\n/m);
  assert.match(body,/^    branches-ignore: \[main\]$/m,'only non-main PRs run direct');
  assert.doesNotMatch(body,/^    branches:/m,'never restrict direct checks to main');
  assert.match(body,/^  workflow_dispatch:$/m,'existing manual QA preserved');
  assert.match(body,/os: \[ubuntu-24\.04, windows-latest\]/,'both OS proof lanes retained');
  assert.match(body,/^  cancel-in-progress: true$/m,'keep original cancellation policy');
  const expected=[
  "aquilum-app/core/src/sync/s3_reserved_replace*.rs",
  "aquilum-app/core/src/sync/guarded_note_write.rs",
  "aquilum-app/core/src/sync/guarded_note_write/tests/**",
  "aquilum-app/core/src/sync/profile_binding.rs",
  "aquilum-app/core/src/sync/s3_preimage_capture.rs",
  "aquilum-app/core/src/sync/pinned_read.rs",
  "aquilum-app/core/src/sync/journal/**",
  "aquilum-app/core/src/sync/mod.rs",
  "aquilum-app/core/src/documents/hub.rs",
  "aquilum-app/core/src/files/gate/write.rs",
  "aquilum-app/core/src/files/gate/mod.rs",
  ".github/workflows/sync-reserved-s3-transaction.yml",
  "docs/sync/p0g2-held-s3-editor-reservation.md",
  "docs/sync/p0g3a-single-filegate-prewrite.md"
];
  assert.equal(expected.length,14,'original 14 path filters remain intact');
  assert.equal(expected.filter(p=>p.includes('*')).length,3,'all three glob owners preserved');
  assert.deepEqual(parse(body),{t:'paths',p:expected});
  assert.deepEqual(owned.OWNED_SUITES.owned_s3_reservation,expected,
    'required Main PR route matches former direct owners exactly');
  for(const source of expected){
    assert.equal(matches(parse(body),source),true,'direct owner preserved '+source);
    assert.equal(owned.assess({paths:[source]}).owned_s3_reservation,true,
      'required route for source-only owner '+source);
    assert.equal(owned.assess({paths:[source,owned.LOCK]}).owned_s3_reservation,true,
      'required route for mixed source+Cargo change '+source);
  }
  const examples=[
    'aquilum-app/core/src/sync/s3_reserved_replace_test.rs',
    'aquilum-app/core/src/sync/s3_reserved_replace_windows.rs',
    'aquilum-app/core/src/sync/guarded_note_write/tests/state.rs',
    'aquilum-app/core/src/sync/guarded_note_write/tests/deep/state.rs',
    'aquilum-app/core/src/sync/journal/record.rs',
    'aquilum-app/core/src/sync/journal/recovery/resume.rs'
  ];
  for(const file of examples){
    assert.equal(matches(parse(body),file),true,'workflow owns glob match '+file);
    assert.equal(owned.assess({paths:[file]}).owned_s3_reservation,true,
      'Main PR required owner must match GitHub pattern '+file);
    assert.equal(owned.assess({paths:[file,owned.LOCK]}).owned_s3_reservation,true,
      'mixed lock-and-glob must route '+file);
  }
  for(const file of [
    'README.md',
    'aquilum-app/native/src/main.rs',
    'aquilum-app/core/src/sync/s3_reserved_other.rs',
    'aquilum-app/core/src/sync/guarded_note_write/test_extra.rs',
    'aquilum-app/core/src/sync/journalized.rs'
  ]){
    assert.equal(matches(parse(body),file),false,'workflow not owner of '+file);
    assert.equal(owned.assess({paths:[file]}).owned_s3_reservation,false,
      'unrelated source cannot activate extra S3 checks '+file);
  }
  assert.match(core,/owned_s3_reservation: \$\{\{ steps\.scope\.outputs\.owned_s3_reservation \}\}/);
  assert.match(core,/owned_s3_reservation: \$\{\{ needs\.classify\.outputs\.owned_s3_reservation == 'true' \}\}/);
  assert.match(core,/SHARED_RISKY: .*owned_s3_reservation/);
  assert.match(shared,/route-s3-reservation:\n    name: Impacted shared manifest \/ s3-reservation\n    if: \$\{\{ inputs\.core \|\| inputs\.lock \|\| inputs\.owned_s3_reservation \}\}/);
  assert.match(shared,/"route-s3-reservation",\["core","lock","owned_s3_reservation"\]/);
  assert.match(shared,/^      owned_s3_reservation:\n        required: true\n        type: boolean$/m);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/,
    'Main PR gate must still depend on routed S3 checks');
});


test('S3 MinIO preserves 30 direct non-main source paths and required main glob routing',()=>{
  const owned=require('./ci-shared-manifest-route.cjs');
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const body=fs.readFileSync(path.join(__dirname,'../workflows/sync-s3-minio.yml'),'utf8');
  const oldPaths=[
  "aquilum-app/core/src/sync/s3_transport.rs",
  "aquilum-app/core/tests/aws_credentials_regression.rs",
  "aquilum-app/core/src/sync/yandex_readonly_probe.rs",
  "aquilum-app/core/src/sync/profile_binding.rs",
  "aquilum-app/core/src/sync/host_admission.rs",
  "aquilum-app/core/src/sync/host_admission/tests/mod.rs",
  "aquilum-app/core/src/sync/host_admission/tests/preimage.rs",
  "aquilum-app/core/src/sync/s3_preimage_capture.rs",
  "aquilum-app/core/src/sync/host_custody_registry.rs",
  "aquilum-app/core/src/sync/journal.rs",
  "aquilum-app/core/src/sync/journal/sqlite_lifecycle.rs",
  "aquilum-app/core/src/sync/journal/publication_recovery.rs",
  "aquilum-app/core/src/sync/journal/phase_transitions.rs",
  "aquilum-app/core/src/sync/journal/tests/mod.rs",
  "aquilum-app/core/src/sync/journal/tests/recovery.rs",
  "aquilum-app/core/src/sync/local_create_tests.rs",
  "aquilum-app/core/src/sync/local_create.rs",
  "aquilum-app/core/src/sync/s3_transport/**",
  "aquilum-app/core/src/sync/s3.rs",
  "aquilum-app/core/src/sync/s3_manifest.rs",
  "aquilum-app/core/src/sync/s3_manifest_delta.rs",
  "aquilum-app/core/src/sync/s3_coord_v2.rs",
  "aquilum-app/core/src/sync/s3_head_probe.rs",
  "aquilum-app/core/src/sync/s3_delta_walker.rs",
  "aquilum-app/core/src/sync/s3_remote_anchor.rs",
  "aquilum-app/core/src/sync/s3_transport/selective_snapshot.rs",
  "aquilum-app/core/src/sync/s3_transport/offline_retention.rs",
  "aquilum-app/core/src/sync/s3_anchored_delta.rs",
  "aquilum-app/core/src/sync/mod.rs",
  ".github/workflows/sync-s3-minio.yml"
];
  assert.equal(oldPaths.length,30,'full original source path coverage');
  assert.deepEqual(oldPaths.filter(p=>p.includes('*')),
    ['aquilum-app/core/src/sync/s3_transport/**'],'preserve exact nested transport glob');
  assert.match(body,/^on:\n  workflow_call:\n  pull_request:\n/m);
  assert.match(body,/^    branches-ignore: \[main\]$/m,'no parallel S3 MinIO workflow on PR targeting main');
  assert.doesNotMatch(body,/^    branches: \[main\]$/m);
  assert.match(body,/^  workflow_dispatch:$/m,'manual disposable MinIO probe remains available');
  assert.match(body,/^\s+runs-on: ubuntu-24\.04$/m);
  assert.match(body,/^  group: sync-s3-minio-.*github\.workflow/m,'original concurrency preserved');
  assert.match(body,/AQ_S3_TEST_ENDPOINT: http:\/\/127\.0\.0\.1:9000/,'retain loopback-only MinIO');
  assert.match(body,/docker run -d --rm --name akula-s3-minio/,'retain disposable container');
  assert.deepEqual(parse(body),{t:'paths',p:oldPaths},
    'non-main pull_request.paths must retain all original patterns');
  assert.deepEqual(owned.OWNED_SUITES.owned_s3_minio,oldPaths,
    'required Main route owns exact same 30 path patterns');
  for(const entry of oldPaths){
    assert.equal(matches(parse(body),entry),true,entry+' direct non-main match');
    assert.equal(owned.assess({paths:[entry]}).owned_s3_minio,true,entry+' required main source-only');
    assert.equal(owned.assess({paths:[entry,owned.LOCK]}).owned_s3_minio,true,entry+' required mixed Cargo');
  }
  for(const entry of [
    'aquilum-app/core/src/sync/s3_transport/receipt.rs',
    'aquilum-app/core/src/sync/s3_transport/deep/retained.rs',
    'aquilum-app/core/src/sync/s3_transport/tests/deep/identity.rs'
  ]){
    assert.equal(matches(parse(body),entry),true,entry+' direct glob for non-main');
    assert.equal(owned.assess({paths:[entry]}).owned_s3_minio,true,entry+' required glob for main');
    assert.equal(owned.assess({paths:[entry,owned.LOCK]}).owned_s3_minio,true,entry+' mixed glob and Cargo');
  }
  for(const entry of [
    'README.md','aquilum-app/native/src/main.rs',
    'aquilum-app/core/src/sync/s3_transport_extra.rs',
    'aquilum-app/core/src/sync/s3_transportable/receipt.rs'
  ]){
    assert.equal(matches(parse(body),entry),false,entry+' not direct S3 owner');
    assert.equal(owned.assess({paths:[entry]}).owned_s3_minio,false,entry+' not required S3 owner');
  }
  assert.match(core,/owned_s3_minio: \$\{\{ steps\.scope\.outputs\.owned_s3_minio \}\}/);
  assert.match(core,/owned_s3_minio: \$\{\{ needs\.classify\.outputs\.owned_s3_minio == 'true' \}\}/);
  assert.match(core,/SHARED_RISKY: .*owned_s3_minio/);
  assert.match(shared,/route-s3-minio:\n    name: Impacted shared manifest \/ s3-minio\n    if: \$\{\{ inputs\.core \|\| inputs\.lock \|\| inputs\.owned_s3_minio \}\}/);
  assert.match(shared,/"route-s3-minio",\["core","lock","owned_s3_minio"\]/);
  assert.match(shared,/^      owned_s3_minio:\n        required: true\n        type: boolean$/m);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/,
    'Main PR gate must still fail closed for MinIO');
});

test('B0c Windows owned sources run only through required main route; non-main and manual remain',()=>{
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const router=require('./ci-shared-manifest-route.cjs');
  const file='.github/workflows/core-server-bootstrap-b0c-windows-ssh.yml';
  const body=fs.readFileSync(path.join(__dirname,'../workflows/core-server-bootstrap-b0c-windows-ssh.yml'),'utf8');
  const policy=parse(body);
  assert.match(body,/^on:\n  workflow_call:\n  pull_request:\n    branches-ignore: \[main\]\n    paths:/m);
  assert.equal(matches(policy,router.NATIVE),false,'native manifest must not launch directly');
  const owners=[
    "aquilum-app/core/src/server_bootstrap_ssh_windows_lab.rs",
    "aquilum-app/core/src/server_bootstrap_ssh_windows_lab/scan.rs",
    "aquilum-app/core/src/server_bootstrap_modules.rs",
    "aquilum-app/core/tests/server_bootstrap_b0c_windows.rs",
    "scripts/ci/akula-b0c-windows-loopback-sshd.py",
    ".github/workflows/core-server-bootstrap-b0c-windows-ssh.yml",
    "docs/platforms/AKULA-SERVER-BOOTSTRAP-B0C.md",
    "aquilum-app/native/src/app/settings/server.rs",
    "aquilum-app/native/src/app/settings/server_state.rs",
    "aquilum-app/native/src/ui/settings/schema/server/server_status.rs",
    "aquilum-app/native/src/ui/settings/schema/server/identity.rs",
    "aquilum-app/native/assets/locales/ru.json",
    "aquilum-app/native/assets/locales/en.json",
    "aquilum-app/native/src/ui/settings/schema/tests.rs",
  ];
  assert.deepEqual(router.OWNED_SUITES.owned_b0c_ssh,owners,'exact previous 14 source owners');
  const direct=body.split('    paths:\n')[1]?.split('  workflow_dispatch:')[0]||'';
  const directOwners=[...direct.matchAll(/^      - "([^"]+)"$/gm)].map(m=>m[1]);
  assert.deepEqual(directOwners,owners,'non-main PRs retain all source triggers');
  assert.match(body,/^  workflow_dispatch:/m,'manual Windows lab remains');
  for(const owner of owners){
    assert.equal(matches(policy,owner),true,owner+' still directly tested on non-main');
    assert.equal(router.assess({paths:[owner]}).owned_b0c_ssh,true,owner+' required on main');
    assert.equal(router.assess({paths:[owner,router.NATIVE]}).owned_b0c_ssh,true,owner+' mixed diff required');
  }
  assert.equal(router.assess({paths:['README.md']}).owned_b0c_ssh,false);
  assert.equal(router.assess({paths:[router.NATIVE]}).owned_b0c_ssh,false);
  assert.ok(core.includes("owned_b0c_ssh: ${{ steps.scope.outputs.owned_b0c_ssh }}"),'classifier exports B0c owner');
  assert.ok(core.includes("owned_b0c_ssh: ${{ needs.classify.outputs.owned_b0c_ssh == 'true' }}"),'required B0c input wired');
  assert.match(core,/SHARED_RISKY: .*owned_b0c_ssh/);
  assert.match(shared,/^      owned_b0c_ssh:\n        required: true\n        type: boolean$/m);
  assert.ok(shared.includes("if: ${{ inputs.native || inputs.owned_b0c_ssh }}"),'one callable B0c route for native risk or source owners');
  assert.match(shared,/\["route-b0c-ssh-native",\["native","owned_b0c_ssh"\]\]/);
  assert.match(shared,/needs: \[[^\n]*route-b0c-ssh-native[^\n]*\]/);
  assert.match(core,/needs: \[classify, required-tests, native-frames, shared-labs\]/);
});

test('extracted journal publication/recovery retains fail-closed OS and S3 owners',()=>{
  const route=require('./ci-shared-manifest-route.cjs');
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const p='aquilum-app/core/src/sync/journal/publication_recovery.rs';
  const child=fs.readFileSync(path.join(__dirname,'../../'+p),'utf8');
  const root=fs.readFileSync(path.join(__dirname,'../../aquilum-app/core/src/sync/journal.rs'),'utf8');
  assert.ok(root.includes('mod publication_recovery;'));
  assert.ok(child.startsWith('use super::*;'));
  assert.ok(child.includes('impl SqliteSyncJournal {'));
  const methods=["record_local_publication_attempt_once","record_native_local_publication_observation_once","read_native_local_publication_observation","local_publication_count","local_publication_review_queue","check_local_recovery_case","record_local_recovery_case_once","read_local_recovery_case","require_appdata_outside_vault","validated_forensic_digest","record_local_recovery_copy_once","read_local_recovery_copy","recovery_review_snapshot","read_recovery_review_rows","recovery_review_history","append_recovery_review"];
  for(const name of methods){
    assert.ok(child.includes('fn '+name+'('),name+' survives');
    assert.ok(!root.includes('fn '+name+'('),name+' not duplicated');
  }
  const routes=[['owned_pinned_read','sync-pinned-read.yml'],
    ['owned_s3_minio','sync-s3-minio.yml'],
    ['owned_s3_reservation','sync-reserved-s3-transaction.yml']];
  for(const [owner,file] of routes){
    const workflow=fs.readFileSync(path.join(__dirname,'../workflows',file),'utf8');
    assert.equal(matches(parse(workflow),p),true,file+' non-main source route');
    assert.equal(route.assess({paths:[p]})[owner],true,owner+' main required');
    assert.equal(route.assess({paths:[p,route.LOCK]})[owner],true,owner+' mixed lock required');
  }
  assert.ok(core.includes('needs: [classify, required-tests, native-frames, shared-labs]'));
  assert.equal(route.assess({paths:['README.md']}).owned_s3_minio,false);
});

test('extracted journal phase transitions retain public APIs and all protected source routes',()=>{
  const route=require('./ci-shared-manifest-route.cjs');
  const {parse,matches}=require('./ci-file-ratchet.cjs');
  const p='aquilum-app/core/src/sync/journal/phase_transitions.rs';
  const source=fs.readFileSync(path.join(__dirname,'../../'+p),'utf8');
  const root=fs.readFileSync(path.join(__dirname,'../../aquilum-app/core/src/sync/journal.rs'),'utf8');
  assert.ok(source.startsWith('use super::*;'));
  assert.ok(source.includes('impl SqliteSyncJournal {'));
  assert.ok(root.includes('mod phase_transitions;'));
  for(const name of ['begin','mark_staged','mark_applied','mark_verified','commit','recovery_queue']){
    assert.ok(source.includes('pub fn '+name+'('),'public method preserved: '+name);
    assert.ok(!root.includes('pub fn '+name+'('),'no duplicate method: '+name);
  }
  for(const [suite,file] of [
    ['owned_pinned_read','sync-pinned-read.yml'],
    ['owned_s3_minio','sync-s3-minio.yml'],
    ['owned_s3_reservation','sync-reserved-s3-transaction.yml']]){
    const yaml=fs.readFileSync(path.join(__dirname,'../workflows',file),'utf8');
    assert.ok(yaml.includes('branches-ignore: [main]'),'non-main direct route: '+file);
    assert.equal(matches(parse(yaml),p),true,'direct source: '+file);
    assert.equal(route.assess({paths:[p]})[suite],true,'main required route: '+suite);
    assert.equal(route.assess({paths:[p,route.LOCK]})[suite],true,'mixed Cargo and source: '+suite);
  }
  assert.ok(core.includes('needs: [classify, required-tests, native-frames, shared-labs]'));
});
