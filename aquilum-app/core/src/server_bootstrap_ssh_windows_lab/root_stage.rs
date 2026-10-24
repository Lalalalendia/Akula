//! Windows signed-KEX transport for B1 root *data staging only*.
//!
//! This is an opt-in NONSHIPPING capability. A one-shot reviewed local
//! consent is consumed before any privileged SSH subprocess may start.
//! The remote /usr/bin/python3 -I -c source comes exclusively from the
//! verified, compiled Core bundle; the remote user cannot supply a path.
//!
//! Root privilege is NOT made safe merely by restricting a sudoers entry to
//! python -c: that entry would grant arbitrary root code execution. This
//! transport requires independently existing administrative sudo authority.
//! The returned remote text is NOT a root attestation, B1d0 receipt, or
//! permission to run the staged ELF or enable the native Install button.
use super::*;
use crate::server_first_root_stage::VerifiedRootStageBundle;
use crate::server_release_lab::review::InertUserConsent;
use std::io::Write;
use std::process::ExitStatus;

const MAX_SOURCE: usize = 32 * 1024;
const MAX_REPLY: u64 = 128;
const ROOT_STAGE_WAIT: Duration = Duration::from_secs(16 * 60);
const REMOTE_HOLD: &[u8] = b"AKULA_ROOT_STAGE_VERIFIED_HOLD\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootStageTransportError {
    ReviewMismatch,
    NoTrustedFixedSource,
    SshUnavailable,
    RemoteRefused,
    UnexpectedRemoteReply,
    TimedOut,
}

/// The remote process *reports* success, but this is not a signed attestation:
/// independent UID0/root-owned witness + B1d0 receipt checks are still needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootStageRemoteReport {
    RemoteReportedSignedDataHold,
}

fn shell_single_quote(source: &str) -> String {
    let mut result = String::with_capacity(source.len() + 32);
    result.push('\'');
    for ch in source.chars() {
        if ch == '\'' {
            result.push_str("'\"'\"'");
        } else {
            result.push(ch);
        }
    }
    result.push('\'');
    result
}

fn fixed_root_command(source: &str) -> Result<String, RootStageTransportError> {
    // The public signing key is substituted only after a B1a verification
    // in a compile-time source. Never accept source taken from SSH stdout,
    // user settings, the package contents, or a relative path.
    if source.len() > MAX_SOURCE
        || source.len() < 256
        || source.contains('\0')
        || !source.contains("PINNED_BOOTSTRAP_ED25519_HEX = \"")
        || source.contains("PINNED_BOOTSTRAP_ED25519_HEX = \"\"")
    {
        return Err(RootStageTransportError::NoTrustedFixedSource);
    }
    Ok(format!(
        "exec sudo -n -- /usr/bin/python3 -I -c {}",
        shell_single_quote(source),
    ))
}

fn read_bounded<R: Read + Send + 'static>(reader: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.take(MAX_REPLY + 1).read_to_end(&mut bytes);
        bytes
    })
}

fn exited(status: ExitStatus, stdout: &[u8], stderr: &[u8], sent: bool)
    -> Result<RootStageRemoteReport, RootStageTransportError>
{
    if !status.success() || !sent {
        return Err(RootStageTransportError::RemoteRefused);
    }
    if stdout != REMOTE_HOLD || !stderr.is_empty() {
        return Err(RootStageTransportError::UnexpectedRemoteReply);
    }
    Ok(RootStageRemoteReport::RemoteReportedSignedDataHold)
}

impl WindowsOpenSshPinnedProbe {
    /// Experimental one-shot transport. The consent has no public constructor,
    /// and is consumed even when SSH fails. The native Windows installed
    /// application has no call path, no installer authorization or sudo grant.
    ///
    /// The signed helper differs from the later Node executable; a future UI
    /// MUST separately disclose the helper SHA and use owner-signed root CAS
    /// before this can be enabled as a production installation operation.
    pub fn stage_verified_root_helper(
        &self,
        bundle: &VerifiedRootStageBundle,
        consent: InertUserConsent,
    ) -> Result<RootStageRemoteReport, RootStageTransportError> {
        let (install_id, plan_hash, host_key, package_hash, manifest) = consent.scope();
        if install_id != *self.id.as_bytes()
            || host_key != self.target.trusted_key_sha256
            || plan_hash == [0; 32]
            || package_hash == [0; 32]
            || manifest == [0; 32]
            || consent.nonce() == [0; 16]
        {
            return Err(RootStageTransportError::ReviewMismatch);
        }
        let source = bundle.fixed_root_source();
        let remote = fixed_root_command(source)?;
        let mut ssh = self
            .pinned_ssh_command(true)
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        ssh.stdin(Stdio::piped()).arg(remote);

        let mut child = ssh
            .spawn()
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(RootStageTransportError::SshUnavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(RootStageTransportError::SshUnavailable)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(RootStageTransportError::SshUnavailable)?;
        let frame = bundle.shared_frame();
        // A writer thread prevents a hostile remote server from holding the
        // UI's synchronously waiting thread in a blocked pipe write.
        let writer = thread::spawn(move || {
            let mut handle = stdin;
            handle.write_all(&frame).is_ok()
        });
        let out = read_bounded(stdout);
        let err = read_bounded(stderr);
        let deadline = Instant::now() + ROOT_STAGE_WAIT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = writer.join();
                    let _ = out.join();
                    let _ = err.join();
                    return Err(RootStageTransportError::TimedOut);
                }
            }
        };
        let sent = writer.join().unwrap_or(false);
        let stdout = out
            .join()
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        let stderr = err
            .join()
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        exited(status, &stdout, &stderr, sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_source_is_one_remote_shell_argument_not_a_user_path() {
        let source = "PINNED_BOOTSTRAP_ED25519_HEX = \"123\"\n# ' ; $(touch /tmp/pwn) \n"
            .repeat(5);
        let command = fixed_root_command(&source).unwrap();
        assert!(command.starts_with("exec sudo -n -- /usr/bin/python3 -I -c '"));
        assert!(command.contains("'\"'\"'"));
        assert!(!command.contains("/tmp/akula.sh"));
    }

    #[test]
    fn no_public_key_or_oversized_script_never_qualifies_for_sudo() {
        assert_eq!(
            fixed_root_command(""),
            Err(RootStageTransportError::NoTrustedFixedSource),
        );
        assert_eq!(
            fixed_root_command(&"X".repeat(MAX_SOURCE + 1)),
            Err(RootStageTransportError::NoTrustedFixedSource),
        );
        assert_eq!(
            fixed_root_command(&format!(
                "PINNED_BOOTSTRAP_ED25519_HEX = \"\"\n{}",
                "#".repeat(400),
            )),
            Err(RootStageTransportError::NoTrustedFixedSource),
        );
    }
}
