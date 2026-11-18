'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const files={
  rust:'labs/video-lite/video18c02b03-rar2-gui/src/main.rs',
  cargo:'labs/video-lite/video18c02b03-rar2-gui/Cargo.toml',
  script:'scripts/windows-video18c02b02b02b-distinct-signed-generations.ps1',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml',
  doc:'docs/release/video18c02b03-positive-rar2-ack-confirm.md'
};
const read=p=>fs.readFileSync(path.join(root,files[p]),'utf8');
function check(rust,cargo,script,workflow,doc){
  const pre=workflow.split(/\r?\npermissions:/,1)[0];
  for(const name of [
    files.rust,files.cargo,files.script,files.doc,
    '.github/scripts/video18c02b03-rar2-commit.test.cjs'
  ])assert.ok(pre.includes("      - '"+name+"'"),
    'VIDEO18C02B03_HOLD: workflow lacks source-trigger '+name);
  for(const needle of [
    'guarded_pr()?','EXPECTED_HEAD_SHA','ProgramFiles',
    'source_commit()==expected','signed.cargo_lock_sha256()==locksha',
    'admit_hosted_lab_signature(', 'signed.verify_stage(&stage)?',
    'signed.verify_stage(&root.join("current")).is_err()',
    'InstallLock::try_acquire(&root)',
    'Err(OrchestrationError::LockBusy)','engine.begin_verified_candidate(',
    'engine.retain_previous()','engine.activate_candidate()',
    'engine.confirm_candidate()', 'engine.recover()',
    'verify_first_presented_frame_ack','super::gui_launcher::lab::case(',
    '["wrong-nonce","wrong-generation","good"]',
    'if mode=="good"','if !accepted','need(!accepted',
    'RecoveryOutcome::UnconfirmedCandidateRolledBack',
    'VIDEO18C02B03_NEGATIVE_ACK_ROLLBACK_PASS',
    'VIDEO18C02B03_POSITIVE_ACK_CONFIRM_PASS',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS'
  ])assert.ok(rust.includes(needle),'VIDEO18C02B03_HOLD: lost real rar2/ACK gate '+needle);
  for(const needle of [
    'publish = false','lab-only-signature',
    'chaptera-update-engine','chaptera-update-orchestrator'
  ])assert.ok(cargo.includes(needle),'VIDEO18C02B03_HOLD: nonshipping crate lost '+needle);
  for(const needle of [
    'Rar2Positive','$proof | Write-Output',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS',
    'VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS',
    'VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS'
  ])assert.ok(script.includes(needle),'VIDEO18C02B03_HOLD: script lost admitted lab '+needle);
  for(const needle of [
    'node --test',files.rust,files.cargo,
    'video18c02b03-rar2-commit.test.cjs',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS',
    'Lalalalendia/rar2','32a803e67244836d2794f9bbbdf9b24deab63d7e',
    'needs: [contract, preflight]'
  ])assert.ok(workflow.includes(needle),'VIDEO18C02B03_HOLD: no Windows acceptance '+needle);
  assert.ok(!/workflow_dispatch:|contents:\s*write|gh release create/.test(workflow),
    'VIDEO18C02B03_HOLD: publishing workflow is prohibited');
  assert.ok(!/publisher_signing_authorized\s*=\s*true|publish_beta|confirm_candidate\s*\(\s*\)\s*;?\s*\/\/\s*without/.test(script),
    'VIDEO18C02B03_HOLD: no publisher/release bypass');
  for(const needle of ['RELEASE HOLD','0.3.20','0.3.21','not a product version upgrade']){
    assert.ok(doc.includes(needle),'VIDEO18C02B03_HOLD: scope limitation lost '+needle);
  }
}
test('Actual signed GUI ACK gates rar2 positive commit; negative GUI receipts roll back',()=>{
  check(read('rust'),read('cargo'),read('script'),read('workflow'),read('doc'));
});
test('Negative mutations fail closed for signed identity, Windows ACK, OS lock and recovery',()=>{
  const rust=read('rust'),cargo=read('cargo'),
    script=read('script'),workflow=read('workflow'),doc=read('doc');
  for(const [field,needle,replacement] of [
    ['rust','engine.confirm_candidate()','// no commit'],
    ['rust','engine.recover()','// no rollback'],
    ['rust','InstallLock::try_acquire(&root)','Err(OrchestrationError::LockBusy)'],
    ['rust','verify_first_presented_frame_ack','verify_frame_without_authentication'],
    ['rust','signed.verify_stage(&stage)?','// no signed verification'],
    ['rust','if !accepted','if accepted'],
    ['rust','need(!accepted','need(accepted'],
    ['rust','RecoveryOutcome::UnconfirmedCandidateRolledBack','RecoveryOutcome::NothingToDo'],
    ['rust','["wrong-nonce","wrong-generation","good"]','["good"]'],
    ['script','$proof | Write-Output','$proof | Out-Host'],
    ['workflow','VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS','SKIP_REAL_RAR2'],
    ['workflow','needs: [contract, preflight]','needs: contract']
  ]){
    const before=field==='rust'?rust:workflow;
    const changed=before.replaceAll(needle,replacement);
    assert.notEqual(before,changed,'VIDEO18C02B03_HOLD: mutation not applied: '+needle);
    assert.throws(()=>check(field==='rust'?changed:rust,cargo,script,
      field==='workflow'?changed:workflow,doc),/VIDEO18C02B03_HOLD/);
  }
});
