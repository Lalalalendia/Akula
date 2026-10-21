#[cfg(target_os = "windows")]
pub fn apply(_payload: &[u8], _before_exit: impl FnOnce()) -> Result<(), String> {
    // Windows must not launch a silently downloaded installer from a PID
    // temp path. Even an accidentally enabled release channel cannot skip
    // publisher-signed admission, OS-locked staging, a previous-generation
    // backup, first-GUI ACK and tested rollback. The current sealed
    // SignedCandidate verification layer is intentionally inert.
    Err("HOLD: Windows Akula updater needs signed transactional install and crash-tested rollback".into())
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
