// RELEASE-APPIMAGE-REAL-E2E-01: CLI harness for disposable CI trials only.
// Uses the exact native Linux transaction module, without linking the updater.
// NEVER deploy this helper into Akula or use it on an installed user path.
// The production updater remains HOLD without its own signed trust root.

#![cfg(target_os = "linux")]

#[allow(dead_code)]
#[path = "../aquilum-app/native/src/updater/linux_transaction.rs"]
mod native_transaction;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use native_transaction::Gate;

const NONCE: &str = "a87c6520d3e9416fa5b32897c6d0e124";

fn hold(msg: &str) -> String {
    format!("AKULA_REAL_APPIMAGE_E2E_HOLD: {msg}")
}

fn check_ci_root(active: &Path) -> Result<PathBuf, String> {
    if env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || env::var("AKULA_DISPOSABLE_UPDATE_LAB").as_deref() != Ok("1")
    {
        return Err(hold("real installed updater operations are forbidden"));
    }
    if !active.is_absolute()
        || active.file_name().and_then(|n| n.to_str())
            != Some("akula-app_0.3.13_x86_64.AppImage")
    {
        return Err(hold("only explicit disposable v1 AppImage path is accepted"));
    }
    let parent = active.parent().ok_or_else(|| hold("no parent"))?;
    let canonical = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if canonical.parent() != Some(Path::new("/tmp"))
        || !canonical.file_name().and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("akula-release-e2e-"))
        || parent != canonical.as_path()
    {
        return Err(hold("only immediate /tmp disposable test directory allowed"));
    }
    Ok(canonical)
}

fn run() -> Result<(), String> {
    let argv: Vec<_> = env::args_os().collect();
    if argv.len() < 3 || argv.len() > 4 {
        return Err(hold("usage: e2e-helper publish|rollback ABSOLUTE_DISPOSABLE_V1 [V2]"));
    }
    let command = argv[1].to_str().ok_or_else(|| hold("bad mode"))?;
    let installed = PathBuf::from(argv[2].clone());
    let root = check_ci_root(&installed)?;
    let gate = Gate::acquire(&installed)?;
    let stage = root.join(format!(".akula-appimage-update.{NONCE}"));
    match command {
        "publish" if argv.len() == 4 => {
            let candidate = PathBuf::from(argv[3].clone());
            if !candidate.is_absolute() || candidate.file_name()
                .and_then(|n| n.to_str()) != Some("akula-app_0.3.14_x86_64.AppImage")
            {
                return Err(hold("wrong candidate filename/version"));
            }
            let meta = fs::symlink_metadata(&candidate).map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() == 0 || meta.len() > 512 * 1024 * 1024 {
                return Err(hold("candidate not a bounded file"));
            }
            // CLI shell first authenticates this exact file using an ephemeral
            // Minisign-signed manifest. Its SHA256 is checked again after disk
            // staging by the companion Python witness.
            let bytes = fs::read(&candidate).map_err(|e| e.to_string())?;
            let generated = gate.prepare(NONCE, &bytes)?;
            if generated != stage {
                return Err(hold("unexpected stage generation"));
            }
            gate.backup_old(&stage)?;
            gate.publish_new(&stage)?;
            println!("AKULA_REAL_APPIMAGE_DISK_PUBLISH_PASS same_fs=1 previous_retained=1");
        }
        "rollback" if argv.len() == 3 => {
            gate.rollback_unacknowledged(&stage)?;
            println!("AKULA_REAL_APPIMAGE_DISK_ROLLBACK_PASS previous_restored=1 failed_candidate_retained=1");
        }
        _ => return Err(hold("wrong command arguments")),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
