'use strict';
const { execFileSync } = require('node:child_process');

// Root manifests, vendor changes and mixed Core/Native edits retain all tests.
// Native source-only changes test the native crate (which builds Core as a dependency)
// and still require the full frame-fidelity lane, without re-running Core unit tests.
function classify(changedPaths) {
  // CI scripts and workflow definitions are validated by Node/policy checks.
  // They are not Rust source changes and must not schedule an entire workspace build.
  const relevant=changedPaths.filter(p=>p.startsWith('aquilum-app/'));
  if(relevant.length===0) return 'none';
  const nativeOnly=relevant.every(p=>p.startsWith('aquilum-app/native/src/')
    ||p.startsWith('aquilum-app/native/tests/'));
  if(nativeOnly) return 'native';
  const syncOnly=relevant.every(p=>p.startsWith('aquilum-app/core/src/sync/')
    ||p==='aquilum-app/core/tests/sync_contract.rs');
  return syncOnly?'sync':'workspace';
}
function changed(base,head='HEAD',cwd=process.cwd()) {
  if(!base||!head||!/^[-.\w/]+$/.test(base)||!/^[-.\w/]+$/.test(head))
    throw Error('Invalid --base or --head');
  // NUL delimiters preserve arbitrary Git filenames. File deletions, renames
  // and type changes must all be considered for CI scope.
  // A move away from updater/ must still run its security gate: include source and destination.
  const raw=execFileSync('git',['diff','--name-only','-z','--no-renames',
    base,head,'--'],{encoding:'utf8',maxBuffer:32*1024*1024,cwd});
  return raw.split('\0').filter(Boolean);
}

// Standalone Text Delta is outside the main Cargo workspace.
// Tests are required by Main PR gate rather than a duplicate PR workflow.
function textDeltaModuleRequired(changedPaths) {
  if (!Array.isArray(changedPaths)) return true;
  return changedPaths.some(p => typeof p !== 'string' || !p ||
    p.startsWith('modules/text-delta-codec/') ||
    p === '.github/workflows/text-delta-codec-isolated.yml');
}

// Pure module-only Text Delta diffs may use the same real S3 parser without
// the unrelated AWS SDK compile. Mixed, Core, S3 and lock changes retain AWS.
function textDeltaPackFastPath(paths) {
  const root='modules/text-delta-codec/';
  return Array.isArray(paths) && paths.length>0 && paths.every(
    p=>typeof p==='string' && p.startsWith(root) && p.length>root.length);
}
// Changes to parser, cfg wiring or route definition run BOTH feature modes.
function textDeltaPackBoundaryChanged(paths) {
  if(!Array.isArray(paths))return true;
  const owners=new Set([
    'aquilum-app/core/Cargo.toml',
    'aquilum-app/core/src/sync/mod.rs',
    'aquilum-app/core/src/sync/s3_transport.rs',
    'aquilum-app/core/src/sync/s3_transport/pack_probe.rs',
    '.github/workflows/sync-text-delta-rust.yml',
    '.github/workflows/ci-shared-manifest-labs.yml',
    '.github/scripts/ci-rust-scope.cjs',
  ]);
  return paths.some(p=>typeof p!=='string'||!p||owners.has(p));
}

const CORE_MANIFEST='aquilum-app/core/Cargo.toml';
const WORKSPACE_LOCK='aquilum-app/Cargo.lock';
// Strict fast path: only opt-in local manifest/linkage without changing any
// existing package record. All other workspace edits retain the full suite.
function safeManifestOnly(paths,route){
  if(!Array.isArray(paths)||!route||route.shared_any_risky!==false)return false;
  return paths.length>0 && paths.includes(CORE_MANIFEST) &&
    paths.every(p=>p===CORE_MANIFEST||p===WORKSPACE_LOCK);
}
function requiredMode(paths,base,head='HEAD'){
  const original=classify(paths);
  if(original!=='workspace' || !paths.includes(CORE_MANIFEST) ||
     !paths.every(p=>p===CORE_MANIFEST||p===WORKSPACE_LOCK))return original;
  // compute() fails closed on malformed or changed existing Cargo.lock data.
  const route=require('./ci-shared-manifest-route.cjs').compute(base,head);
  return safeManifestOnly(paths,route)?'manifest':'workspace';
}


// Standalone macOS updater acceptance is independent of arbitrary Core/Native
// edits. Keep every source and installer rule that can affect its trust boundary.
const UPDATER_SOURCE_PREFIX='aquilum-app/native/src/updater/';
const UPDATER_OWNERS=new Set([
  'aquilum-app/native/Cargo.toml',
  'scripts/install.sh',
  'scripts/uninstall.sh',
  'scripts/verify-macos-release.sh',
  'scripts/test-install-scripts.sh',
  'scripts/test-macos-release-verifier.sh',
  '.github/workflows/macos-smoke.yml',
  '.github/workflows/macos-updater-integration.yml',
  '.github/workflows/app-checks.yml',
  '.github/scripts/ci-rust-scope.cjs',
]);
function updaterSecurityAffected(paths){
  // Invalid discovery must never turn security tests off.
  if(!Array.isArray(paths))return true;
  return paths.some(p=>typeof p!=='string' || !p || p.startsWith(UPDATER_SOURCE_PREFIX) ||
    UPDATER_OWNERS.has(p));
}

function main(argv) {
  const args=Object.fromEntries(argv.map(part=>{
    const match=part.match(/^--(base|head)=([^\s]+)$/);
    if(!match) throw Error('Expected --base=<sha> --head=<sha>');
    return [match[1],match[2]];
  }));
  if(!args.base) throw Error('--base is required');
  const paths=changed(args.base,args.head||'HEAD');
  const mode=requiredMode(paths,args.base,args.head||'HEAD');
  process.stdout.write('mode='+mode+'\ntext_delta_module_required='+textDeltaModuleRequired(paths)+'\ntext_delta_pack_fast_path='+textDeltaPackFastPath(paths)+'\ntext_delta_pack_boundary_changed='+textDeltaPackBoundaryChanged(paths)+'\nupdater_security_required='+updaterSecurityAffected(paths)+'\n');
}
if(require.main===module) {
  try {main(process.argv.slice(2));}
  catch(err){console.error(err.message);process.exitCode=1;}
}
module.exports={classify,changed,safeManifestOnly,requiredMode,textDeltaModuleRequired,textDeltaPackFastPath,textDeltaPackBoundaryChanged,updaterSecurityAffected,main};
