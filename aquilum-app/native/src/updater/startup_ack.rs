//! P0-C6d: generation-bound startup acknowledgement for macOS updates.
//! An accepted OS launch alone never authorizes deletion of the old app.
//! Acknowledgement is written only by an installed app after its first
//! rendered/presented graphical frame; its backup stays until a later gate.
//! Cooperative same-user filesystem assumption, NOT publisher attestation.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::macos_transaction::AppGate;

const PENDING: &str = ".akula-updater.pending.v1";
const ACKED: &str = ".akula-updater.acked.v1";
const PENDING_HEADER: &str = "AKULA-PENDING-STARTUP-V1";
const ACK_HEADER: &str = "AKULA-STARTUP-ACK-V1";
const MAX_RECEIPT: u64 = 4096;
const MAX_EXECUTABLE: u64 = 256 * 1024 * 1024;
const MAX_PLIST: u64 = 1024 * 1024;
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Generation {
    nonce: String,
    executable: String,
    info: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    NoPending,
    Recorded,
    AlreadyRecorded,
}

fn regular_file(path: &Path, max: u64) -> Result<u64, String> {
    let info = fs::symlink_metadata(path).map_err(|e| format!("missing file {}: {e}", path.display()))?;
    if !info.file_type().is_file() || info.file_type().is_symlink() || info.len() == 0 || info.len() > max {
        return Err(format!("unsafe, empty or oversized file: {}", path.display()));
    }
    Ok(info.len())
}

fn file_hash(path: &Path, max: u64) -> Result<String, String> {
    let expected = regular_file(path, max)?;
    let mut reader = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let len = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        total = total.checked_add(len as u64).ok_or("file length overflow")?;
        if total > max {
            return Err("file grew beyond startup proof size budget".into());
        }
        hasher.update(&buf[..len]);
    }
    if total != expected {
        return Err("file modified while generating startup proof".into());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn stage_nonce(stage: &Path) -> Result<&str, String> {
    let name = stage.file_name().and_then(|name| name.to_str()).ok_or("invalid staging pathname")?;
    let nonce = name.strip_prefix(".akula-install.").ok_or("stage lacks Akula namespace")?;
    if !(16..=64).contains(&nonce.len())
        || !nonce.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return Err("invalid immutable updater generation nonce".into());
    }
    Ok(nonce)
}

fn identity(app: &Path, nonce: &str) -> Result<Generation, String> {
    Ok(Generation {
        nonce: nonce.to_owned(),
        executable: file_hash(&app.join("Contents/MacOS/aquilum-app"), MAX_EXECUTABLE)?,
        info: file_hash(&app.join("Contents/Info.plist"), MAX_PLIST)?,
    })
}

fn encode(record: &Generation, marker: &str) -> String {
    format!(
        "{marker}\n{}\n{}\n{}\n",
        record.nonce, record.executable, record.info
    )
}

fn read_receipt(path: &Path, marker: &str) -> Result<Generation, String> {
    let _ = regular_file(path, MAX_RECEIPT)?;
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut lines = raw.lines();
    let head = lines.next().ok_or("missing receipt header")?;
    let nonce = lines.next().ok_or("missing receipt nonce")?;
    let executable = lines.next().ok_or("missing executable digest")?;
    let info = lines.next().ok_or("missing Info.plist digest")?;
    if head != marker
        || !raw.ends_with('\n')
        || lines.next().is_some()
        || !(16..=64).contains(&nonce.len())
        || !nonce.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        || ![executable, info]
            .iter()
            .all(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("corrupt, ambiguous or unsupported startup receipt".into());
    }
    Ok(Generation {
        nonce: nonce.to_owned(),
        executable: executable.to_owned(),
        info: info.to_owned(),
    })
}

fn save_once(path: &Path, content: &str) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("refusing to replace startup receipt: {e}"))?;
    output.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

/// Must be called with the AppGate lock held, after bounded TAR unpack
/// and before any old->backup rename.
pub(super) fn record_candidate(stage: &Path) -> Result<(), String> {
    let nonce = stage_nonce(stage)?;
    let candidate = stage.join("Aquilum.app");
    let meta = fs::symlink_metadata(&candidate).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
        return Err("new candidate is not a regular application directory".into());
    }
    let record = identity(&candidate, nonce)?;
    save_once(&stage.join(PENDING), &encode(&record, PENDING_HEADER))
}

/// Verify exact native executable and Info.plist bytes under shared OS lock.
/// The result never removes the previous version or resolves a HOLD.
fn acknowledge(gate: &AppGate, running_exe: &Path) -> Result<Outcome, String> {
    let Some(stage) = gate.startup_stage()? else {
        return Ok(Outcome::NoPending);
    };
    let nonce = stage_nonce(&stage)?;
    let backup = stage.join("Aquilum.previous.app");
    let prior = fs::symlink_metadata(&backup).map_err(|e| format!("missing previous bundle: {e}"))?;
    if !prior.file_type().is_dir() || prior.file_type().is_symlink() {
        return Err("startup ACK requires a real previous version backup".into());
    }
    for item in fs::read_dir(&stage).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let known = item.file_name();
        if known.as_os_str() != std::ffi::OsStr::new("Aquilum.previous.app")
            && known.as_os_str() != std::ffi::OsStr::new(PENDING)
            && known.as_os_str() != std::ffi::OsStr::new(ACKED)
        {
            return Err(format!("unknown updater stage entry: {}", item.path().display()));
        }
    }
    let bundle = gate.app_path();
    let binary = bundle.join("Contents/MacOS/aquilum-app");
    let running = fs::canonicalize(running_exe).map_err(|e| e.to_string())?;
    let expected = fs::canonicalize(&binary).map_err(|e| e.to_string())?;
    if running != expected {
        return Err("startup ACK refused: process is not the installed Akula binary".into());
    }
    let pending = read_receipt(&stage.join(PENDING), PENDING_HEADER)?;
    if pending.nonce != nonce {
        return Err("pending startup receipt belongs to another generation".into());
    }
    let actual = identity(bundle, nonce)?;
    if pending != actual {
        return Err("installed binary or Info.plist differs from pending update generation".into());
    }
    let ack_path = stage.join(ACKED);
    match fs::symlink_metadata(&ack_path) {
        Ok(_) => {
            let prior = read_receipt(&ack_path, ACK_HEADER)?;
            if prior != actual {
                return Err("existing ACK conflicts with installed generation".into());
            }
            return Ok(Outcome::AlreadyRecorded);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    save_once(&ack_path, &encode(&actual, ACK_HEADER))?;
    Ok(Outcome::Recorded)
}

pub(super) fn acknowledge_rendered_gui_startup() -> Result<Outcome, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let home = std::env::var_os("HOME").ok_or("macOS HOME not set")?;
    let current = exe
        .to_string_lossy()
        .split_once(".app/Contents/MacOS/")
        .map(|(root, _)| PathBuf::from(format!("{root}.app")))
        .ok_or("startup executable is not in a packaged .app")?;
    let gate = AppGate::acquire(&current, Path::new(&home))?;
    acknowledge(&gate, &exe)
}

/// Called once AFTER a nonempty CPU frame reached the output surface.
pub(super) fn on_first_presented_frame() {
    if FIRST_FRAME.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        for attempt in 0..80 {
            match acknowledge_rendered_gui_startup() {
                Ok(Outcome::NoPending | Outcome::AlreadyRecorded) => return,
                Ok(Outcome::Recorded) => {
                    eprintln!("Akula: first-render startup ACK recorded for exact installed generation; backup retained");
                    return;
                }
                Err(err) if err.contains("holds the shared lock") && attempt < 79 => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(err) => {
                    eprintln!("Akula startup ACK on HOLD; old backup preserved: {err}");
                    return;
                }
            }
        }
    });
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        home: PathBuf,
        app: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = ID.fetch_add(1, Ordering::Relaxed);
            let home = std::env::temp_dir().join(format!(
                "akula-startup-ack-{}-{id}", std::process::id()
            ));
            let app = home.join("Applications/Aquilum.app");
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::write(app.join("Contents/Info.plist"), b"previous plist").unwrap();
            fs::write(app.join("Contents/MacOS/aquilum-app"), b"previous executable").unwrap();
            Self { home, app }
        }

        fn gate(&self) -> AppGate {
            AppGate::acquire(&self.app, &self.home).unwrap()
        }

        fn prepared(&self, gate: &AppGate) -> PathBuf {
            let stage = gate.new_stage("1234567890abcdef1234567890abcdef").unwrap();
            let candidate = stage.join("Aquilum.app");
            fs::create_dir_all(candidate.join("Contents/MacOS")).unwrap();
            fs::write(candidate.join("Contents/Info.plist"), b"new plist").unwrap();
            fs::write(candidate.join("Contents/MacOS/aquilum-app"), b"new executable").unwrap();
            record_candidate(&stage).unwrap();
            stage
        }

        fn promoted(&self) -> (AppGate, PathBuf) {
            let gate = self.gate();
            let stage = self.prepared(&gate);
            gate.promote(&stage).unwrap();
            (gate, stage)
        }

        fn running(&self) -> PathBuf {
            self.app.join("Contents/MacOS/aquilum-app")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    #[test]
    fn no_pending_generation_does_not_create_ack() {
        let fixture = Fixture::new();
        let gate = fixture.gate();
        assert_eq!(acknowledge(&gate, &fixture.running()).unwrap(), Outcome::NoPending);
    }

    #[test]
    fn exact_new_binary_ack_preserves_old_backup() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        assert_eq!(acknowledge(&gate, &fixture.running()).unwrap(), Outcome::Recorded);
        assert!(stage.join(PENDING).is_file());
        assert!(stage.join(ACKED).is_file());
        assert_eq!(
            fs::read(stage.join("Aquilum.previous.app/Contents/MacOS/aquilum-app")).unwrap(),
            b"previous executable"
        );
        assert!(gate.recover().is_err(), "ACK must not silently clear backup HOLD");
    }

    #[test]
    fn repeated_ack_is_idempotent_but_retains_old_app() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        assert_eq!(acknowledge(&gate, &fixture.running()).unwrap(), Outcome::Recorded);
        assert_eq!(acknowledge(&gate, &fixture.running()).unwrap(), Outcome::AlreadyRecorded);
        assert!(stage.join("Aquilum.previous.app").is_dir());
    }

    #[test]
    fn missing_pending_receipt_does_not_accept_new_app() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::remove_file(stage.join(PENDING)).unwrap();
        let error = acknowledge(&gate, &fixture.running()).unwrap_err();
        assert!(error.contains("missing file"));
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn damaged_pending_marker_refuses_ack() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::write(stage.join(PENDING), b"broken\n").unwrap();
        assert!(acknowledge(&gate, &fixture.running()).is_err());
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn foreign_nonce_receipt_is_not_reused() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        let receipt = stage.join(PENDING);
        let original = fs::read_to_string(&receipt).unwrap();
        fs::write(&receipt, original.replace(
            "1234567890abcdef1234567890abcdef",
            "ffffffffffffffffffffffffffffffff",
        )).unwrap();
        assert!(acknowledge(&gate, &fixture.running()).unwrap_err().contains("another generation"));
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn modified_installed_binary_is_not_original_candidate() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::write(fixture.running(), b"tampered new executable").unwrap();
        assert!(acknowledge(&gate, &fixture.running()).unwrap_err().contains("differs"));
        assert!(!stage.join(ACKED).exists());
        assert!(stage.join("Aquilum.previous.app").is_dir());
    }

    #[test]
    fn modified_installed_plist_is_not_original_candidate() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::write(fixture.app.join("Contents/Info.plist"), b"tampered plist").unwrap();
        assert!(acknowledge(&gate, &fixture.running()).unwrap_err().contains("differs"));
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn another_running_executable_cannot_ack_installed_bundle() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        let other = fixture.home.join("another-executable");
        fs::write(&other, b"same binary on a different path").unwrap();
        assert!(acknowledge(&gate, &other).unwrap_err().contains("not the installed"));
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn unexpected_stage_entry_forces_hold() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::write(stage.join("foreign.txt"), b"preserve").unwrap();
        assert!(acknowledge(&gate, &fixture.running()).unwrap_err().contains("unknown"));
        assert_eq!(fs::read(stage.join("foreign.txt")).unwrap(), b"preserve");
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn contradictory_ack_receipt_cannot_be_overwritten() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::write(stage.join(ACKED), b"bogus").unwrap();
        assert!(acknowledge(&gate, &fixture.running()).is_err());
        assert_eq!(fs::read(stage.join(ACKED)).unwrap(), b"bogus");
    }

    #[test]
    fn no_old_backup_means_no_ack_even_with_new_app() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::rename(stage.join("Aquilum.previous.app"), stage.join("Elsewhere.app")).unwrap();
        assert!(acknowledge(&gate, &fixture.running()).is_err());
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn multiple_generations_block_ack_without_choosing_one() {
        let fixture = Fixture::new();
        let (gate, stage) = fixture.promoted();
        fs::create_dir(fixture.home.join("Applications/.akula-install.ffffffffffffffff")).unwrap();
        assert!(acknowledge(&gate, &fixture.running()).unwrap_err().contains("multiple"));
        assert!(!stage.join(ACKED).exists());
    }

    #[test]
    fn receipt_creation_never_replaces_existing_file() {
        let fixture = Fixture::new();
        let gate = fixture.gate();
        let stage = fixture.prepared(&gate);
        assert!(record_candidate(&stage).is_err());
        assert!(stage.join(PENDING).is_file());
    }

    #[test]
    fn startup_ack_does_not_approve_a_stage_before_promotion() {
        let fixture = Fixture::new();
        let gate = fixture.gate();
        let stage = fixture.prepared(&gate);
        assert!(acknowledge(&gate, &fixture.running()).is_err());
        assert!(!stage.join(ACKED).exists());
    }
}
