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
  doc:'docs/release/video18c02b06-real-versioned-rar2.md'
};
const get=k=>fs.readFileSync(path.join(root,file[k]),'utf8');
const hold=(ok,message)=>assert.ok(ok,'VIDEO18C02B06_HOLD: '+message);
function gate(a){
  const paths=a.workflow.split(/\r?\npermissions:/,1)[0];
  for(const p of [file.old,file.signed,file.verifier,file.rar2,file.doc,
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
    'version = "0\\.3\\.21"',
    'VIDEO18C02B02B02A_OLD_GUI_VISIBLE_PASS'
  ])hold(a.old.includes(required),'separate source provenance/version check lost '+required);
  for(const required of [
    '$oldLockBlob -cne $oldLockWorktree',
    '$newLockBlob -cne $newLockWorktree',
    '$oldLockBlob -ceq $newLockBlob',
    'VERSION_WITNESS',
    'Assert-RealFile $versionWitness',
    "$versionWitness --admit $NewAppPath '0.3.21' '0.3.20' '0.3.21'",
    'VIDEO18C02B04_STRICT_VERSION_FLOOR_PASS',
    '$oldLockSha=File-Sha',
    '$newLockSha=File-Sha',
    '$oldLockSha -ceq $newLockSha',
    "source_commit=$gen[3];cargo_lock_sha256=$gen[6];components=$records",
    "@('old',$root,$oldHashes,$oldCommit,'0.3.20',$trustOld,$oldLockSha)",
    "@('new',$current,$newHashes,$env:EXPECTED_HEAD_SHA,'0.3.21',$trustNew,$newLockSha)",
    '& $mini -Vm $manifest -x $signature -p $pub -Q',
    'VIDEO18C02B06_SIGNED_REAL_VERSION_PAIR_PASS',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS'
  ])hold(a.signed.includes(required),'signed version pair provenance lost '+required);
  for(const required of [
    'EXPECTED_HEAD_SHA: ${{ github.event.pull_request.head.sha }}',
    'github-hosted','windows-2022',
    'node scripts/akula-version.cjs check',
    'cargo metadata --locked --no-deps --format-version 1',
    "packages[0].version -cne '0.3.21'",
    'rustc --edition=2024 labs/video-lite/video18c02b04-native-pe-version.rs',
    '--admit $env:VIDEO18B04_APP',
    "'0.3.21' '0.3.20' '0.3.21'",
    "@('0.3.20','0.3.20'),@('0.3.20','0.3.22'),@('0.3.21','0.3.21')",
    'VIDEO18C02B04_HOLD: signed label is not a genuine newer native PE version',
    'VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS',
    'VIDEO18C02B06_VERSION_WITNESS=$witness',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS',
    '.github/scripts/video18c02b06-real-versioned-rar2.test.cjs'
  ])hold(a.workflow.includes(required),'Windows version+rar2 gate lost '+required);
  for(const required of ['GetFileVersionInfoW','VerQueryValueW',
    'only exact source-built native EXE admitted','embedded<=installed'])
    hold(a.verifier.includes(required),'not genuine PE version verification '+required);
  for(const required of ['InstallLock::try_acquire(&root)','engine.confirm_candidate()',
    'engine.recover()','verify_first_presented_frame_ack',
    'signed.verify_stage(&stage)?','["wrong-nonce","wrong-generation","good"]'])
    hold(a.rar2.includes(required),'existing rar2 safety proof removed '+required);
  hold(!/workflow_dispatch:|contents:\s*write|gh release create|cargo publish|publish_beta/.test(a.workflow),
    'release-capable CI is forbidden');
  for(const required of ['RELEASE HOLD','0.3.20','0.3.21','ephemeral','publisher',
    'crash/reboot','five','rar2']){
    hold(a.doc.includes(required),'missing scope/release disclaimer '+required);
  }
}
test('Real tracked 0.3.20 -> 0.3.21 signed five-PE rar2 transition is version-bound',()=>{
  gate(Object.fromEntries(Object.keys(file).map(k=>[k,get(k)])));
});
test('Mutations cannot bypass source custody, native PE, signed manifest and rar2 admission',()=>{
  const a=Object.fromEntries(Object.keys(file).map(k=>[k,get(k)]));
  const cases=[
    ['old','$oldCargoBlob -cne $oldCargoWorktree','$false'],
    ['old','$oldLockBlob -cne $oldLockWorktree','$false'],
    ['old','$oldCargoBlob -ceq $newCargoBlob','$false'],
    ['old','version = "0\\.3\\.21"','version = "0\\.3\\.20"'],
    ['signed','$newLockBlob -cne $newLockWorktree','$false'],
    ['signed','$versionWitness --admit $NewAppPath','echo --admit $NewAppPath'],
    ['signed','Assert-RealFile $versionWitness','Write-Host fake-witness'],
    ['signed','$oldLockSha -ceq $newLockSha','$false'],
    ['signed','cargo_lock_sha256=$gen[6]','cargo_lock_sha256=$newLockSha'],
    ['signed','& $mini -Vm $manifest -x $signature -p $pub -Q','Write-Host fake-signature'],
    ['workflow','node scripts/akula-version.cjs check','echo fake-source'],
    ['workflow','VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS','SKIPPED_NATIVE_FLOOR'],
    ['workflow',"'0.3.21' '0.3.20' '0.3.21'","'0.3.21' '0.3.21' '0.3.21'"],
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
