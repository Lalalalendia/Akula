// Windows beta uses the same proven NSIS upgrade flags as upstream Aquilum.
// Unlike the inherited API, the executable entrypoint cannot accept arbitrary
// Vec<u8>: only Akula's publisher-authenticated SignedCandidate can reach it.
// This intentionally does NOT promise automatic crash rollback of NSIS.
#[cfg(target_os = "windows")]
pub(crate) fn apply(
    candidate: &super::release::SignedCandidate,
    before_exit: impl FnOnce(),
) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    use windows::core::{HSTRING, w};

    if !super::release::channel_ready() {
        return Err("HOLD: Akula signed Windows release channel is not published yet".into());
    }
    let installer = stage_signed_windows_setup(candidate.bytes(), candidate.sha256())?;
    // All signature verification, signed size checks and a disk rehash have
    // completed before saving state, closing the UI or executing an installer.
    before_exit();
    let started = unsafe {
        ShellExecuteW(
            None, w!("open"), &HSTRING::from(installer.as_os_str()),
            &HSTRING::from("/S /R /UPDATE /ARGS"), None, SW_SHOW,
        )
    };
    if started.0 as isize <= 32 {
        return Err(format!("Akula NSIS update could not start: {}", started.0 as isize));
    }
    // Let NSIS replace the per-user installed exe after old process handles
    // close. The beta installer is not a transactional rollback engine.
    std::process::exit(0);
}

#[cfg(target_os = "windows")]
fn stage_signed_windows_setup(bytes: &[u8], expected_sha256: &str) -> Result<std::path::PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    if !bytes.starts_with(b"MZ") || bytes.len() < 128 {
        return Err("signed Windows installer is not a PE executable".into());
    }
    // create_dir is exclusive: never reuse a PID-named directory or overwrite
    // an existing file. The current per-user temp root supplies its ACL.
    let directory = std::env::temp_dir()
        .join(format!("akula-signed-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).map_err(|e| format!("cannot stage Akula update: {e}"))?;
    let installer = directory.join("Akula-setup.exe");
    let stage_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true).create_new(true).open(&installer)
            .map_err(|e| format!("cannot create fresh Akula installer: {e}"))?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);

        // Verify the exact on-disk bytes again. The signed in-memory payload
        // alone is insufficient proof of what will be handed to ShellExecuteW.
        let mut file = std::fs::File::open(&installer).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() != bytes.len() as u64 {
            return Err("staged Akula NSIS byte count differs from signed candidate".into());
        }
        let mut hash = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            hash.update(&buf[..n]);
        }
        let actual: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if actual != expected_sha256 {
            return Err("staged Akula NSIS hash differs from publisher-signed candidate".into());
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        // We created this unique folder ourselves; never remove an arbitrary
        // preexisting or caller-provided update path.
        let _ = std::fs::remove_dir_all(&directory);
        return Err(error);
    }
    Ok(installer)
}

#[cfg(all(test, target_os = "windows"))]
mod windows_beta_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn staging_requires_pe_and_rehashes_exact_bytes() {
        assert!(stage_signed_windows_setup(b"not an executable", &"0".repeat(64)).is_err());
        let mut bytes = vec![0x42u8; 256];
        bytes[..2].copy_from_slice(b"MZ");
        assert!(stage_signed_windows_setup(&bytes, &"0".repeat(64)).is_err());
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        let path = stage_signed_windows_setup(&bytes, &hash).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

#[cfg(target_os = "macos")]
pub fn apply(payload: &[u8], before_exit: impl FnOnce()) -> Result<(), String> {
    use super::macos_transaction::{AppGate, Recovery};

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let text = exe.to_string_lossy();
    let (prefix, _) = text
        .split_once(".app/Contents/MacOS/")
        .ok_or("app updater requires a native .app bundle")?;
    let bundle = std::path::PathBuf::from(format!("{prefix}.app"));
    let home = std::env::var_os("HOME").ok_or("macOS HOME unavailable")?;
    // The gate uses the same persistent BSD flock as both shell scripts.
    // No fallback to a PID lock or a second authority for /Applications.
    let gate = AppGate::acquire(&bundle, std::path::Path::new(&home))?;
    match gate.recover()? {
        Recovery::Clean => {}
        Recovery::PreviousRestored => {
            return Err("previous app restored after interrupted update; retry only after review".into());
        }
    }

    // No PID-derived overwrite or recursive removal of a previous run's
    // stage. A fresh random name is atomically created under the held lock.
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let stage = gate.new_stage(&nonce)?;
    let decoded = flate2::read::GzDecoder::new(payload);
    if let Err(error) = super::tar::unpack(decoded, &stage) {
        if let Err(cleanup) = gate.discard_unpublished(&stage) {
            return Err(format!("{error}; stage preserved for review: {cleanup}"));
        }
        return Err(error);
    }

    // Record a durable, non-overwriting generation fingerprint BEFORE
    // old->backup or fresh->destination. No publication without this proof.
    if let Err(error) = super::startup_ack::record_candidate(&stage) {
        if let Err(cleanup) = gate.discard_unpublished(&stage) {
            return Err(format!("{error}; failed candidate retained for review: {cleanup}"));
        }
        return Err(error);
    }

    // A failure during application shutdown leaves the previous app live and
    // the stage for review; no new bundle is silently accepted.
    before_exit();
    gate.promote(&stage)?;

    // The program may still have mapped pages from its old bundle while it
    // runs; macOS must accept a new app launch before we exit this process.
    let dispatch = std::process::Command::new("open")
        .arg("-n")
        .arg(&bundle)
        .status();
    match dispatch {
        Ok(status) if status.success() => {
            // "open" success does not prove the new binary is healthy.
            // Keep the original backup until a separately tested startup
            // acknowledgement is implemented. Further installs fail closed.
            gate.preserve_until_startup_ack(&stage)?;
            std::process::exit(0);
        }
        other => {
            let reason = match other {
                Ok(status) => format!("macOS app relaunch rejected: {status}"),
                Err(e) => format!("cannot dispatch macOS app relaunch: {e}"),
            };
            gate.rollback_launch(&stage)
                .map_err(|rollback| format!("{reason}; rollback failed: {rollback}"))?;
            Err(format!("{reason}; old app restored, failed new generation retained for review"))
        }
    }
}

fn appimage() -> Option<std::path::PathBuf> {
    std::env::var_os("APPIMAGE").map(std::path::PathBuf::from)
}

pub fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos")) || cfg!(target_os = "linux") && appimage().is_some()
}

#[cfg(target_os = "linux")]
pub fn apply(payload: &[u8], before_exit: impl FnOnce()) -> Result<(), String> {
    // Direct in-place rename destroyed the previous AppImage before the new
    // generation could prove a healthy startup. All Linux update entrypoints
    // now share the native transaction module's explicit fail-closed gate.
    // Even a mistaken future channel_ready() enablement cannot re-enable
    // the old overwrite. The crash-tested state machine is compiled into
    // the app, but production signing evidence + first-render ACK are HOLD.
    super::linux_transaction::guarded_runtime_apply(payload, before_exit)
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn apply(_payload: &[u8], _before_exit: impl FnOnce()) -> Result<(), String> {
    Err("платформа не поддерживается".into())
}
