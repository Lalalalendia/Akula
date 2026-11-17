//! VIDEO-18b-13: experimental host-only five-component generation crash lab.
//! Compiled only by isolated Windows PR CI. NOT linked into shipping updater.
//! Proves rename/backup recovery from process exit, NOT power-loss durability,
//! signature admission, Windows installer integration or actual first-frame ACK.
#![cfg(target_os = "windows")]
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const COMPONENTS: [&str; 5] = [
    "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
    "akula-yt-dlp.exe", "akula-quickjs.exe",
];
const JOURNAL: &str = "VIDEO18B13-PREPARED-1:0123456789abcdef\n";
const ACK: &str = "VIDEO18B13-FIRST-FRAME-TEST-ONLY:0123456789abcdef\n";

fn safe_real_dir(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(format!("VIDEO18B13_HOLD: aliased or non-directory {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
fn check_components(dir: &Path, expected: &str) -> Result<(), String> {
    if !safe_real_dir(dir)? {
        return Err("VIDEO18B13_HOLD: generation directory missing".into());
    }
    let found = fs::read_dir(dir).map_err(|e| e.to_string())?.count();
    if found != COMPONENTS.len() {
        return Err("VIDEO18B13_HOLD: incomplete or extra five-component generation".into());
    }
    for name in COMPONENTS {
        let p = dir.join(name);
        let md = fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
        if !md.file_type().is_file() || md.file_type().is_symlink() {
            return Err("VIDEO18B13_HOLD: aliased generation component".into());
        }
        let bytes = fs::read(&p).map_err(|e| e.to_string())?;
        if bytes != format!("MZ-VIDEO18B13-{expected}-{name}").as_bytes() {
            return Err("VIDEO18B13_HOLD: generation component mismatch".into());
        }
    }
    Ok(())
}
fn durable_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f=OpenOptions::new().create_new(true).write(true).open(path)
        .map_err(|e| format!("VIDEO18B13_HOLD: no overwrite of receipt: {e}"))?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}
fn receipt(path: &Path, expected: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 256 => {
            if fs::read_to_string(path).map_err(|e| e.to_string())? != expected {
                Err("VIDEO18B13_HOLD: corrupt or mismatched generation receipt".into())
            } else {
                Ok(true)
            }
        }
        Ok(_) => Err("VIDEO18B13_HOLD: receipt alias or oversize".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
struct Gate {
    root: PathBuf,
    _lock: File,
}
impl Gate {
    fn acquire(root: &Path) -> Result<Self, String> {
        // Never permit running this test against real installation paths.
        if std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
            || std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request")
            || std::env::var("RUNNER_OS").as_deref() != Ok("Windows")
        {
            return Err("VIDEO18B13_HOLD: hosted Windows PR fixture required".into());
        }
        let temp = PathBuf::from(std::env::var_os("RUNNER_TEMP").ok_or("missing RUNNER_TEMP")?);
        let home = fs::canonicalize(&temp).map_err(|e| e.to_string())?;
        let sandbox = fs::canonicalize(root).map_err(|e| e.to_string())?;
        if !sandbox.starts_with(&home)
            || sandbox.parent() != Some(home.as_path())
            || !sandbox.file_name().and_then(|x| x.to_str())
                .is_some_and(|x| x.starts_with("AkulaVideo18b13-"))
            || !safe_real_dir(root)?
        {
            return Err("VIDEO18B13_HOLD: not an isolated throwaway fixture".into());
        }
        let lock = OpenOptions::new().read(true).write(true).create(true)
            .open(root.join("lock.v1")).map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|e| format!("VIDEO18B13_HOLD: concurrent updater denied: {e}"))?;
        Ok(Self {root: sandbox,_lock:lock})
    }
    fn live(&self)->PathBuf {self.root.join("active")}
    fn stage(&self)->PathBuf {self.root.join("candidate")}
    fn backup(&self)->PathBuf {self.root.join("previous")}
    fn failed(&self)->PathBuf {self.root.join("failed")}
    fn journal(&self)->PathBuf {self.root.join("journal.v1")}
    fn ack_path(&self)->PathBuf {self.root.join("ack.v1")}

    fn prepare_and_promote(&self,stop_after: &str) -> Result<(),String> {
        if safe_real_dir(&self.backup())? || safe_real_dir(&self.failed())?
            || self.journal().exists() || self.ack_path().exists() {
            return Err("VIDEO18B13_HOLD: cannot start while another generation is pending".into());
        }
        check_components(&self.live(),"OLD")?;
        check_components(&self.stage(),"NEW")?;
        durable_new(&self.journal(),JOURNAL.as_bytes())?;
        if stop_after == "after-receipt" {std::process::exit(73);}
        fs::rename(self.live(),self.backup()).map_err(|e| e.to_string())?;
        if stop_after == "after-backup" {std::process::exit(73);}
        fs::rename(self.stage(),self.live()).map_err(|e| e.to_string())?;
        if stop_after == "after-promote" {std::process::exit(73);}
        Ok(())
    }
    fn test_only_first_frame(&self) -> Result<(),String> {
        if !receipt(&self.journal(),JOURNAL)?
            || safe_real_dir(&self.stage())?
            || !safe_real_dir(&self.backup())? {
            return Err("VIDEO18B13_HOLD: cannot ACK a missing/ambiguous generation".into());
        }
        check_components(&self.live(),"NEW")?;
        check_components(&self.backup(),"OLD")?;
        durable_new(&self.ack_path(),ACK.as_bytes())?;
        Ok(())
    }
    fn recover(&self) -> Result<&'static str,String> {
        let journal = receipt(&self.journal(),JOURNAL)?;
        let ack = receipt(&self.ack_path(),ACK)?;
        let has_previous=safe_real_dir(&self.backup())?;
        let has_live=safe_real_dir(&self.live())?;
        let has_candidate=safe_real_dir(&self.stage())?;
        let has_failed=safe_real_dir(&self.failed())?;
        if !journal {
            if has_previous || ack || has_failed || !has_live {
                return Err("VIDEO18B13_HOLD: orphan generation without journal".into());
            }
            check_components(&self.live(),"OLD")?;
            return Ok("CLEAN_OLD");
        }
        if ack {
            if has_candidate || has_failed || !has_live {
                return Err("VIDEO18B13_HOLD: ACK does not identify active candidate".into());
            }
            check_components(&self.live(),"NEW")?;
            if has_previous {
                check_components(&self.backup(),"OLD")?;
                fs::remove_dir_all(self.backup()).map_err(|e| e.to_string())?;
            }
            // Delete journal before ack so crash cannot lose all commit proof.
            fs::remove_file(self.journal()).map_err(|e| e.to_string())?;
            fs::remove_file(self.ack_path()).map_err(|e| e.to_string())?;
            return Ok("COMMITTED_AFTER_ACK");
        }
        // No GUI ACK: never silently commit an unconfirmed generation.
        if has_previous {
            check_components(&self.backup(),"OLD")?;
            if has_live {
                check_components(&self.live(),"NEW")?;
                if has_failed {return Err("VIDEO18B13_HOLD: ambiguous failed generation".into());}
                fs::rename(self.live(),self.failed()).map_err(|e| e.to_string())?;
            }
            fs::rename(self.backup(),self.live()).map_err(|e| e.to_string())?;
            check_components(&self.live(),"OLD")?;
            return Ok("RESTORED_PREVIOUS");
        }
        if has_live && has_candidate && !has_failed {
            check_components(&self.live(),"OLD")?;
            check_components(&self.stage(),"NEW")?;
            return Ok("PREPARED_NOT_PROMOTED");
        }
        if has_live && has_failed && !has_candidate {
            check_components(&self.live(),"OLD")?;
            check_components(&self.failed(),"NEW")?;
            return Ok("RESTORED_PREVIOUS");
        }
        Err("VIDEO18B13_HOLD: unknown or untrusted generation state".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime,UNIX_EPOCH};
    fn new_fixture()->PathBuf {
        let t=std::env::var_os("RUNNER_TEMP").expect("hosted CI RUNNER_TEMP");
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root=PathBuf::from(t).join(format!("AkulaVideo18b13-{}-{stamp}",std::process::id()));
        fs::create_dir(&root).unwrap();
        for (dir,label) in [("active","OLD"),("candidate","NEW")] {
            fs::create_dir(root.join(dir)).unwrap();
            for name in COMPONENTS {
                fs::write(root.join(dir).join(name),
                    format!("MZ-VIDEO18B13-{label}-{name}")).unwrap();
            }
        }
        root
    }
    #[test]
    fn crash_child() {
        let Ok(stage) = std::env::var("VIDEO18B13_CHILD_STOP") else {return;};
        let root=PathBuf::from(std::env::var_os("VIDEO18B13_CHILD_ROOT").unwrap());
        Gate::acquire(&root).unwrap().prepare_and_promote(&stage).unwrap();
        panic!("child should exit at the selected journal boundary");
    }
    #[test]
    fn abrupt_child_exits_restore_old_exact_five_or_leave_old_intact() {
        for phase in ["after-receipt","after-backup","after-promote"] {
            let root=new_fixture();
            let status=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","tests::crash_child","--nocapture"])
                .env("VIDEO18B13_CHILD_STOP",phase)
                .env("VIDEO18B13_CHILD_ROOT",&root)
                .status().expect("spawn exact child test");
            assert_eq!(status.code(),Some(73),"real child must terminate without cleanup");
            let gate=Gate::acquire(&root).expect("OS lock must be released by process exit");
            let outcome=gate.recover().unwrap();
            assert_eq!(outcome,if phase=="after-receipt"{"PREPARED_NOT_PROMOTED"}else{"RESTORED_PREVIOUS"});
            check_components(&gate.live(),"OLD").unwrap();
            assert!(!safe_real_dir(&gate.backup()).unwrap());
            drop(gate);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn only_generation_bound_ack_can_commit_a_complete_five_component_set() {
        let root=new_fixture();
        let gate=Gate::acquire(&root).unwrap();
        assert!(Gate::acquire(&root).is_err(),"second updater cannot take the OS file lock");
        gate.prepare_and_promote("none").unwrap();
        assert!(gate.prepare_and_promote("none").is_err(),"pending journal denies a second update");
        gate.test_only_first_frame().unwrap();
        assert_eq!(gate.recover().unwrap(),"COMMITTED_AFTER_ACK");
        check_components(&gate.live(),"NEW").unwrap();
        assert!(!gate.backup().exists());
        assert!(!gate.journal().exists());
        drop(gate);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn wrong_ack_and_corrupt_previous_never_delete_backup() {
        let root=new_fixture();
        let gate=Gate::acquire(&root).unwrap();
        gate.prepare_and_promote("none").unwrap();
        durable_new(&gate.ack_path(),b"WRONG-GENERATION\n").unwrap();
        assert!(gate.recover().is_err(),"mismatched ACK cannot commit");
        check_components(&gate.backup(),"OLD").unwrap();
        fs::remove_file(gate.ack_path()).unwrap();
        fs::write(gate.backup().join("shiva-pdf-preview.exe"),b"TAMPER").unwrap();
        assert!(gate.recover().is_err(),"tampered previous generation cannot be restored");
        assert!(gate.backup().exists(),"blocked recovery preserves evidence");
        drop(gate);
        fs::remove_dir_all(root).unwrap();
    }
}
