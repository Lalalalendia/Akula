//! VIDEO-18b-18b: nonshipping Windows controller witness.
//! The test process is external to current/, elevated only on an ephemeral,
//! GitHub-hosted PR runner. It never executes a candidate or confirms an ACK.
#![cfg(windows)]
#[cfg(test)]
mod acceptance {
    use akula_video18b18_signed_five::admit_hosted_lab_signature;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdatePhase};
    use chaptera_update_orchestrator::{InstallLock, OrchestrationError};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::env;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};

    const FILES: [&str; 5] = [
        "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
        "akula-yt-dlp.exe", "akula-quickjs.exe",
    ];
    const PREFIX: &str = "AkulaVideo18bRealNsisLab-";
    const CANDIDATE: &str = "video18b18b-candidate";
    const SIGNED_DIR: &str = "video18b18b-trust";

    fn required(name: &str) -> String {
        env::var(name).unwrap_or_else(|_| panic!("VIDEO18B18B_HOLD: missing {name}"))
    }

    fn check_real(path: &Path, dir: bool) {
        let meta=fs::symlink_metadata(path).expect("VIDEO18B18B_HOLD: path missing");
        assert!(!meta.file_type().is_symlink(), "VIDEO18B18B_HOLD: filesystem alias");
        assert!(if dir {meta.is_dir()} else {meta.is_file()},
            "VIDEO18B18B_HOLD: unexpected file type: {}",path.display());
        use std::os::windows::fs::MetadataExt;
        assert_eq!(meta.file_attributes() & 0x400,0,
            "VIDEO18B18B_HOLD: unexpected Windows reparse point");
    }

    fn digest(path: &Path) -> (u64,String) {
        check_real(path, false);
        let mut file=File::open(path).unwrap();
        let len=file.metadata().unwrap().len();
        assert!((512..=512*1024*1024).contains(&len), "VIDEO18B18B_HOLD: PE size");
        let mut mz=[0u8;2];
        file.read_exact(&mut mz).unwrap();
        assert_eq!(&mz,b"MZ","VIDEO18B18B_HOLD: missing real PE header");
        let mut hash=Sha256::new();
        hash.update(mz);
        let mut n=2;
        let mut buf=[0u8;64*1024];
        loop {
            let count=file.read(&mut buf).unwrap();
            if count==0 {break}
            hash.update(&buf[..count]);
            n+=count as u64;
        }
        assert_eq!(n,len,"VIDEO18B18B_HOLD: PE grew during digest");
        (len,hash.finalize().iter().map(|b|format!("{b:02x}")).collect())
    }

    fn hashes(dir: &Path)->BTreeMap<String,(u64,String)> {
        check_real(dir,true);
        let items=fs::read_dir(dir).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        assert_eq!(items.len(),5,"VIDEO18B18B_HOLD: exactly five files required");
        let mut out=BTreeMap::new();
        for name in FILES {out.insert(name.to_owned(),digest(&dir.join(name)));}
        out
    }

    fn guarded_roots()->(PathBuf,PathBuf,PathBuf,PathBuf) {
        for (name,value) in [
            ("GITHUB_ACTIONS","true"),("RUNNER_OS","Windows"),
            ("RUNNER_ENVIRONMENT","github-hosted"),("GITHUB_EVENT_NAME","pull_request"),
        ] {
            assert_eq!(required(name),value,"VIDEO18B18B_HOLD: hosted Windows PR only");
        }
        let pf=fs::canonicalize(required("ProgramFiles")).unwrap();
        let root_raw=PathBuf::from(required("AKULA_VIDEO18B18B_INSTALL_ROOT"));
        check_real(&root_raw,true);
        let root=fs::canonicalize(&root_raw).unwrap();
        let leaf=root.file_name().unwrap().to_str().unwrap();
        let tag=leaf.strip_prefix(PREFIX).expect("VIDEO18B18B_HOLD: wrong install root");
        assert!(
            root.parent()==Some(pf.as_path()) && tag.len()==32 &&
            tag.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase()),
            "VIDEO18B18B_HOLD: only a fresh GUID lab root under Program Files");
        let tmp=fs::canonicalize(required("RUNNER_TEMP")).unwrap();
        let stage_raw=PathBuf::from(required("AKULA_VIDEO18B18B_STAGE_ROOT"));
        check_real(&stage_raw,true);
        let stage=fs::canonicalize(&stage_raw).unwrap();
        assert_eq!(stage.parent(),Some(tmp.as_path()));
        assert_eq!(stage.file_name().unwrap().to_str().unwrap(),format!("Video18b16Stage-{tag}"));
        let candidate=stage.join(CANDIDATE);
        let trust=stage.join(SIGNED_DIR);
        check_real(&candidate,true);
        check_real(&trust,true);
        assert_eq!(candidate.parent(),Some(stage.as_path()));
        assert_eq!(trust.parent(),Some(stage.as_path()));
        check_real(&root.join("uninstall.exe"),false);
        (root,stage,candidate,trust)
    }

    fn corrupt_byte(path:&Path, original:Option<u8>)->u8 {
        let mut file=OpenOptions::new().read(true).write(true).open(path).unwrap();
        file.seek(SeekFrom::Start(1024)).unwrap();
        let mut byte=[0u8];
        file.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Start(1024)).unwrap();
        file.write_all(&[original.unwrap_or(byte[0]^0x40)]).unwrap();
        file.sync_all().unwrap();
        byte[0]
    }

    #[test]
    fn exact_signed_five_real_pe_cannot_reach_journal_without_lock_and_admission() {
        let (root,_stage,candidate,trust)=guarded_roots();
        let old=hashes(&root.join("current"));
        for name in FILES {
            assert_eq!(digest(&root.join(name)),old[name],
                "VIDEO18B18B_HOLD: legacy fallback not byte-identical");
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(),7,
            "VIDEO18B18B_HOLD: root not a clean two-generation NSIS install");
        let new=hashes(&candidate);
        for name in FILES {
            assert_ne!(new[name],old[name],"VIDEO18B18B_HOLD: unsigned trial generation not distinct");
        }

        // All on-disk admission is performed while holding the actual pinned
        // rar2 installation lock, never with a synthetic PID-file mutex.
        let owner=InstallLock::try_acquire(&root).unwrap();
        assert!(matches!(InstallLock::try_acquire(&root),Err(OrchestrationError::LockBusy)),
            "VIDEO18B18B_HOLD: concurrent install owner was not denied");
        let engine=UpdateEngine::new(&root);
        assert!(engine.read_journal().unwrap().is_none());

        let raw=fs::read(trust.join("five-manifest.json")).unwrap();
        let signature=STANDARD.encode(fs::read(trust.join("five-manifest.minisig")).unwrap());
        let public_key=STANDARD.encode(fs::read(trust.join("test-publisher.pub")).unwrap());
        let other=STANDARD.encode(fs::read(trust.join("other.pub")).unwrap());
        assert!(admit_hosted_lab_signature(&raw,&signature,&other,"0.3.20").is_err());
        assert!(admit_hosted_lab_signature(&raw,&signature,&public_key,"0.3.21").is_err());
        let mut altered=raw.clone();
        altered.push(b' ');
        assert!(admit_hosted_lab_signature(&altered,&signature,&public_key,"0.3.20").is_err());
        assert!(engine.read_journal().unwrap().is_none(),
            "VIDEO18B18B_HOLD: untrusted bytes opened an update journal");

        let signed=admit_hosted_lab_signature(&raw,&signature,&public_key,"0.3.20").unwrap();
        assert_eq!(signed.version(),"0.3.21");
        assert_eq!(signed.source_commit(),required("EXPECTED_HEAD_SHA"));
        assert_eq!(signed.cargo_lock_sha256(),required("AKULA_VIDEO18B18B_CARGO_LOCK_SHA256"));
        let manifest_hash:String=Sha256::digest(&raw).iter()
            .map(|b|format!("{b:02x}")).collect();
        assert_eq!(signed.generation_sha256(),manifest_hash);
        signed.verify_stage(&candidate).unwrap();
        assert!(signed.verify_stage(&root.join("current")).is_err(),
            "VIDEO18B18B_HOLD: old generation falsely authenticated as new");

        // Corruption after signing and an extra file must fail with no journal.
        let first=candidate.join(FILES[0]);
        let previous=corrupt_byte(&first,None);
        assert!(signed.verify_stage(&candidate).is_err(),
            "VIDEO18B18B_HOLD: same-size tampered real PE accepted");
        corrupt_byte(&first,Some(previous));
        signed.verify_stage(&candidate).unwrap();
        let extra=candidate.join("extra.exe");
        fs::write(&extra,b"MZ").unwrap();
        assert!(signed.verify_stage(&candidate).is_err(),
            "VIDEO18B18B_HOLD: extra PE passed signed identity");
        fs::remove_file(extra).unwrap();
        assert!(engine.read_journal().unwrap().is_none());
        assert_eq!(hashes(&candidate),new);

        // This is a controlled simulation of a handoff in ONE privileged test
        // process. The copied control is not launched; no real user ACK exists.
        let transaction="video18b18b-signed-five";
        let updater=Path::new("akula-app.exe");
        let copied=engine.begin_verified_candidate(transaction,signed.version(),&candidate,updater)
            .unwrap();
        assert_eq!(digest(&copied),old[FILES[0]],
            "VIDEO18B18B_HOLD: copied old control is wrong generation");
        let paths=engine.paths_for(transaction,updater).unwrap();
        assert_eq!(engine.read_journal().unwrap().unwrap().phase,UpdatePhase::Prepared);
        signed.verify_stage(&paths.staged_candidate).unwrap();
        engine.retain_previous().unwrap();
        engine.activate_candidate().unwrap();
        assert_eq!(engine.read_journal().unwrap().unwrap().phase,UpdatePhase::CandidateActivated);
        signed.verify_stage(&root.join("current")).unwrap();
        assert_eq!(hashes(&root.join("current")),new);
        for name in FILES {assert_eq!(digest(&root.join(name)),old[name]);}

        // No rendered UI frame or generation-bound ACK was seen. A signed PE
        // identity alone MUST NOT turn this into confirm_candidate().
        drop(engine);
        let recovery=UpdateEngine::new(&root);
        assert_eq!(recovery.recover().unwrap(),RecoveryOutcome::UnconfirmedCandidateRolledBack);
        assert!(recovery.read_journal().unwrap().is_none());
        assert_eq!(hashes(&root.join("current")),old);
        for name in FILES {assert_eq!(digest(&root.join(name)),old[name]);}
        recovery.cleanup_orphaned_transactions().unwrap();

        for dirname in [".staging",".rollback"] {
            let p=root.join(dirname);
            if p.exists(){fs::remove_dir(p).unwrap();}
        }
        drop(owner);
        let again=InstallLock::try_acquire(&root).unwrap();
        drop(again);
        for name in [
            "update-journal.json.prev","update-journal.json.next",".chaptera-install.lock"
        ] {
            let p=root.join(name);
            if p.exists(){fs::remove_file(p).unwrap();}
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(),7,
            "VIDEO18B18B_HOLD: control/journal leftovers break dual NSIS uninstall");
        println!(
            "VIDEO18B18B_SIGNED_REAL_PE_PROTECTED_ROLLBACK_PASS \
             signed_manifest=1 actual_PE=5 wrong_key_denied=1 wrong_version_denied=1 \
             same_size_tamper_denied=1 OS_lock=1 activated_then_rolled_back=1 \
             first_frame_ACK=0 release=0"
        );
    }
}
