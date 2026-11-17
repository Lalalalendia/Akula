//! VIDEO-18b-15 non-release Akula adapter against pinned rar2 UpdateEngine.
//! This source is copied into an isolated rar2 checkout by a PR-only CI job.
#![cfg(windows)]
use chaptera_update_engine::{RecoveryOutcome, UpdateEngine};
use std::{fs, path::{Path, PathBuf}};

const COMPONENTS: [&str; 5] = [
    "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
    "akula-yt-dlp.exe", "akula-quickjs.exe",
];

fn seed(dir: &Path, generation: &str) {
    fs::create_dir_all(dir).unwrap();
    for name in COMPONENTS {
        fs::write(dir.join(name), format!("MZ-VIDEO18B15-{generation}-{name}")).unwrap();
    }
}
fn exact_five(dir: &Path) -> Result<(), String> {
    let files = fs::read_dir(dir).map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if files.len() != 5 { return Err("VIDEO18B15_HOLD: wrong component count".into()); }
    for item in files {
        let name = item.file_name();
        let name = name.to_str().ok_or("invalid name")?;
        if !COMPONENTS.contains(&name) { return Err("VIDEO18B15_HOLD: extra component".into()); }
        let m = fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
        if !m.file_type().is_file() || m.file_type().is_symlink() || m.len() < 8 {
            return Err("VIDEO18B15_HOLD: alias or empty component".into());
        }
    }
    Ok(())
}
// This is a synthetic shape check, NOT cryptographic publisher admission.
// The upstream engine requires its *caller* to authenticate and inspect input.
fn preflight(current: &Path, candidate: &Path) -> Result<(), String> {
    exact_five(current)?;
    exact_five(candidate)
}
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let tmp=tempfile::tempdir().unwrap();
    let root=tmp.path().join("install");
    let candidate=tmp.path().join("candidate");
    let note=tmp.path().join("user-note.md");
    seed(&root.join("current"), "OLD");
    seed(&candidate, "NEW");
    fs::write(&note,b"preserve private note\n").unwrap();
    (tmp,root,candidate,note)
}
fn assert_generation(dir: &Path, generation: &str) {
    exact_five(dir).unwrap();
    for name in COMPONENTS {
        assert_eq!(fs::read(dir.join(name)).unwrap(),
                   format!("MZ-VIDEO18B15-{generation}-{name}").as_bytes());
    }
}
fn begin(engine: &UpdateEngine, candidate: &Path, tx: &str) {
    preflight(&engine.current_dir(),candidate).unwrap();
    let controller=engine.begin_verified_candidate(
        tx, "0.3.99", candidate, Path::new("akula-app.exe")).unwrap();
    assert!(controller.exists());
}
#[test]
fn prepared_without_activation_keeps_old_and_user_data() {
    let (_tmp,root,candidate,note)=fixture();
    let engine=UpdateEngine::new(root);
    begin(&engine,&candidate,"akula-video15-prepared");
    assert_eq!(engine.recover().unwrap(),RecoveryOutcome::PreparedTransactionAborted);
    assert_generation(&engine.current_dir(),"OLD");
    assert_eq!(fs::read(note).unwrap(),b"preserve private note\n");
}
#[test]
fn retained_previous_before_activate_restores_five() {
    let (_tmp,root,candidate,note)=fixture();
    let engine=UpdateEngine::new(root);
    begin(&engine,&candidate,"akula-video15-retained");
    engine.retain_previous().unwrap();
    assert!(!engine.current_dir().exists());
    assert_eq!(engine.recover().unwrap(),RecoveryOutcome::UnconfirmedCandidateRolledBack);
    assert_generation(&engine.current_dir(),"OLD");
    assert_eq!(fs::read(note).unwrap(),b"preserve private note\n");
}
#[test]
fn akula_unconfirmed_new_generation_rolls_back_without_partial_mix() {
    let (_tmp,root,candidate,note)=fixture();
    let engine=UpdateEngine::new(root);
    begin(&engine,&candidate,"akula-video15-activated");
    engine.retain_previous().unwrap();
    engine.activate_candidate().unwrap();
    assert_generation(&engine.current_dir(),"NEW");
    assert_eq!(engine.recover().unwrap(),RecoveryOutcome::UnconfirmedCandidateRolledBack);
    assert_generation(&engine.current_dir(),"OLD");
    assert_eq!(fs::read(note).unwrap(),b"preserve private note\n");
}
#[test]
fn explicit_lab_confirmation_keeps_whole_new_generation() {
    let (_tmp,root,candidate,note)=fixture();
    let engine=UpdateEngine::new(root);
    begin(&engine,&candidate,"akula-video15-confirmed");
    engine.retain_previous().unwrap();
    engine.activate_candidate().unwrap();
    // TEST ONLY: no rendered-frame or publisher proof exists here.
    engine.confirm_candidate().unwrap();
    assert_generation(&engine.current_dir(),"NEW");
    assert_eq!(fs::read(note).unwrap(),b"preserve private note\n");
}
#[test]
fn akula_adapter_rejects_missing_or_extra_component_before_journal() {
    let (_tmp,root,candidate,_note)=fixture();
    let current=root.join("current");
    fs::remove_file(candidate.join("akula-quickjs.exe")).unwrap();
    assert!(preflight(&current,&candidate).is_err());
    fs::write(candidate.join("akula-quickjs.exe"),b"MZ-restore").unwrap();
    fs::write(candidate.join("extra.exe"),b"MZ-extra").unwrap();
    assert!(preflight(&current,&candidate).is_err());
    assert!(!root.join("update-journal.json").exists());
}
#[test]
fn flat_nsis_layout_is_not_implicitly_compatible_with_upstream_current_tree() {
    let tmp=tempfile::tempdir().unwrap();
    let root=tmp.path().join("flat-nsis");
    let candidate=tmp.path().join("candidate");
    seed(&root,"OLD");
    seed(&candidate,"NEW");
    let engine=UpdateEngine::new(&root);
    assert!(engine.begin_verified_candidate(
        "akula-video15-no-bridge","0.3.99",&candidate,Path::new("akula-app.exe")
    ).is_err());
    assert_generation(&root,"OLD");
    println!("VIDEO18B15_FLAT_NSIS_MIGRATION_HOLD");
}
