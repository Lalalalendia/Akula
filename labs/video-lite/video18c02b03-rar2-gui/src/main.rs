//! VIDEO-18c-02b-03: REAL rar2 candidate activation, real GUI first-frame
//! signed-five ACK, then positive confirm. Two negative GUI receipts roll back.
//! Disposable GitHub-hosted Windows PR only; no product updater or release.
#[cfg(not(windows))]
fn main() {
    eprintln!("VIDEO18C02B03_HOLD: disposable Windows PR only");
    std::process::exit(2);
}
#[cfg(windows)]
#[path = "../../video18c02b_real_gui_ack.rs"]
mod gui_launcher;

#[cfg(windows)]
mod lab {
    use akula_video18b18_signed_five::{SignedFive, admit_hosted_lab_signature};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdatePhase};
    use chaptera_update_orchestrator::{InstallLock, OrchestrationError};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    const NAMES: [&str; 5] = [
        "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
        "akula-yt-dlp.exe", "akula-quickjs.exe",
    ];
    const PREFIX: &str = "AkulaVideo18bRealNsisLab-";
    mod process_crash {
        include!("../../video18c02b07_process_crash.rs");
    }
    fn err<T: ToString>(e: T) -> String { e.to_string() }
    fn need(ok: bool, reason: &str) -> Result<(), String> {
        if ok { Ok(()) } else { Err(format!("VIDEO18C02B03_HOLD: {reason}")) }
    }
    fn real(path: &Path, directory: bool) -> Result<(), String> {
        let m = fs::symlink_metadata(path).map_err(err)?;
        need((m.file_attributes() & 0x400) == 0
            && !m.file_type().is_symlink()
            && if directory { m.is_dir() } else { m.is_file() },
            "missing/reparse/unexpected protected path")
    }
    fn sha(path: &Path) -> Result<String, String> {
        real(path, false)?;
        let bytes=fs::read(path).map_err(err)?;
        need(!bytes.is_empty(), "empty protected hash input")?;
        Ok(Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect())
    }
    fn five(dir: &Path) -> Result<BTreeMap<String, String>, String> {
        real(dir, true)?;
        need(fs::read_dir(dir).map_err(err)?.count() == 5,
            "not exactly five PE in protected generation")?;
        let mut found=BTreeMap::new();
        for name in NAMES {
            let path=dir.join(name);
            real(&path, false)?;
            let mut f=fs::File::open(&path).map_err(err)?;
            let mut mz=[0u8;2];
            use std::io::Read;
            f.read_exact(&mut mz).map_err(err)?;
            need(&mz==b"MZ" && f.metadata().map_err(err)?.len()>=512,
                "not genuine MZ/PE")?;
            found.insert(name.to_string(), sha(&path)?);
        }
        Ok(found)
    }
    fn old_flat(root: &Path, old: &BTreeMap<String, String>) -> Result<(), String> {
        for (name,digest) in old {
            need(sha(&root.join(name))? == *digest,
                "legacy flat fallback changed during transaction")?;
        }
        Ok(())
    }
    fn witness(
        engine: &UpdateEngine, signed: &SignedFive, root: &Path,
        candidate: &Path, old: &BTreeMap<String, String>, mode: &str,
    ) -> Result<(), String> {
        let current=root.join("current");
        let tx=format!("video18c02b03-{mode}");
        let control=engine.begin_verified_candidate(
            &tx, signed.version(), candidate, Path::new("akula-app.exe")
        ).map_err(err)?;
        need(sha(&control)? == old["akula-app.exe"],
            "control updater was not copied from OLD generation")?;
        need(engine.read_journal().map_err(err)?
            .is_some_and(|j|j.phase==UpdatePhase::Prepared),
            "rar2 journal did not reach prepared")?;
        engine.retain_previous().map_err(err)?;
        engine.activate_candidate().map_err(err)?;
        need(engine.read_journal().map_err(err)?
            .is_some_and(|j|j.phase==UpdatePhase::CandidateActivated),
            "rar2 journal did not reach activated")?;
        signed.verify_stage(&current)?;
        old_flat(root, old)?;
        let generation=if mode=="wrong-generation" {
            if signed.generation_sha256()=="f".repeat(64) { "0".repeat(64) }
            else { "f".repeat(64) }
        } else { signed.generation_sha256().to_owned() };
        let check=|receipt:&[u8],nonce:&str,pid:u32| {
            signed.verify_first_presented_frame_ack(receipt,nonce,pid).is_ok()
        };
        // The actual protected current/Akula GUI must render a visible window,
        // return a private OS HANDLE ACK and exit. A synthetic receipt is never
        // accepted; any launch/error also fails the overall admission.
        let result=super::gui_launcher::lab::case(
            &current.join("akula-app.exe"),&generation,mode,Some(&check)
        );
        let accepted=match result {
            Ok(v)=>v,
            Err(e)=>{
                let recovery=engine.recover().map_err(err)?;
                return Err(format!(
                    "VIDEO18C02B03_HOLD: actual GUI witness failed ({e}); recovery={recovery:?}"
                ));
            }
        };
        if mode=="good" {
            if !accepted {
                engine.recover().map_err(err)?;
                return Err("VIDEO18C02B03_HOLD: authentic real GUI ACK rejected".into());
            }
            signed.verify_stage(&current)?;
            old_flat(root,old)?;
            need(five(&current)? != *old, "candidate does not change real PE")?;
            // Only this exact positive path can confirm. Neither signed bytes
            // alone nor a negative/forged GUI receipt authorize commit.
            engine.confirm_candidate().map_err(err)?;
            need(engine.read_journal().map_err(err)?.is_none(),
                "rar2 confirmed transaction still has active journal")?;
            signed.verify_stage(&current)?;
            old_flat(root,old)?;
            println!(
                "VIDEO18C02B03_POSITIVE_ACK_CONFIRM_PASS rar2_activated=1 \
                 real_gui=1 signed_manifest=1 first_frame_ack=1 exact_five=1 \
                 confirm_candidate=1 old_flat_unchanged=1 release=0"
            );
        } else {
            let recovery=engine.recover().map_err(err)?;
            need(!accepted, "invalid real GUI receipt was accepted")?;
            need(recovery==RecoveryOutcome::UnconfirmedCandidateRolledBack,
                "negative GUI ACK did not trigger rar2 rollback")?;
            need(five(&current)? == *old, "rollback left a mixed/changed old generation")?;
            old_flat(root,old)?;
            println!(
                "VIDEO18C02B03_NEGATIVE_ACK_ROLLBACK_PASS mode={mode} \
                 real_gui=1 refused=1 five_old_restored=1 release=0"
            );
        }
        engine.cleanup_orphaned_transactions().map_err(err)?;
        Ok(())
    }
    pub fn run()->Result<(),String> {
        super::gui_launcher::lab::guarded_pr()?;
        let args:Vec<String>=env::args().collect();
        if args.len()==8 && args[1]=="--crash-child" {
            return process_crash::child(&args);
        }
        need(args.len()==6, "expected root, stage, manifest, signature and test pubkey")?;
        let expected=env::var("EXPECTED_HEAD_SHA").map_err(err)?;
        need(expected.len()==40 && expected.bytes().all(|c|c.is_ascii_hexdigit()),
            "missing exact PR SHA")?;
        let workspace=fs::canonicalize(env::var("GITHUB_WORKSPACE").map_err(err)?)
            .map_err(err)?;
        let runner=fs::canonicalize(env::var("RUNNER_TEMP").map_err(err)?).map_err(err)?;
        let program=fs::canonicalize(env::var("ProgramFiles").map_err(err)?).map_err(err)?;
        let root=fs::canonicalize(&args[1]).map_err(err)?;
        let leaf=root.file_name().and_then(|x|x.to_str()).ok_or("bad root name")?;
        let tag=leaf.strip_prefix(PREFIX).ok_or("non-lab protected root")?;
        need(root.parent()==Some(program.as_path()) && tag.len()==32
            && tag.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c)),
            "not the disposable protected root")?;
        need(fs::read_dir(&root).map_err(err)?.count()==7,
            "protected root has non-baseline entries before rar2")?;
        let app=super::gui_launcher::lab::validate_app(
            root.join("current").join("akula-app.exe").to_str().ok_or("bad current app path")?
        )?;
        need(app==root.join("current").join("akula-app.exe"),
            "GUI executable is not the expected protected current")?;
        let stage=fs::canonicalize(&args[2]).map_err(err)?;
        let work=stage.parent().ok_or("stage without parent")?;
        let workleaf=work.file_name().and_then(|s|s.to_str()).ok_or("work name")?;
        need(work.parent()==Some(runner.as_path())
            && workleaf==format!("Video18c02b02b02b-{tag}")
            && stage.file_name().and_then(|x|x.to_str())==Some("candidate"),
            "candidate escaped unique runner stage")?;
        let trust=work.join("new-trust");
        let manifest=fs::canonicalize(&args[3]).map_err(err)?;
        let signature=fs::canonicalize(&args[4]).map_err(err)?;
        let pubkey=fs::canonicalize(&args[5]).map_err(err)?;
        need(manifest==trust.join("five-manifest.json")
            && signature==trust.join("five-manifest.minisig")
            && pubkey==trust.join("test-publisher.pub"),
            "signed manifest/key outside unique ephemeral trust directory")?;
        for p in [&manifest,&signature,&pubkey] { real(p,false)?; }
        let bytes=fs::read(&manifest).map_err(err)?;
        let sig=fs::read(&signature).map_err(err)?;
        let pubbytes=fs::read(&pubkey).map_err(err)?;
        need((1..=8192).contains(&bytes.len()) && (1..=4096).contains(&sig.len())
            && (1..=4096).contains(&pubbytes.len()),
            "oversized/empty signed witness inputs")?;
        let signed=admit_hosted_lab_signature(
            &bytes,&STANDARD.encode(&sig),&STANDARD.encode(&pubbytes),"0.3.20"
        )?;
        need(signed.version()=="0.3.21" && signed.source_commit()==expected,
            "candidate signed version/source mismatch")?;
        let locksha=sha(&workspace.join("aquilum-app/Cargo.lock"))?;
        need(signed.cargo_lock_sha256()==locksha,
            "signed Cargo.lock source mismatch")?;
        signed.verify_stage(&stage)?;
        let old=five(&root.join("current"))?;
        need(old==five(&root.join("current"))?,"baseline unstable")?;
        old_flat(&root,&old)?;
        need(five(&stage)? != old,"old and new real five-PE sets identical")?;
        need(signed.verify_stage(&root.join("current")).is_err(),
            "old generation accepted under new signature")?;
        // A real separate controller process is forcibly terminated at four
        // durable rar2 phases; it never confirms merely because GUI ACK arrived.
        // The following existing positive/negative GUI tests run only AFTER
        // all four independent lock-reacquired recoveries restore old five PE.
        process_crash::matrix(&root,&stage,&manifest,&signature,&pubkey,&signed,&old)?;
        let oslock=InstallLock::try_acquire(&root).map_err(err)?;
        need(matches!(
            InstallLock::try_acquire(&root),Err(OrchestrationError::LockBusy)
        ),"second OS installation lock acquired")?;
        let engine=UpdateEngine::new(&root);
        need(engine.read_journal().map_err(err)?.is_none(),
            "unsafe existing update journal in fresh protected lab")?;
        for mode in ["wrong-nonce","wrong-generation","good"] {
            signed.verify_stage(&stage)?;
            witness(&engine,&signed,&root,&stage,&old,mode)?;
        }
        let after=five(&root.join("current"))?;
        need(after==five(&stage)?,"confirmed candidate contains wrong five PE")?;
        old_flat(&root,&old)?;
        need(engine.read_journal().map_err(err)?.is_none(),
            "active journal survived positive commit")?;
        drop(oslock);
        for name in [
            "update-journal.json.prev","update-journal.json.next",
            ".chaptera-install.lock"
        ] {
            let file=root.join(name);
            if file.exists(){fs::remove_file(file).map_err(err)?;}
        }
        for dirname in [".staging",".rollback"] {
            let dir=root.join(dirname);
            if dir.exists(){fs::remove_dir(dir).map_err(err)?;}
        }
        need(fs::read_dir(&root).map_err(err)?.count()==7,
            "rar2 left an uninstall-breaking protected root")?;
        println!(
            "VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS \
             distinct_source_gui=1 signed_five=1 os_lock=1 negative_ack_rollback=2 \
             positive_first_frame_ack=1 confirm_candidate=1 old_flat_intact=1 release=0"
        );
        Ok(())
    }
}
#[cfg(windows)]
fn main(){
    if let Err(e)=lab::run(){
        eprintln!("VIDEO18C02B03_HOLD: {e}");
        std::process::exit(1);
    }
}
