//! VIDEO-18c-02b-02b-01: ephemeral Minisign-authenticated exact-five
//! *runnable* protected current/ GUI ACK, without any update confirmation.
//! This is a nonshipping, standalone GitHub-hosted Windows PR-only program.
#[cfg(not(windows))]
fn main() {
    eprintln!("VIDEO18C02B02B_HOLD: Windows-only disposable laboratory");
    std::process::exit(2);
}

#[cfg(windows)]
#[path = "../../video18c02b_real_gui_ack.rs"]
mod gui_launcher;

#[cfg(windows)]
mod signed_lab {
    use akula_video18b18_signed_five::admit_hosted_lab_signature;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha256};
    use std::env;
    use std::fs::{self, File};
    use std::io::{Read, ErrorKind};
    use std::path::{Path, PathBuf};

    fn required(name: &str) -> Result<String, String> {
        let v = env::var(name).map_err(|_| format!("missing {name}"))?;
        if v.is_empty() { return Err(format!("empty {name}")); }
        Ok(v)
    }

    fn regular(path: &Path) -> Result<(), String> {
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("not regular/non-reparse test input: {}", path.display()));
        }
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("reparse point in signed lab inputs".into());
        }
        Ok(())
    }

    fn limited(path: &Path, size: u64) -> Result<Vec<u8>, String> {
        regular(path)?;
        let f = File::open(path).map_err(|e| e.to_string())?;
        if f.metadata().map_err(|e| e.to_string())?.len() > size {
            return Err("oversized signed test input".into());
        }
        let mut v = Vec::new();
        f.take(size + 1).read_to_end(&mut v).map_err(|e| match e.kind() {
            ErrorKind::InvalidData => "invalid test bytes".into(),
            _ => e.to_string(),
        })?;
        if v.is_empty() || v.len() as u64 > size {
            return Err("empty or oversized signed test input".into());
        }
        Ok(v)
    }

    fn pinned_under_runner(path: &Path, runner: &Path) -> Result<PathBuf, String> {
        regular(path)?;
        let actual = fs::canonicalize(path).map_err(|e| e.to_string())?;
        if !actual.starts_with(runner) || actual == runner {
            return Err("test signer escaped disposable runner temporary root".into());
        }
        Ok(actual)
    }

    pub fn run() -> Result<(), String> {
        super::gui_launcher::lab::guarded_pr()?;
        let args: Vec<String> = env::args().collect();
        if args.len() != 5 {
            return Err("expected current/Akula, manifest, signature, test pubkey".into());
        }
        let runner = fs::canonicalize(required("RUNNER_TEMP")?).map_err(|e| e.to_string())?;
        let app = super::gui_launcher::lab::validate_app(&args[1])?;
        let manifest = pinned_under_runner(Path::new(&args[2]), &runner)?;
        let signature = pinned_under_runner(Path::new(&args[3]), &runner)?;
        let public_key = pinned_under_runner(Path::new(&args[4]), &runner)?;
        if ![manifest.file_name().and_then(|x| x.to_str()),
            signature.file_name().and_then(|x| x.to_str()),
            public_key.file_name().and_then(|x| x.to_str())]
            .iter().zip(["five-manifest.json", "five-manifest.minisig", "test-publisher.pub"])
            .all(|(actual, expected)| *actual == Some(expected))
        {
            return Err("unexpected Minisign fixture filenames".into());
        }
        let trust = manifest.parent().ok_or("missing signed trust directory")?;
        if signature.parent() != Some(trust) || public_key.parent() != Some(trust) {
            return Err("test manifest/signature/key not in same protected fixture".into());
        }
        let bytes = limited(&manifest, 8192)?;
        let detached = STANDARD.encode(limited(&signature, 4096)?);
        let key = STANDARD.encode(limited(&public_key, 4096)?);
        // Only the feature-restricted ephemeral-key path may create SignedFive;
        // cryptographic authentication happens before parsing version/digests.
        let signed = admit_hosted_lab_signature(&bytes, &detached, &key, "0.3.20")?;
        if signed.version() != "0.3.21" {
            return Err("unexpected lab-only signed candidate version".into());
        }
        if signed.source_commit() != required("EXPECTED_HEAD_SHA")? {
            return Err("manifest was not produced by this exact PR HEAD".into());
        }
        let cargo_lock = Path::new(&required("GITHUB_WORKSPACE")?)
            .join("aquilum-app/Cargo.lock");
        regular(&cargo_lock)?;
        let got_lock = Sha256::digest(fs::read(&cargo_lock).map_err(|e| e.to_string())?)
            .iter().map(|x| format!("{x:02x}")).collect::<String>();
        if signed.cargo_lock_sha256() != got_lock {
            return Err("signed source dependency lock differs from exact PR checkout".into());
        }
        let current = app.parent().ok_or("missing protected current")?;
        signed.verify_stage(current)?;
        let digest = signed.generation_sha256();
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err("unexpected authenticated manifest digest".into());
        }
        let verify = |receipt: &[u8], nonce: &str, os_pid: u32| {
            signed.verify_first_presented_frame_ack(receipt, nonce, os_pid).is_ok()
        };
        // The exact same process performs signed admission, OS-child/window
        // witness, and byte-for-byte SignedFive verification of real GUI ACK.
        let accepted = super::gui_launcher::lab::case(&app, digest, "good", Some(&verify))?;
        if !accepted { return Err("real GUI ACK did not bind signed five-PE generation".into()); }

        let wrong_nonce =
            super::gui_launcher::lab::case(&app, digest, "wrong-nonce", Some(&verify))?;
        if wrong_nonce { return Err("wrong nonce was accepted".into()); }

        // Even a real window plus a validly formatted response for a different
        // manifest must not pass SignedFive's signed-generation comparison.
        let wrong_sha = if digest == "f".repeat(64) { "0".repeat(64) }
            else { "f".repeat(64) };
        let wrong_generation =
            super::gui_launcher::lab::case(&app, &wrong_sha, "good", Some(&verify))?;
        if wrong_generation { return Err("wrong signed manifest digest was accepted".into()); }

        signed.verify_stage(current)?;
        println!(
            "VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS real_current_gui=1 \
             exact_five_pe_sha=1 ephemeral_minisign=1 manifest_digest=1 \
             os_child_handle=1 visible_window=1 nonce_denied=1 \
             wrong_generation_denied=1 old_new_distinct=0 \
             rar2_activation=0 confirm=0 owner_key=0 release=0"
        );
        Ok(())
    }
}

#[cfg(windows)]
fn main() {
    if let Err(e) = signed_lab::run() {
        eprintln!("VIDEO18C02B02B_HOLD: {e}");
        std::process::exit(1);
    }
}
