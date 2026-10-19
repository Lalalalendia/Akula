#[cfg(target_os = "windows")]
pub fn apply(payload: &[u8], before_exit: impl FnOnce()) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    use windows::core::{HSTRING, w};
    let dir = std::env::temp_dir().join(format!("akula-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let installer = dir.join("Akula-setup.exe");
    std::fs::write(&installer, payload).map_err(|e| e.to_string())?;
    before_exit();
    let started = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(installer.as_os_str()), &HSTRING::from("/S /R /UPDATE /ARGS"), None, SW_SHOW) };
    if started.0 as isize <= 32 {
        return Err(format!("установщик не запустился, код {}", started.0 as isize));
    }
    std::process::exit(0);
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
    use std::os::unix::fs::PermissionsExt;
    let image = appimage().ok_or("обновление поддерживается только для AppImage")?;
    let fresh = image.with_extension("update");
    std::fs::write(&fresh, payload).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    before_exit();
    std::fs::rename(&fresh, &image).map_err(|e| e.to_string())?;
    let _ = std::process::Command::new(&image).spawn();
    std::process::exit(0);
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn apply(_payload: &[u8], _before_exit: impl FnOnce()) -> Result<(), String> {
    Err("платформа не поддерживается".into())
}
