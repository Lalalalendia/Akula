//! B0b Linux OpenSSH preflight, developer lab only. No installer or writer.
//! The trusted ed25519 host key is supplied OUTSIDE the SSH connection.
//! OpenSSH verifies the actual signed KEX host key before public-key auth.
//! No ssh-keyscan TOFU, passwords, agent, proxy, agent forwarding or shell input.

use crate::server_bootstrap_lab::{
    B0Error, B0Input, PreviewTransport, ReadOnlyHostFacts, ServerTarget,
};
use ring::{constant_time, digest};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[path = "server_bootstrap_ssh_common.rs"]
mod common;
use common::{decode_ed25519_wire, parse_facts, FACTS_COMMAND, MAX_OUTPUT};

const SSH_PATH: &str = "/usr/bin/ssh";
const DEADLINE: Duration = Duration::from_secs(12);

struct PinnedFile {
    dir: PathBuf,
    file: PathBuf,
}
impl PinnedFile {
    fn create(install_id: Uuid, key: &str) -> Result<Self, B0Error> {
        // Fixed absolute pathname avoids OpenSSH percent-expansion and
        // untrusted TMPDIR environment options. 0700 directory, 0600 file.
        let dir = PathBuf::from("/tmp").join(format!(
            "akula-b0-{}-{}", install_id.simple(), Uuid::new_v4().simple()
        ));
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        builder.create(&dir).map_err(|_| B0Error::KeyUnavailable)?;
        let file = dir.join("known_hosts");
        let result = (|| {
            let mut out = OpenOptions::new().create_new(true).write(true).mode(0o600)
                .open(&file).map_err(|_| B0Error::KeyUnavailable)?;
            writeln!(out, "akula-b0-{} ssh-ed25519 {}", install_id.simple(), key)
                .map_err(|_| B0Error::KeyUnavailable)?;
            out.sync_all().map_err(|_| B0Error::KeyUnavailable)
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&dir);
            return Err(error);
        }
        Ok(Self { dir, file })
    }
}
impl Drop for PinnedFile {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.dir); }
}

/// Only used explicitly by developer tests, never by the installed GUI.
pub struct OpenSshPinnedProbe {
    target: ServerTarget,
    id: Uuid,
    key_wire: Vec<u8>,
    known: PinnedFile,
    private_key: PathBuf,
}

impl OpenSshPinnedProbe {
    /// Trusted host key is a two-field OpenSSH ed25519 PUBLIC key, acquired
    /// from an independent provider console, not the current SSH peer.
    pub fn new(input: &B0Input, trusted_public: &str, private_key: &Path)
        -> Result<Self, B0Error>
    {
        let mut parts = trusted_public.split_ascii_whitespace();
        if parts.next() != Some("ssh-ed25519") { return Err(B0Error::KeyUnavailable); }
        let encoded = parts.next().ok_or(B0Error::KeyUnavailable)?;
        if parts.next().is_some() { return Err(B0Error::KeyUnavailable); }
        let key_wire = decode_ed25519_wire(encoded)?;
        let digest = digest::digest(&digest::SHA256, &key_wire);
        if constant_time::verify_slices_are_equal(
            digest.as_ref(), &input.target.trusted_key_sha256
        ).is_err() { return Err(B0Error::HostKeyMismatch); }

        // No direct method on this transport may bypass B0a input checks.
        input.target.validate()?;
        input.package.validate()?;
        if input.install_id.is_nil() { return Err(B0Error::InvalidTarget); }
        let md = fs::symlink_metadata(private_key).map_err(|_| B0Error::InvalidTarget)?;
        if !private_key.is_absolute() || !md.is_file() || md.file_type().is_symlink()
            || md.len() == 0 || md.len() > 64 * 1024
            || md.permissions().mode() & 0o077 != 0
            || private_key.to_string_lossy().contains('%')
        { return Err(B0Error::InvalidTarget); }
        if !Path::new(SSH_PATH).is_file() { return Err(B0Error::InspectionUnavailable); }

        let known = PinnedFile::create(input.install_id, encoded)?;
        Ok(Self {
            target: input.target.clone(), id: input.install_id,
            key_wire, known, private_key: private_key.to_owned(),
        })
    }

    fn fixed_read(&self, fixed_command: &'static str) -> Result<String, B0Error> {
        let known = self.known.file.to_str().ok_or(B0Error::InvalidTarget)?;
        let identity = self.private_key.to_str().ok_or(B0Error::InvalidTarget)?;
        let mut cmd = Command::new(SSH_PATH);
        cmd.env("LC_ALL", "C")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            // System/user config and every SSH side channel are disabled.
            .args(["-F", "/dev/null", "-T", "-o", "BatchMode=yes"])
            .args(["-o", "StrictHostKeyChecking=yes"])
            .args(["-o", "HostKeyAlgorithms=ssh-ed25519"])
            .args(["-o", "GlobalKnownHostsFile=/dev/null"])
            .args(["-o", "KnownHostsCommand=none"])
            .args(["-o", "UpdateHostKeys=no"])
            .args(["-o", "VerifyHostKeyDNS=no"])
            .args(["-o", "CheckHostIP=no"])
            .args(["-o", "ProxyCommand=none"])
            .args(["-o", "ProxyJump=none"])
            .args(["-o", "CanonicalizeHostname=no"])
            .args(["-o", "GSSAPIAuthentication=no"])
            .args(["-o", "KbdInteractiveAuthentication=no"])
            .args(["-o", "PasswordAuthentication=no"])
            .args(["-o", "PubkeyAuthentication=yes"])
            .args(["-o", "PreferredAuthentications=publickey"])
            .args(["-o", "IdentityAgent=none"])
            .args(["-o", "IdentitiesOnly=yes"])
            .args(["-o", "CertificateFile=none"])
            .args(["-o", "ControlMaster=no"])
            .args(["-o", "ClearAllForwardings=yes"])
            .args(["-o", "PermitLocalCommand=no"])
            .args(["-o", "RequestTTY=no"])
            .args(["-o", "ConnectTimeout=5"])
            .args(["-o", "ConnectionAttempts=1"])
            .args(["-o", "NumberOfPasswordPrompts=0"])
            .arg("-o").arg(format!("UserKnownHostsFile={known}"))
            .arg("-o").arg(format!("HostKeyAlias=akula-b0-{}", self.id.simple()))
            .arg("-o").arg(format!("IdentityFile={identity}"))
            .arg("-p").arg(self.target.ssh_port.to_string())
            .arg("-l").arg(&self.target.ssh_user)
            .arg("--").arg(&self.target.host)
            .arg(fixed_command);

        let mut child = cmd.spawn().map_err(|_| B0Error::InspectionUnavailable)?;
        let stdout = child.stdout.take().ok_or(B0Error::InspectionUnavailable)?;
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes);
            bytes
        });
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(value) = child.try_wait().map_err(|_| B0Error::InspectionUnavailable)? {
                break value;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(B0Error::InspectionUnavailable);
            }
            thread::sleep(Duration::from_millis(15));
        };
        let out = reader.join().map_err(|_| B0Error::InspectionUnavailable)?;
        if !status.success() || out.len() as u64 > MAX_OUTPUT {
            // Do not propagate potentially attacker-controlled SSH stderr.
            return Err(B0Error::InspectionUnavailable);
        }
        String::from_utf8(out).map_err(|_| B0Error::InspectionUnavailable)
    }
}

impl PreviewTransport for OpenSshPinnedProbe {
    fn handshake_public_key(&mut self, target: &ServerTarget)
        -> Result<Vec<u8>, B0Error>
    {
        if target != &self.target { return Err(B0Error::InvalidTarget); }
        // The actual SSH signed KEX is verified by strict known_hosts.
        // Return only the already pinned reference bytes after success.
        self.fixed_read("true")?;
        Ok(self.key_wire.clone())
    }
    fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
        parse_facts(&self.fixed_read(FACTS_COMMAND)?)
    }
}
