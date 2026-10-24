'use strict';
const {execFileSync}=require('node:child_process');
const {matches: matchesWorkflowPaths}=require('./ci-file-ratchet.cjs');
const CORE='aquilum-app/core/Cargo.toml';
const LOCK='aquilum-app/Cargo.lock';
const LIB='aquilum-app/core/src/lib.rs';
const ROOT='aquilum-app/Cargo.toml';
const NATIVE='aquilum-app/native/Cargo.toml';
const WINDOWS='aquilum-app/native/packaging/akula-windows.json';
const VERSION_ONLY_PATHS=new Set([CORE,LOCK,NATIVE,WINDOWS]);
// A version bump changes package identity strings, not the Rust dependency
// graph. Require byte-for-byte equality of every other manifest/lock field.
function versionAt(body,marker){
  if(typeof body!=='string'||body.split(marker).length!==2)return null;
  const m=/^((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))"/.exec(body.split(marker)[1]);
  if(!m||!m[1].split('.').every(x=>Number.isSafeInteger(Number(x))))return null;
  return m[1];
}
function versionIncreases(oldV,newV){
  const a=oldV.split('.').map(Number),b=newV.split('.').map(Number);
  for(let i=0;i<3;i++)if(a[i]!==b[i])return b[i]>a[i];
  return false;
}
function versionBumpOnly({paths,coreBefore,coreAfter,nativeBefore,nativeAfter,lockBefore,lockAfter,windowsBefore,windowsAfter}){
  if(!Array.isArray(paths)||paths.length!==4||new Set(paths).size!==4||
     paths.some(p=>!VERSION_ONLY_PATHS.has(p)))return false;
  const core='[package]\nname = "aquilum-core"\nversion = "';
  const native='[package]\nname = "aquilum-native"\nversion = "';
  const lockCore='[[package]]\nname = "aquilum-core"\nversion = "';
  const lockNative='[[package]]\nname = "aquilum-native"\nversion = "';
  const windows='"version": "';
  const oldV=versionAt(coreBefore,core),newV=versionAt(coreAfter,core);
  if(!oldV||!newV||!versionIncreases(oldV,newV))return false;
  for(const [before,after,marker] of [
    [nativeBefore,nativeAfter,native],
    [lockBefore,lockAfter,lockCore],
    [lockBefore,lockAfter,lockNative],
    [windowsBefore,windowsAfter,windows],
  ]){
    if(versionAt(before,marker)!==oldV||versionAt(after,marker)!==newV)return false;
  }
  const swap=(s,marker)=>s.replace(marker+oldV+'"',marker+newV+'"');
  return swap(coreBefore,core)===coreAfter&&
    swap(nativeBefore,native)===nativeAfter&&
    swap(swap(lockBefore,lockCore),lockNative)===lockAfter&&
    swap(windowsBefore,windows)===windowsAfter&&
    windowsBefore.includes('"productName": "Akula"');
}
// Owner-only OS acceptance always runs for its own sources, not for every shared manifest.
const INSTALLED_OWNER_PATHS=new Set(["aquilum-app/core/src/sync/host_install_unix_root.rs","aquilum-app/core/src/sync/host_install_witness.rs","aquilum-app/core/src/sync/mod.rs","aquilum-app/native/Cargo.toml",".github/workflows/sync-installed-os-owner-witness.yml","docs/sync/adr-0038-unix-root-owned-install-receipt.md"]);

// Preserve the exact former pull_request.path owners. These source-owned labs
// run through the required Main PR gate, including mixed source/manifest diffs.
const OWNED_SUITES=Object.freeze({
  "owned_host_custody": [
    "aquilum-app/core/src/app_core.rs",
    "aquilum-app/core/src/app_core_installed_host_tests.rs",
    "aquilum-app/core/src/files/gate/write.rs",
    "aquilum-app/core/src/history/service.rs",
    "aquilum-app/core/src/sync/mod.rs",
    "aquilum-app/core/src/sync/host_custody_registry.rs",
    "aquilum-app/core/src/sync/host_admission.rs",
    "aquilum-app/core/src/sync/local_create_tests.rs",
    "aquilum-app/core/src/sync/host_ledger_identity.rs",
    "aquilum-app/core/src/sync/host_cold_identity_tests.rs",
    "aquilum-app/core/src/sync/host_install_witness.rs",
    "aquilum-app/core/src/app_core_witness_tests.rs",
    "aquilum-app/core/src/sync/host_core_epoch_tests.rs",
    "aquilum-app/native/src/workspace.rs",
    "aquilum-app/native/src/workspace_installed_witness_tests.rs",
    ".github/workflows/sync-host-custody.yml"
  ],
  "owned_sync_protection": [
    "aquilum-app/core/src/sync/protection_store.rs",
    "aquilum-app/core/src/sync/mod.rs",
    ".github/workflows/sync-protection.yml"
  ],
  "owned_text_delta": [
    "aquilum-app/core/src/sync/text_delta_codec.rs",
    "aquilum-app/core/src/sync/mod.rs",
    "aquilum-app/core/src/sync/s3_transport/pack_probe.rs",
    ".github/workflows/sync-text-delta-rust.yml",
    "docs/sync/text-delta-03-rust-codec.md"
  ],
  "owned_appimage": [
    ".github/workflows/appimage-beta.yml",
    ".github/scripts/appimage-beta-identity.test.cjs",
    "scripts/appimage-beta-smoke.sh",
    "scripts/appimage-real-upgrade-accept.py",
    "scripts/appimage-real-upgrade-helper.rs",
    "aquilum-app/native/packaging/akula-linux.json",
    "aquilum-app/native/packaging/Akula.desktop",
    "aquilum-app/native/Cargo.toml"
  ],
  "owned_windows_beta": [
    ".github/workflows/windows-beta.yml",
    ".github/scripts/windows-beta-identity.test.cjs",
    "scripts/test-windows-beta.ps1",
    "scripts/build-windows-update-fixture.ps1",
    "scripts/test-windows-beta-upgrade.ps1",
    "scripts/windows-beta-gui-input.ps1",
    "scripts/windows-beta-candidate-manifest.ps1",
    "scripts/windows-accesskit-uia.ps1",
    "scripts/windows-p2p-installed-uia.ps1",
    "aquilum-app/native/packaging/akula-windows.json",
    "aquilum-app/native/Cargo.toml"
  ],
  "owned_linux_beta": [
    ".github/workflows/linux-beta.yml",
    ".github/scripts/linux-beta-identity.test.cjs",
    "scripts/linux-beta-smoke.sh",
    "aquilum-app/native/packaging/akula-linux-nfpm.yaml",
    "aquilum-app/native/packaging/Akula.desktop",
    "aquilum-app/native/Cargo.toml"
  ],
  "owned_pinned_read": [
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
    "aquilum-app/core/src/sync/core_scan.rs",
    "aquilum-app/core/src/sync/path.rs",
    "aquilum-app/core/src/search/paths.rs",
    "aquilum-app/core/src/sync/host_admission.rs",
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
  ],
  "owned_s3_reservation": [
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
  ],
  "owned_s3_minio": [
    "aquilum-app/core/src/sync/s3_transport.rs",
    "aquilum-app/core/tests/aws_credentials_regression.rs",
    "aquilum-app/core/src/sync/yandex_readonly_probe.rs",
    "aquilum-app/core/src/sync/profile_binding.rs",
    "aquilum-app/core/src/sync/host_admission.rs",
    "aquilum-app/core/src/sync/s3_preimage_capture.rs",
    "aquilum-app/core/src/sync/host_custody_registry.rs",
    "aquilum-app/core/src/sync/journal.rs",
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
  ]
});
function ownedRoutes(paths){
  const changed=new Set(paths);
  return Object.fromEntries(Object.entries(OWNED_SUITES).map(([key,owned])=>
    [key,[...changed].some(file=>matchesWorkflowPaths({t:'paths',p:owned},file))]));
}
function changes(diff){
  return String(diff||'').split(/\r?\n/)
    .filter(x=>(x.startsWith('+')&&!x.startsWith('+++'))||(x.startsWith('-')&&!x.startsWith('---')))
    .filter(x=>!/^[-+]\s*(?:#(?!\[).*|)$/.test(x));
}
function cargoAdditive(diff){
  const lines=changes(diff),added=[],removed=[];
  for(const row of lines)(row[0]==='+'?added:removed).push(row.slice(1).trim());
  const packageNames=new Set(),features=new Set();
  const pathDep=/^([a-z][a-z0-9-]*)\s*=\s*\{\s*path\s*=\s*"\.\.\/\.\.\/modules\/[a-z][a-z0-9-]*"\s*,\s*optional\s*=\s*true\s*\}$/;
  const optFeature=/^([a-z][a-z0-9-]*)\s*=\s*\[\s*"dep:([a-z][a-z0-9-]*)"\s*\]$/;
  const emptyFeature=/^([a-z][a-z0-9-]*)\s*=\s*\[\s*\]$/;
  for(const line of added){
    let m=pathDep.exec(line);
    if(m){if(packageNames.has(m[1]))return {safe:false};packageNames.add(m[1]);continue;}
    m=optFeature.exec(line);
    if(m){if(features.has(m[1]))return {safe:false};features.add(m[1]);continue;}
    m=emptyFeature.exec(line);
    if(m&&/(?:-lab|-probe)$/.test(m[1])){
      if(features.has(m[1]))return {safe:false};features.add(m[1]);continue;
    }
    return {safe:false};
  }
  for(const line of removed){
    const m=emptyFeature.exec(line);
    if(!m||!features.has(m[1])||!added.some(a=>optFeature.test(a)&&optFeature.exec(a)[1]===m[1]))return {safe:false};
  }
  const referenced=added.map(line=>optFeature.exec(line)).filter(Boolean).map(x=>x[2]);
  if(packageNames.size!==new Set(referenced).size || [...packageNames].some(p=>!referenced.includes(p)))return {safe:false};
  if(referenced.some(p=>!packageNames.has(p)))return {safe:false};
  return {safe:true,packages:packageNames,features};
}
function blocks(raw){
  const out=new Map();
  const sections=String(raw||'').split(/\n(?=\[\[package\]\]\n)/);
  if(!sections[0].includes('version = 4'))return null;
  for(const section of sections){
    if(!section.startsWith('[[package]]\n'))continue;
    const name=/^name = "([^"]+)"$/m.exec(section)?.[1];
    const version=/^version = "([^"]+)"$/m.exec(section)?.[1];
    if(!name||!version)return null;
    const key=name+'@'+version;
    if(out.has(key))return null;
    out.set(key,section);
  }
  return out;
}
function deps(block){
  const m=/^dependencies = \[\n((?: [^\n]*\n)*)\]/m.exec(block);
  return m?m[1].split('\n').filter(Boolean).map(x=>/^ "([^"]+)",$/.exec(x)?.[1]):null;
}
function lockAdditive(before,after,addedPackages){
  const old=blocks(before),now=blocks(after);
  if(!old||!now||!old.size||!now.size)return false;
  const added=[];
  for(const [key,val] of now){
    if(!old.has(key))added.push([key,val]);
    else if(old.get(key)!==val){
      if(!key.startsWith('aquilum-core@'))return false;
      const oldBlock=old.get(key),x=deps(oldBlock),y=deps(val);
      if(!x||!y||x.some(z=>!y.includes(z))||new Set(y).size!==y.length)return false;
      const extra=y.filter(z=>!x.includes(z));
      if(extra.length!==addedPackages.size||extra.some(z=>!addedPackages.has(z)))return false;
      const scrub=s=>s.replace(/^dependencies = \[\n(?: [^\n]*\n)*\]/m,'dependencies = [REDACTED]');
      if(scrub(oldBlock)!==scrub(val))return false;
    }
  }
  if([...old.keys()].some(k=>!now.has(k)))return false;
  if(added.length!==addedPackages.size)return false;
  for(const [key,val] of added){
    const name=key.slice(0,key.lastIndexOf('@'));
    if(!addedPackages.has(name)||!key.endsWith('@0.1.0')||/^source = |^checksum = /m.test(val))return false;
    if(!/^\[\[package\]\]\nname = "[^"]+"\nversion = "0\.1\.0"\ndependencies = \[\n(?: "[^"]+",\n)*\]\n?$/m.test(val.trim()))return false;
  }
  if(added.length&&old.get([...old.keys()].find(k=>k.startsWith('aquilum-core@')))===now.get([...old.keys()].find(k=>k.startsWith('aquilum-core@'))))return false;
  return true;
}
function libAdditive(diff,features){
  const rows=changes(diff);
  if(rows.length%2!==0||rows.some(x=>!x.startsWith('+')))return false;
  for(let i=0;i<rows.length;i+=2){
    const feature=/^\+\s*#\[cfg\(feature = "([a-z][a-z0-9-]*)"\)\]$/.exec(rows[i]);
    const module=/^\+\s*pub mod ([a-z][a-z0-9_]*)\s*;$/.exec(rows[i+1]);
    if(!feature||!module||!features.has(feature[1])||module[1]!==feature[1].replaceAll('-','_'))return false;
  }
  return true;
}
function assess({paths,coreDiff='',lockBefore='',lockAfter='',libDiff='',versionOnly=false}){
  const changed=new Set(paths);
  const metadataOnly=versionOnly&&paths.length===4&&changed.size===4&&
    [...changed].every(p=>VERSION_ONLY_PATHS.has(p));
  const cargo=changed.has(CORE)?cargoAdditive(coreDiff):{safe:true,packages:new Set(),features:new Set()};
  const lockSafe=changed.has(LOCK)&&changed.has(CORE)&&cargo.safe&&lockAdditive(lockBefore,lockAfter,cargo.packages);
  const coreRisk=changed.has(CORE)&&!metadataOnly&&(!cargo.safe||(changed.has(LOCK)&&!lockSafe));
  const lockRisk=changed.has(LOCK)&&!metadataOnly&&!lockSafe;
  const libRisk=changed.has(LIB)&&!(changed.has(CORE)&&cargo.safe&&libAdditive(libDiff,cargo.features));
  const rootRisk=changed.has(ROOT);
  const installedOwnerOwned=!metadataOnly&&[...changed].some(p=>INSTALLED_OWNER_PATHS.has(p));
  const native_manifest_risky=changed.has(NATIVE)&&!metadataOnly;
  return {shared_core_cargo_risky:coreRisk,shared_lock_risky:lockRisk,shared_core_lib_risky:libRisk,shared_root_cargo_risky:rootRisk,
    shared_any_risky:coreRisk||lockRisk||libRisk||rootRisk,
    installed_owner_owned:installedOwnerOwned,native_manifest_risky,...ownedRoutes(paths)};
}
function git(...args){return execFileSync('git',args,{encoding:'utf8',maxBuffer:16*1024*1024});}
function compute(base,head){
  if(!/^[0-9a-f]{40}$/.test(base)||!(head==='HEAD'||/^[0-9a-f]{40}$/.test(head)))throw Error('bad base/head SHA');
  const names=git('diff','--name-only','-z','--no-renames',base,head,'--').split('\0').filter(Boolean);
  const coreDiff=names.includes(CORE)?git('diff','--unified=0',base,head,'--',CORE):'';
  const libDiff=names.includes(LIB)?git('diff','--unified=0',base,head,'--',LIB):'';
  const lockBefore=names.includes(LOCK)?git('show',base+':'+LOCK):'';
  const lockAfter=names.includes(LOCK)?git('show',head+':'+LOCK):'';
  // Only the exact four-file version bump is exempted from shared OS labs.
  // Mixed release/CI/source changes (such as PR #621) retain every OS lab.
  const pureVersionPaths=names.length===4&&new Set(names).size===4&&
    names.every(p=>VERSION_ONLY_PATHS.has(p));
  const versionOnly=pureVersionPaths&&versionBumpOnly({
    paths:names,
    coreBefore:git('show',base+':'+CORE),coreAfter:git('show',head+':'+CORE),
    nativeBefore:git('show',base+':'+NATIVE),nativeAfter:git('show',head+':'+NATIVE),
    lockBefore,lockAfter,
    windowsBefore:git('show',base+':'+WINDOWS),windowsAfter:git('show',head+':'+WINDOWS),
  });
  if(versionOnly)process.stderr.write('CI_SHARED_MANIFEST_VERSION_ONLY no dependency graph changes\n');
  return assess({paths:names,coreDiff,libDiff,lockBefore,lockAfter,versionOnly});
}
if(require.main===module){
  try{
    const p=Object.fromEntries(process.argv.slice(2).map(x=>{const m=/^--(base|head)=(.+)$/.exec(x);if(!m)throw Error('bad argument');return [m[1],m[2]];}));
    const r=compute(p.base,p.head||'HEAD');
    for(const [k,v] of Object.entries(r))process.stdout.write(k+'='+v+'\n');
    process.stderr.write('CI_SHARED_MANIFEST_ROUTE '+JSON.stringify(r)+'\n');
  }catch(e){console.error('CI_SHARED_MANIFEST_ROUTE_FAIL_CLOSED '+e.message);process.exitCode=1;}
}
module.exports={OWNED_SUITES,ownedRoutes,cargoAdditive,lockAdditive,libAdditive,versionBumpOnly,assess,compute,CORE,LOCK,LIB,ROOT,NATIVE,WINDOWS};
