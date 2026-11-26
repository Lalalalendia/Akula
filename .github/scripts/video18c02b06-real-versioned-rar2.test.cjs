'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const file={
  old:'scripts/windows-video18c02b02b02a-distinct-native.ps1',
  signed:'scripts/windows-video18c02b02b02b-distinct-signed-generations.ps1',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml',
  verifier:'labs/video-lite/video18c02b04-native-pe-version.rs',
  rar2:'labs/video-lite/video18c02b03-rar2-gui/src/main.rs',
  version:'scripts/akula-version.cjs',
  gui:'labs/video-lite/video18c02b_real_gui_ack.rs',
  crash:'labs/video-lite/video18c02b07_process_crash.rs',
  doc:'docs/release/video18c02b06-real-versioned-rar2.md'
};
const get=k=>fs.readFileSync(path.join(root,file[k]),'utf8');
const hold=(ok,message)=>assert.ok(ok,'VIDEO18C02B06_HOLD: '+message);
function gate(a){
  const paths=a.workflow.split(/\r?\npermissions:/,1)[0];
  for(const p of [file.old,file.signed,file.verifier,file.rar2,file.doc,file.version,file.gui,file.crash,
    '.github/scripts/video18c02b06-real-versioned-rar2.test.cjs',
    'aquilum-app/native/Cargo.toml','aquilum-app/core/Cargo.toml','aquilum-app/Cargo.lock']){
    hold(paths.includes("      - '"+p+"'"),'missing exact-head workflow path trigger '+p);
  }
  for(const required of [
    '(git -C $oldTree hash-object aquilum-app/native/Cargo.toml)',
    '(git -C $repo hash-object aquilum-app/native/Cargo.toml)',
    '(git -C $oldTree hash-object aquilum-app/Cargo.lock)',
    '(git -C $repo hash-object aquilum-app/Cargo.lock)',
    '$oldCargoBlob -cne $oldCargoWorktree',
    '$newCargoBlob -cne $newCargoWorktree',
    '$oldLockBlob -cne $oldLockWorktree',
    '$newLockBlob -cne $newLockWorktree',
    '$oldCargoBlob -ceq $newCargoBlob',
    '$oldLockBlob -ceq $newLockBlob',
    'version = "0\\.3\\.20"',
    "'scripts/akula-version.cjs') check",
    'VIDEO18C02B02B02A_OLD_GUI_VISIBLE_PASS'
  ])hold(a.old.includes(required),'separate source provenance/version check lost '+required);
  for(const required of [
    '$oldLockBlob -cne $oldLockWorktree',
    '$newLockBlob -cne $newLockWorktree',
    '$oldLockBlob -ceq $newLockBlob',
    'VERSION_WITNESS',
    'Assert-RealFile $versionWitness',
    "$versionWitness --admit $NewAppPath $newVersion '0.3.20' $newVersion",
    'VIDEO18C02B04_STRICT_VERSION_FLOOR_PASS',
    '$oldLockSha=File-Sha',
    '$newLockSha=File-Sha',
    '$oldLockSha -ceq $newLockSha',
    "source_commit=$gen[3];cargo_lock_sha256=$gen[6];components=$records",
    "@('old',$root,$oldHashes,$oldCommit,'0.3.20',$trustOld,$oldLockSha)",
    "@('new',$current,$newHashes,$env:EXPECTED_HEAD_SHA,$newVersion,$trustNew,$newLockSha)",
    '& $mini -Vm $manifest -x $signature -p $pub -Q',
    'VIDEO18C02B06_SIGNED_REAL_VERSION_PAIR_PASS',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS'
  ])hold(a.signed.includes(required),'signed version pair provenance lost '+required);
  for(const required of [
    'EXPECTED_HEAD_SHA: ${{ github.event.pull_request.head.sha }}',
    'github-hosted','windows-2022',
    'node scripts/akula-version.cjs check',
    'cargo metadata --locked --no-deps --format-version 1',
    "packages[0].version -cne $sourceVersion",
    'rustc --edition=2024 labs/video-lite/video18c02b04-native-pe-version.rs',
    '--admit $env:VIDEO18B04_APP',
    '$sourceVersion $previousVersion $sourceVersion',
    '@($previousVersion,$previousVersion),@($previousVersion,$futureVersion),@($sourceVersion,$sourceVersion)',
    'VIDEO18C02B04_HOLD: signed label is not a genuine newer native PE version',
    'VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS',
    'VIDEO18C02B06_VERSION_WITNESS=$witness',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS',
    '.github/scripts/video18c02b06-real-versioned-rar2.test.cjs'
  ])hold(a.workflow.includes(required),'Windows version+rar2 gate lost '+required);
  // The last negative PE verifier deliberately exits nonzero. This expected
  // refusal must not fail GitHub's PowerShell wrapper after all checks pass.
  // Never permit a reset before (or instead of) the three fail-closed refusals.
  const nativeGate=a.workflow.split('      - name: Bind current PE identity to the pinned historical floor before signing')[1]
    ?.split('      - name: Compile historical pinned Akula GUI independently from real source')[0];
  const reject=nativeGate?.indexOf("if($code -eq 0 -or !(($no | Out-String).Contains('VIDEO18C02B04_HOLD:");
  const passed=nativeGate?.indexOf('Write-Host "VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS');
  const reset=nativeGate?.indexOf('$global:LASTEXITCODE=0');
  hold(!!nativeGate && reject>=0 && passed>reject && reset>passed &&
    nativeGate.split('$global:LASTEXITCODE=0').length===2,
    'expected nonzero native rejection cleared once, only after all denials and success');
  for(const required of ['GetFileVersionInfoW','VerQueryValueW',
    'only exact source-built native EXE admitted','embedded<=installed'])
    hold(a.verifier.includes(required),'not genuine PE version verification '+required);
  for(const required of ['InstallLock::try_acquire(&root)','engine.confirm_candidate()',
    'engine.recover()','verify_first_presented_frame_ack',
    'signed.verify_stage(&stage)?','["wrong-nonce","wrong-generation","good"]'])
    hold(a.rar2.includes(required),'existing rar2 safety proof removed '+required);
  hold(a.gui.includes('include_str!("../../aquilum-app/native/Cargo.toml")') &&
    a.gui.includes('let version=candidate_version()?;') &&
    a.rar2.includes('signed.version()==super::gui_launcher::lab::candidate_version()?') &&
    a.crash.includes('signed.version()==super::super::gui_launcher::lab::candidate_version()?'),
    'the GUI ACK and both rar2 processes must bind the tracked candidate version');
  hold(!/workflow_dispatch:|contents:\s*write|gh release create|cargo publish|publish_beta/.test(a.workflow),
    'release-capable CI is forbidden');
  for(const required of ['RELEASE HOLD','0.3.20','0.3.21','ephemeral','publisher',
    'crash/reboot','five','rar2']){
    hold(a.doc.includes(required),'missing scope/release disclaimer '+required);
  }
}
test('Real pinned 0.3.20 to current signed five-PE rar2 transition is version-bound',()=>{
  gate(Object.fromEntries(Object.keys(file).map(k=>[k,get(k)])));
});
test('Mutations cannot bypass source custody, native PE, signed manifest and rar2 admission',()=>{
  const a=Object.fromEntries(Object.keys(file).map(k=>[k,get(k)]));
  const cases=[
    ['old','$oldCargoBlob -cne $oldCargoWorktree','$false'],
    ['old','$oldLockBlob -cne $oldLockWorktree','$false'],
    ['old','$oldCargoBlob -ceq $newCargoBlob','$false'],
    ['old',"'scripts/akula-version.cjs') check","'scripts/akula-version.cjs') fake"],
    ['signed','$newLockBlob -cne $newLockWorktree','$false'],
    ['signed','$versionWitness --admit $NewAppPath','echo --admit $NewAppPath'],
    ['signed','Assert-RealFile $versionWitness','Write-Host fake-witness'],
    ['signed','$oldLockSha -ceq $newLockSha','$false'],
    ['signed','cargo_lock_sha256=$gen[6]','cargo_lock_sha256=$newLockSha'],
    ['signed','& $mini -Vm $manifest -x $signature -p $pub -Q','Write-Host fake-signature'],
    ['workflow','node scripts/akula-version.cjs check','echo fake-source'],
    ['workflow','VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS','SKIPPED_NATIVE_FLOOR'],
    ['workflow','$global:LASTEXITCODE=0','// stale negative exit status'],
    ['workflow',`Write-Host "VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS old_source=$previousVersion new_source=$sourceVersion pe_verified=1 negatives=3 nonrelease=1"
          # GitHub's pwsh wrapper`,
     `$global:LASTEXITCODE=0
          Write-Host "VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS old_source=$previousVersion new_source=$sourceVersion pe_verified=1 negatives=3 nonrelease=1"
          # GitHub's pwsh wrapper`],
    ['workflow','$sourceVersion $previousVersion $sourceVersion','$sourceVersion $sourceVersion $sourceVersion'],
    ['workflow','VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS','SKIPPED_RAR2'],
    ['rar2','engine.confirm_candidate()','// no confirmation'],
    ['rar2','engine.recover()','// no rollback'],
    ['verifier','embedded<=installed','false']
  ];
  for(const [k,from,to] of cases){
    const changed=a[k].replaceAll(from,to);
    assert.notEqual(changed,a[k],'VIDEO18C02B06_HOLD: test mutation missing '+k+' '+from);
    assert.throws(()=>gate({...a,[k]:changed}),/VIDEO18C02B06_HOLD/);
  }
});
