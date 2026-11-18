'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const files={
  crash:'labs/video-lite/video18c02b07_process_crash.rs',
  main:'labs/video-lite/video18c02b03-rar2-gui/src/main.rs',
  gui:'labs/video-lite/video18c02b_real_gui_ack.rs',
  ps:'scripts/windows-video18c02b02b02b-distinct-signed-generations.ps1',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml',
  doc:'docs/release/video18c02b07-process-crash.md'
};
const read=name=>fs.readFileSync(path.join(root,files[name]),'utf8');
const hold=(cond,message)=>assert.ok(cond,'VIDEO18C02B07_HOLD: '+message);
function check(a){
  const paths=a.workflow.split(/\r?\npermissions:/,1)[0];
  for(const p of [...Object.values(files),'.github/scripts/video18c02b07-process-crash.test.cjs']){
    hold(paths.includes("      - '"+p+"'"),'workflow missing exact-head trigger '+p);
  }
  for(const text of [
    'mod process_crash {','include!("../../video18c02b07_process_crash.rs");',
    'if args.len()==8 && args[1]=="--crash-child"',
    'return process_crash::child(&args);',
    'process_crash::matrix(&root,&stage,&manifest,&signature,&pubkey,&signed,&old)?;',
    'let oslock=InstallLock::try_acquire(&root)',
    '["wrong-nonce","wrong-generation","good"]',
    'engine.confirm_candidate()'
  ])hold(a.main.includes(text),'real signed GUI path or parent crash test disconnected '+text);
  const idx=a.main.indexOf('process_crash::matrix(&root');
  hold(idx>=0 && idx<a.main.indexOf('let oslock=InstallLock::try_acquire(&root)'),
    'forced kills must be completed before positive GUI acceptance');
  for(const text of [
    'const PHASES: [&str; 4] = ["prepared", "retained", "activated", "ack-preconfirm"]',
    'super::super::gui_launcher::lab::guarded_pr()?',
    'root.parent()==Some(program.as_path())',
    'stage.file_name().and_then(|s|s.to_str())==Some("candidate")',
    'exe==expected_exe',
    'trust.join("five-manifest.json")','trust.join("five-manifest.minisig")',
    'trust.join("test-publisher.pub")','String::from_utf8_lossy(&git.stdout).trim()==head',
    'admit_hosted_lab_signature(', 'signed.cargo_lock_sha256()==sha(',
    'signed.verify_stage(&stage)?','old_flat(&root,&old)?',
    'InstallLock::try_acquire(&root)',
    'engine.begin_verified_candidate(', 'engine.retain_previous()',
    'engine.activate_candidate()',
    '"crash-preconfirm",Some(&check)',
    'signed.verify_first_presented_frame_ack',
    'journal.phase==expected_phase',
    'VIDEO18C02B07_KILL_READY phase={phase}',
    'std::io::stdout().flush()', 'loop { std::thread::park(); }',
    'Command::new(&exe)','--crash-child', 'recv_timeout(Duration::from_secs(90))',
    'worker.try_wait()', 'Err(OrchestrationError::LockBusy)',
    'worker.kill().is_ok()', 'worker.wait()',
    'let _recovery_lock=InstallLock::try_acquire(root)',
    'Err(UpdateError::ActiveTransaction(_))',
    'let outcome=engine.recover()', 'RecoveryOutcome::PreparedTransactionAborted',
    'RecoveryOutcome::UnconfirmedCandidateRolledBack',
    'five(&root.join("current"))?==*old',
    'signed.verify_stage(&root.join("current")).is_err()',
    'sha(&note)?==note_hash','engine.cleanup_orphaned_transactions()',
    'VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS'
  ])hold(a.crash.includes(text),'not a real forcibly killed signed five-PE crash matrix: '+text);
  hold(a.gui.includes('if !matches!(mode, "good" | "wrong-nonce" | "wrong-generation" | "crash-preconfirm")'),
    'crash GUI profile not isolated by a strict mode allowlist');
  for(const text of [
    'VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS',
    'VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS'
  ])hold(a.ps.includes(text),'Windows script skipped crash or positive witness '+text);
  for(const text of [
    '.github/scripts/video18c02b07-process-crash.test.cjs',
    'VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS',
    'VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS',
    'needs: [contract, preflight]',
    'repository: Lalalalendia/rar2',
    'ref: 32a803e67244836d2794f9bbbdf9b24deab63d7e'
  ])hold(a.workflow.includes(text),'CI not wired to protected old/new and both proofs '+text);
  hold(!/workflow_dispatch:|contents:\s*write|gh release create|publish_beta/.test(a.workflow),
    'publishing or write credentials are forbidden in this lab');
  for(const text of [
    'VIDEO-18 RELEASE HOLD','TerminateProcess','prepared','retained',
    'activated','ack-preconfirm','five','publisher','reboot',
    '0.3.20','0.3.21','no release'
  ])hold(a.doc.toLowerCase().includes(text.toLowerCase()),
    'release boundary or crash scope missing '+text);
}
test('A separate, live OS-locked Windows process is killed after four signed rar2 phases',()=>{
  check(Object.fromEntries(Object.keys(files).map(k=>[k,read(k)])));
});
test('Mutation tests reject fabricated kills, false ACK, journal skip, missing lock or loss of old PE',()=>{
  const a=Object.fromEntries(Object.keys(files).map(k=>[k,read(k)]));
  const cases=[
    ['main','process_crash::matrix(&root,&stage,&manifest,&signature,&pubkey,&signed,&old)?;','// omitted crash matrix'],
    ['main','return process_crash::child(&args);','return Ok(());'],
    ['crash','worker.kill().is_ok()','true'],
    ['crash','worker.try_wait()','worker.wait()'],
    ['crash','Err(OrchestrationError::LockBusy)','Ok(_)'],
    ['crash','let _recovery_lock=InstallLock::try_acquire(root)','let _recovery_lock=Ok(())'],
    ['crash','Err(UpdateError::ActiveTransaction(_))','Ok(_)'],
    ['crash','let outcome=engine.recover()','let outcome=Ok(RecoveryOutcome::NothingToDo)'],
    ['crash','signed.verify_stage(&stage)?','// trust unchecked candidate'],
    ['crash','signed.verify_first_presented_frame_ack','verify_without_signature'],
    ['crash','"crash-preconfirm",Some(&check)','"good",None'],
    ['crash','sha(&note)?==note_hash','true'],
    ['crash','five(&root.join("current"))?==*old','true'],
    ['crash','VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS','PRETEND_RECOVERY_PASS'],
    ['gui','"wrong-generation" | "crash-preconfirm"','"wrong-generation"'],
    ['ps',"'VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS'","'FAKE_CRASH_PASS'"],
    ['workflow','VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS','SKIP_PROCESS_KILL'],
    ['workflow','needs: [contract, preflight]','needs: contract']
  ];
  for(const [name,from,to] of cases){
    const modified=a[name].replaceAll(from,to);
    assert.notEqual(modified,a[name],'VIDEO18C02B07_HOLD: mutation not applied '+from);
    assert.throws(()=>check({...a,[name]:modified}),/VIDEO18C02B07_HOLD/);
  }
});
