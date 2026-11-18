'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const files={
  script:'scripts/windows-video18c02b02b02b-distinct-signed-generations.ps1',
  test:'.github/scripts/video18c02b02b02b-distinct-signed-generations.test.cjs',
  doc:'docs/release/video18c02b02b02b-distinct-installed-signed-generations.md',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml'
};
const read=key=>fs.readFileSync(path.join(root,files[key]),'utf8');
function guard(ps,wf,doc){
  const on=wf.split(/\r?\npermissions:/,1)[0];
  for(const name of [files.script,files.test,files.doc]){
    assert.ok(on.includes("      - '"+name+"'"),'VIDEO18C02B02B02B_HOLD: no workflow path trigger: '+name);
  }
  for(const marker of [
    "GITHUB_EVENT_NAME -cne 'pull_request'","RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "(git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA",
    "ec6d6d48a78e60fca7a6f810b825418500bad64b",
    "source-pinned build roots",
    "if($oldHashes['akula-app'] -ceq $newHashes['akula-app'])",
    "Assert-Set $root $oldHashes $true",
    "Assert-Set $current $newHashes $false",
    "source_commit=$gen[3]","cargo_lock_sha256=$gen[6]",
    "version=$gen[4]","& $mini -S -s $key",
    "& $mini -Vm $manifest -x $signature -p $pub -Q",
    "& $mini -Vm $manifests.new.manifest -x $manifests.old.signature",
    "Run-OldProtectedGui (Join-Path $root 'akula-app.exe')",
    "VIDEO18C02B02B02B_OLD_PROTECTED_GUI_FALLBACK_PASS",
    "VIDEO18C02B02B02B_TORN_NEW_REJECTED_PASS",
    "VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS",
    "Write-Output 'VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS",
    "no_rar2_commit=1","release=0"
  ])assert.ok(ps.includes(marker),'VIDEO18C02B02B02B_HOLD: removed proof '+marker);
  for(const marker of [
    "node --test",".github/scripts/video18c02b02b02b-distinct-signed-generations.test.cjs",
    "windows-video18c02b02b02b-distinct-signed-generations.ps1",
    "VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS",
    "VIDEO18C02B02B02A_OLD_APP","needs: [contract, preflight]"
  ])assert.ok(wf.includes(marker),'VIDEO18C02B02B02B_HOLD: no CI acceptance '+marker);
  assert.ok(!/workflow_dispatch:|contents:\s*write|gh release create/.test(wf),
    'VIDEO18C02B02B02B_HOLD: no release or privileged workflow');
  assert.ok(!/confirm_candidate\s*\(|publish_beta/.test(ps),
    'VIDEO18C02B02B02B_HOLD: updater and publisher activation forbidden');
  assert.ok(doc.includes('RELEASE HOLD')&&doc.includes('0.3.20')&&
    doc.includes('0.3.21')&&doc.includes('не доказывает'),
    'VIDEO18C02B02B02B_HOLD: source/version limitations missing');
}
test('Two genuine installed Windows five-PE generations have different signed manifests',()=>{
  guard(read('script'),read('workflow'),read('doc'));
});
test('Negative: lose source pin, separate signed sets, fallback or receipt => reject',()=>{
  const ps=read('script'),wf=read('workflow'),doc=read('doc');
  for(const [field,needle,replacement] of [
    ['script',"if($oldHashes['akula-app'] -ceq $newHashes['akula-app'])","if($false)"],
    ['script',"& $mini -Vm $manifests.new.manifest -x $manifests.old.signature","& $mini -Vm $manifests.new.manifest -x $manifests.new.signature"],
    ['script',"Run-OldProtectedGui (Join-Path $root 'akula-app.exe')","Write-Host 'pretend old GUI'"],
    ['script',"Assert-Set $root $oldHashes $true","Write-Host 'pretend old PE intact'"],
    ['script',"VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS","VIDEO18C02B02B_GUI_ACK_SKIPPED"],
    ['script',"Write-Output 'VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS","Write-Host 'VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS"],
    ['workflow',"VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS","IGNORED_MARKER"],
    ['workflow',"needs: [contract, preflight]","needs: contract"],
  ]){
    const original=field==='script'?ps:wf;
    const mutated=original.replaceAll(needle,replacement);
    assert.notEqual(mutated,original,'VIDEO18C02B02B02B_HOLD: mutation skipped '+needle);
    assert.throws(()=>guard(field==='script'?mutated:ps,field==='workflow'?mutated:wf,doc),
      /VIDEO18C02B02B02B_HOLD/);
  }
});
