// VIDEO-18c-02b-07: forcibly terminate a SEPARATE controller process while
// signed, distinct real five-PE generations are transacting in Program Files.
// PR-only, ephemeral Minisign key, no public updater or release.
use super::{SignedFive, err, five, old_flat, real, sha};
use akula_video18b18_signed_five::admit_hosted_lab_signature;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdateError, UpdatePhase};
use chaptera_update_orchestrator::{InstallLock, OrchestrationError};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const PHASES: [&str; 4] = ["prepared", "retained", "activated", "ack-preconfirm"];

fn insist(ok: bool, reason: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(format!("VIDEO18C02B07_HOLD: {reason}")) }
}
fn phase_state(phase: &str) -> Result<UpdatePhase, String> {
    match phase {
        "prepared" => Ok(UpdatePhase::Prepared),
        "retained" => Ok(UpdatePhase::PreviousRetained),
        "activated" | "ack-preconfirm" => Ok(UpdatePhase::CandidateActivated),
        _ => Err(format!("VIDEO18C02B07_HOLD: unrecognized crash phase {phase}")),
    }
}
fn tx_id(phase: &str) -> String {
    format!("video18c02b07-{phase}")
}
fn lab_exe(work: &Path) -> PathBuf {
    work.join("rar2-ack-positive-build/release/akula-video18c02b03-rar2-positive-lab.exe")
}

// Child arguments are NOT a general updater API. Recheck all hosted and
// protected-root boundaries independently; the parent cannot lend trust.
pub(super) fn child(args: &[String]) -> Result<(), String> {
    super::super::gui_launcher::lab::guarded_pr()?;
    insist(args.len() == 8 && args[1] == "--crash-child",
        "crash child requires exact isolated argument shape")?;
    let phase=args[2].as_str();
    let expected_phase=phase_state(phase)?;
    let runner=fs::canonicalize(env::var_os("RUNNER_TEMP")
        .ok_or("VIDEO18C02B07_HOLD: no RUNNER_TEMP")?).map_err(err)?;
    let program=fs::canonicalize(env::var_os("ProgramFiles")
        .ok_or("VIDEO18C02B07_HOLD: no ProgramFiles")?).map_err(err)?;
    let root=fs::canonicalize(&args[3]).map_err(err)?;
    real(&root,true)?;
    let leaf=root.file_name().and_then(|s|s.to_str())
        .ok_or("VIDEO18C02B07_HOLD: invalid root name")?;
    let tag=leaf.strip_prefix("AkulaVideo18bRealNsisLab-")
        .ok_or("VIDEO18C02B07_HOLD: not protected NSIS lab")?;
    insist(root.parent()==Some(program.as_path()) && tag.len()==32
        && tag.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c)),
        "not an exclusive GUID child of Program Files")?;
    let stage=fs::canonicalize(&args[4]).map_err(err)?;
    real(&stage,true)?;
    let work=stage.parent().ok_or("VIDEO18C02B07_HOLD: candidate has no parent")?;
    insist(work.parent()==Some(runner.as_path())
        && work.file_name().and_then(|s|s.to_str())
            ==Some(format!("Video18c02b02b02b-{tag}").as_str())
        && stage.file_name().and_then(|s|s.to_str())==Some("candidate"),
        "untrusted stage escaped isolated runner work")?;
    let exe=fs::canonicalize(env::current_exe().map_err(err)?).map_err(err)?;
    let expected_exe=fs::canonicalize(lab_exe(work)).map_err(err)?;
    insist(exe==expected_exe, "unrecognized crash worker executable")?;
    let trust=work.join("new-trust");
    let manifest=fs::canonicalize(&args[5]).map_err(err)?;
    let signature=fs::canonicalize(&args[6]).map_err(err)?;
    let pubkey=fs::canonicalize(&args[7]).map_err(err)?;
    insist(manifest==trust.join("five-manifest.json")
        && signature==trust.join("five-manifest.minisig")
        && pubkey==trust.join("test-publisher.pub"),
        "crash worker trust files escaped its ephemeral directory")?;
    for p in [&manifest,&signature,&pubkey] { real(p,false)?; }
    let workspace=fs::canonicalize(env::var_os("GITHUB_WORKSPACE")
        .ok_or("VIDEO18C02B07_HOLD: no checkout")?).map_err(err)?;
    let head=env::var("EXPECTED_HEAD_SHA").map_err(err)?;
    insist(head.len()==40 && head.bytes().all(|c|c.is_ascii_hexdigit()),
        "invalid exact PR head")?;
    let git=Command::new("git").arg("-C").arg(&workspace)
        .args(["rev-parse","HEAD"]).output().map_err(err)?;
    insist(git.status.success()
        && String::from_utf8_lossy(&git.stdout).trim()==head,
        "crash worker checkout differs from exact PR head")?;
    let bytes=fs::read(&manifest).map_err(err)?;
    let sig=fs::read(&signature).map_err(err)?;
    let key=fs::read(&pubkey).map_err(err)?;
    insist((1..=8192).contains(&bytes.len()) && (1..=4096).contains(&sig.len())
        && (1..=4096).contains(&key.len()), "invalid signature input size")?;
    let signed=admit_hosted_lab_signature(
        &bytes,&STANDARD.encode(sig),&STANDARD.encode(key),"0.3.20"
    )?;
    insist(signed.version()==super::super::gui_launcher::lab::candidate_version()? && signed.source_commit()==head,
        "wrong signed generation/version/source")?;
    insist(signed.cargo_lock_sha256()==sha(&workspace.join("aquilum-app/Cargo.lock"))?,
        "new signed generation bound to different source lock")?;
    signed.verify_stage(&stage)?;
    let current=root.join("current");
    let old=five(&current)?;
    old_flat(&root,&old)?;
    insist(signed.verify_stage(&current).is_err(),
        "old generation unexpectedly accepted as new signed candidate")?;

    let _lock=InstallLock::try_acquire(&root).map_err(err)?;
    let engine=UpdateEngine::new(&root);
    insist(engine.read_journal().map_err(err)?.is_none(),
        "cannot crash-test an already unresolved update")?;
    let copied=engine.begin_verified_candidate(
        &tx_id(phase),signed.version(),&stage,Path::new("akula-app.exe")
    ).map_err(err)?;
    insist(sha(&copied)?==old["akula-app.exe"],
        "test control was not copied from the historical GUI")?;
    if phase!="prepared" {
        engine.retain_previous().map_err(err)?;
    }
    if matches!(phase,"activated"|"ack-preconfirm") {
        engine.activate_candidate().map_err(err)?;
        signed.verify_stage(&current)?;
        old_flat(&root,&old)?;
    }
    if phase=="ack-preconfirm" {
        let check=|receipt:&[u8],nonce:&str,pid:u32| {
            signed.verify_first_presented_frame_ack(receipt,nonce,pid).is_ok()
        };
        let accepted=super::super::gui_launcher::lab::case(
            &current.join("akula-app.exe"),signed.generation_sha256(),
            "crash-preconfirm",Some(&check)
        ).map_err(err)?;
        insist(accepted, "signed real first-frame ACK was not observed")?;
    }
    insist(engine.read_journal().map_err(err)?
        .is_some_and(|journal|journal.phase==expected_phase),
        "worker did not reach the requested durable journal phase")?;
    old_flat(&root,&old)?;
    println!("VIDEO18C02B07_KILL_READY phase={phase}");
    std::io::stdout().flush().map_err(err)?;
    // No successful process exit, no confirmation and no transaction cleanup.
    // The parent uses Windows TerminateProcess via Child::kill while the OS
    // InstallLock is still held, then independently verifies recovery.
    loop { std::thread::park(); }
}

pub(super) fn matrix(
    root: &Path, stage: &Path, manifest: &Path, signature: &Path, pubkey: &Path,
    signed: &SignedFive, old: &BTreeMap<String,String>,
) -> Result<(), String> {
    super::super::gui_launcher::lab::guarded_pr()?;
    let work=stage.parent().ok_or("VIDEO18C02B07_HOLD: missing work root")?;
    let exe=fs::canonicalize(env::current_exe().map_err(err)?).map_err(err)?;
    insist(exe==fs::canonicalize(lab_exe(work)).map_err(err)?,
        "matrix cannot run from an arbitrary executable")?;
    let note=work.join("vault/Protected historical note.md");
    let note_hash=sha(&note)?;
    for phase in PHASES {
        insist(five(&root.join("current"))?==*old,
            "crash matrix must start from exact historical five PE")?;
        old_flat(root,old)?;
        signed.verify_stage(stage)?;
        insist(UpdateEngine::new(root).read_journal().map_err(err)?.is_none(),
            "previous transaction not fully settled before next crash")?;
        let mut worker=Command::new(&exe)
            .arg("--crash-child").arg(phase)
            .arg(root).arg(stage).arg(manifest).arg(signature).arg(pubkey)
            .stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().map_err(err)?;
        let stdout=worker.stdout.take()
            .ok_or("VIDEO18C02B07_HOLD: missing private child stdout")?;
        let (sender,receiver)=mpsc::channel();
        std::thread::spawn(move || {
            let mut line=String::new();
            let _=BufReader::new(stdout).read_line(&mut line);
            let _=sender.send(line);
        });
        let marker=receiver.recv_timeout(Duration::from_secs(90))
            .unwrap_or_default();
        let alive=worker.try_wait().map_err(err)?.is_none();
        let busy=matches!(InstallLock::try_acquire(root),
            Err(OrchestrationError::LockBusy));
        let killed=worker.kill().is_ok();
        let status=worker.wait().map_err(err)?;
        insist(marker.trim()==format!("VIDEO18C02B07_KILL_READY phase={phase}"),
            "worker did not prove the exact phase before forced termination")?;
        insist(alive && busy && killed && !status.success(),
            "forced external process kill did not release an owned OS lock")?;
        let _recovery_lock=InstallLock::try_acquire(root).map_err(err)?;
        let engine=UpdateEngine::new(root);
        let expected_phase=phase_state(phase)?;
        insist(engine.read_journal().map_err(err)?
            .is_some_and(|j|j.phase==expected_phase),
            "crash did not preserve expected durable transaction journal")?;
        insist(matches!(engine.begin_verified_candidate(
            "video18c02b07-overlap",signed.version(),stage,Path::new("akula-app.exe")
        ),Err(UpdateError::ActiveTransaction(_))),
            "crashed transaction did not prohibit a second update")?;
        let outcome=engine.recover().map_err(err)?;
        let expected_outcome=if phase=="prepared" {
            RecoveryOutcome::PreparedTransactionAborted
        } else {
            RecoveryOutcome::UnconfirmedCandidateRolledBack
        };
        insist(outcome==expected_outcome,
            "restarted updater did not restore exactly the expected phase")?;
        insist(five(&root.join("current"))?==*old,
            "recovery left a mixed or changed five-PE generation")?;
        old_flat(root,old)?;
        insist(signed.verify_stage(&root.join("current")).is_err(),
            "recovered old generation could be authenticated as new")?;
        insist(sha(&note)?==note_hash, "private note changed across forced kill")?;
        insist(engine.read_journal().map_err(err)?.is_none(),
            "unconfirmed active journal survived independent recovery")?;
        engine.cleanup_orphaned_transactions().map_err(err)?;
        println!("VIDEO18C02B07_FORCED_PROCESS_KILL_ROLLBACK_PASS phase={phase} \
            os_lock_reacquired=1 exact_five_old=1 signed_candidate_refused=1 \
            private_note_preserved=1 release=0");
    }
    signed.verify_stage(stage)?;
    println!("VIDEO18C02B07_PROCESS_CRASH_MATRIX_PASS phases=4 \
        real_signed_pe=5 forced_kill=4 real_gui_ack_before_commit=1 \
        no_false_confirmation=1 release=0");
    Ok(())
}
