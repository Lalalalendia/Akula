//! Optional discovery of an SSH *public* key from a target whose independently
//! supplied SHA-256 fingerprint is already pinned. Untrusted ssh-keyscan output
//! is never enrolled: an exact SHA-256 match plus signed ssh.exe KEX is required.
use super::{B0Error, ServerTarget, WindowsOpenSshPinnedProbe};
use ring::{constant_time, digest};
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

impl WindowsOpenSshPinnedProbe {
    pub fn inspect_readonly_fingerprint_target(
        target: &ServerTarget,
    ) -> Result<super::ReadOnlyHostFacts, B0Error> {
        target.validate()?;
        let candidate = discover_matching_ed25519(target)?;
        // The actual ssh.exe connection must authenticate the host's
        // SIGNATURE using the key matching the independent fingerprint.
        Self::inspect_readonly_target(target, &candidate)
    }
}

pub(super) fn discover_matching_ed25519(target: &ServerTarget) -> Result<String, B0Error> {
    let scanner = super::ssh_system_binary()?.with_file_name("ssh-keyscan.exe");
    let md = fs::symlink_metadata(&scanner).map_err(|_| B0Error::KeyUnavailable)?;
    if !md.is_file() || md.file_type().is_symlink() {
        return Err(B0Error::KeyUnavailable);
    }
    let mut child = Command::new(&scanner)
        .args(["-T", "5", "-t", "ed25519", "-p"])
        .arg(target.ssh_port.to_string())
        .arg(&target.host)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| B0Error::InspectionUnavailable)?;
    let stdout = child.stdout.take().ok_or(B0Error::InspectionUnavailable)?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.take(8193).read_to_end(&mut output);
        output
    });
    let deadline = Instant::now() + Duration::from_secs(9);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| B0Error::InspectionUnavailable)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(B0Error::InspectionUnavailable);
        }
        thread::sleep(Duration::from_millis(15));
    };
    let output = reader.join().map_err(|_| B0Error::InspectionUnavailable)?;
    if !status.success() || output.len() > 8192 {
        return Err(B0Error::InspectionUnavailable);
    }
    let output = std::str::from_utf8(&output).map_err(|_| B0Error::KeyUnavailable)?;
    let mut matched: Option<String> = None;
    for line in output.lines().filter(|line| !line.starts_with('#')) {
        let mut words = line.split_ascii_whitespace();
        let (Some(_host), Some("ssh-ed25519"), Some(encoded), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        let Ok(wire) = super::decode_ed25519_wire(encoded) else {
            continue;
        };
        let actual = digest::digest(&digest::SHA256, &wire);
        if constant_time::verify_slices_are_equal(
            actual.as_ref(),
            &target.trusted_key_sha256,
        ).is_ok() {
            let public = format!("ssh-ed25519 {encoded}");
            if matched.as_ref().is_some_and(|value| value != &public) {
                return Err(B0Error::HostKeyMismatch);
            }
            matched = Some(public);
        }
    }
    matched.ok_or(B0Error::HostKeyMismatch)
}
