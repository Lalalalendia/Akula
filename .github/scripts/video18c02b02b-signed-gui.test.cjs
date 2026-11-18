'use strict';
const {test}=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../..');
const paths={
  rust:'labs/video-lite/video18c02b02b-signed-gui/src/main.rs',
  cargo:'labs/video-lite/video18c02b02b-signed-gui/Cargo.toml',
  launcher:'labs/video-lite/video18c02b_real_gui_ack.rs',
  contract:'.github/scripts/video18c02b02b-signed-gui.test.cjs',
  script:'scripts/windows-video18b16-protected-versioned-bridge.ps1',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml',
  doc:'docs/release/video18c02b02b-signed-current-gui.md'
};
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
function verify(files){
  const {rust,cargo,launcher,script,workflow}=files;
  const trigger=workflow.split(/\r?\npermissions:/,1)[0];
  for(const p of Object.values(paths))
    assert.ok(trigger.includes("      - '"+p+"'"),'VIDEO18C02B02B_HOLD: missing trigger '+p);
  for(const key of [
    'admit_hosted_lab_signature(', 'signed.verify_stage(current)?',
    'signed.verify_first_presented_frame_ack(receipt, nonce, os_pid).is_ok()',
    'signed.generation_sha256()', 'signed.version() != "0.3.21"',
    'signed.source_commit() != required("EXPECTED_HEAD_SHA")?',
    'signed.cargo_lock_sha256() != got_lock',
    'gui_launcher::lab::guarded_pr()', 'gui_launcher::lab::validate_app',
    'gui_launcher::lab::case(&app, digest, "good", Some(&verify))?',
    'gui_launcher::lab::case(&app, digest, "wrong-nonce", Some(&verify))?',
    'gui_launcher::lab::case(&app, &wrong_sha, "wrong-generation", Some(&verify))?',
    'if wrong_generation { return Err(', 'pinned_under_runner(', 'limited(&manifest, 8192)',
    'read_to_end(', 'fs::symlink_metadata(', 'file_attributes() & 0x400',
    'VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS',
    'old_new_distinct=0', 'rar2_activation=0 confirm=0 owner_key=0 release=0'
  ])assert.ok(rust.includes(key),'VIDEO18C02B02B_HOLD: unsigned/unchecked GUI '+key);
  assert.ok(rust.match(/let accepted\s*=\s*super::gui_launcher::lab::case/),
    'VIDEO18C02B02B_HOLD: real Akula GUI never launched');
  assert.ok(rust.match(/signed.verify_stage\(current\)\?;/g)?.length===2,
    'VIDEO18C02B02B_HOLD: five-PE SHA must be checked before and after real GUI');
  for(const x of [
    'pub(crate) mod lab {','pub(crate) fn case(', 'signed_check:',
    'let signed_bound = signed_check.map_or(true',
    'check(&receipt, &random, pi.pid)',
    'let valid=receipt==expected.as_bytes()',
    'Ok(valid && signed_bound)',
    'creation_time(child.0.0)?==first_creation',
    'let prefix = if signed_check.is_some()',
    '"Video18c02b02bSignedFrame"', '"Video18c02bFrame"',
    'if folder.exists() {return Err("reused laboratory profile".into());}',
    'if !matches!(mode, "good" | "wrong-nonce" | "wrong-generation" | "crash-preconfirm")'
  ])assert.ok(launcher.includes(x),'VIDEO18C02B02B_HOLD: OS/process witness lost '+x);
  assert.ok(cargo.includes('publish = false')&&cargo.includes('[workspace]')&&
    cargo.includes('lab-only-signature')&&cargo.includes('base64 = "0.22"'),
    'VIDEO18C02B02B_HOLD: signed GUI lab is not isolated and nonshipping');
  for(const t of [
    'Video18c02b02bBuild-', "version='0.3.21'",'source_commit=$env:EXPECTED_HEAD_SHA',
    "cargo_lock_sha256=$lockSha",'sha256=$sha;size=',
    "$sha -cne $expected[$name]", "schema=1",
    "purpose='protected-video-five-exe'", 'Get-Command minisign.exe',
    '& $mini -G -W', '& $mini -S', '& $mini -Vm',
    'cargo generate-lockfile --manifest-path $manifestCrate',
    'cargo build --release --locked --manifest-path $manifestCrate',
    '$proof=@(& $bin (Join-Path $current',
    "VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS",
    'VIDEO18C02B02B_PROTECTED_FIVE_UNCHANGED_PASS'
  ])assert.ok(script.includes(t),'VIDEO18C02B02B_HOLD: missing real signed five-PE script '+t);
  assert.ok(script.includes('Assert-Five $current')&&script.includes('Assert-Flat $root'),
    'VIDEO18C02B02B_HOLD: old and current files must remain intact');
  const sha='37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479';
  for(const x of [
    'node --test .github/scripts/video18c02b02b-signed-gui.test.cjs',
    'VIDEO18C02B02B_WINDOWS_SIGNED_GUI_COMPILE_PASS',
    'VIDEO18C02B02B_MINISIGN_PINNED_PASS',
    'cargo build --locked --manifest-path $manifest',
    sha, 'needs: [contract, preflight]', 'persist-credentials: false'
  ])assert.ok(workflow.includes(x),'VIDEO18C02B02B_HOLD: CI protection removed '+x);
  assert.ok(!workflow.includes('workflow_dispatch:')&&!/contents:\s*write/.test(workflow),
    'VIDEO18C02B02B_HOLD: no publishing permission');
  assert.ok(!/\b(confirm_candidate\s*\(|publish_beta|gh release create|remove_dir_all)\b/.test(rust+launcher),
    'VIDEO18C02B02B_HOLD: no product update confirmation or release');
  assert.ok(!fs.existsSync(path.join(root,'labs/video-lite/video18c02b02b-signed-gui/Cargo.lock')),
    'VIDEO18C02B02B_HOLD: unreviewed lab dependency lock was committed');
}
const all=()=>Object.fromEntries(Object.entries(paths).map(([k,p])=>[k,read(p)]));
test('Ephemeral Minisign exact five PE and real current GUI ACK share one OS child supervisor',()=>{
  verify(all());
});
test('Negative: signed manifest admission, five-file bytes, OS identity and digest cannot be bypassed',()=>{
  const src=all();
  for(const [field,from,to] of [
    ['rust','admit_hosted_lab_signature(','fake_admission('],
    ['rust','signed.verify_stage(current)?','unsafe_trust_current()'],
    ['rust','signed.verify_first_presented_frame_ack(receipt, nonce, os_pid).is_ok()','true'],
    ['rust','signed.generation_sha256()','"f".repeat(64)'],
    ['rust','signed.cargo_lock_sha256() != got_lock','false'],
    ['rust','if wrong_generation { return Err(','if false { return Err('],
    ['launcher','Ok(valid && signed_bound)','Ok(valid)'],
    ['launcher','creation_time(child.0.0)?==first_creation','true'],
    ['launcher','"Video18c02b02bSignedFrame"','"Video18c02bFrame"'],
    ['launcher','if folder.exists() {return Err("reused laboratory profile".into());}','if false {return Err("reused laboratory profile".into());}'],
    ['launcher','if !matches!(mode, "good" | "wrong-nonce" | "wrong-generation" | "crash-preconfirm")','if false'],
    ['script','& $mini -Vm','& $mini -NO-VERIFY'],
    ['script','$sha -cne $expected[$name]','false'],
    ['workflow','needs: [contract, preflight]','needs: contract'],
    ['workflow','37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479',
      '0000000000000000000000000000000000000000000000000000000000000000']
  ]){
    const mutated={...src,[field]:src[field].replace(from,to)};
    assert.notEqual(mutated[field],src[field],'VIDEO18C02B02B_HOLD: negative mutation absent '+from);
    assert.throws(()=>verify(mutated),/VIDEO18C02B02B_HOLD/);
  }
});
