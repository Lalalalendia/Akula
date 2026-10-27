//! Experimental read-only post-reboot root-stage reinspection over the same
//! independently pinned Windows OpenSSH host identity. Never executes a
//! helper, commits Node, changes systemd, or grants install authority.
use super::*;

const INSPECT_WAIT: Duration = Duration::from_secs(150);
const INSPECT_REPLY_PREFIX: &[u8] = b"AKULA_ROOT_STAGE_REINSPECTED_HOLD:";

fn fixed_inspect_command(source: &str) -> Result<String, RootStageTransportError> {
    let mut command = fixed_root_command(source)?;
    command.push_str(" --inspect-hold");
    Ok(command)
}

fn matching_readback(stdout: &[u8], expected_sha: [u8; 32]) -> bool {
    let mut expected = Vec::with_capacity(INSPECT_REPLY_PREFIX.len() + 65);
    expected.extend_from_slice(INSPECT_REPLY_PREFIX);
    for byte in expected_sha {
        expected.extend_from_slice(format!("{byte:02x}").as_bytes());
    }
    expected.push(b'\n');
    stdout == expected
}

impl WindowsOpenSshPinnedProbe {
    /// Only obtains an UNSIGNED report over pinned SSH from a fixed,
    /// OS-elevated read-only root-stage verifier. A valid report is not an
    /// independent root attestation or permission to activate Node.
    pub fn reinspect_reported_root_stage(
        &self,
        bundle: &VerifiedRootStageBundle,
    ) -> Result<RootStageRemoteReport, RootStageTransportError> {
        let command = fixed_inspect_command(bundle.fixed_root_source())?;
        let mut ssh = self
            .pinned_ssh_command(false)
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        ssh.stdin(Stdio::null()).arg(command);
        let mut child = ssh
            .spawn()
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(RootStageTransportError::SshUnavailable)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(RootStageTransportError::SshUnavailable)?;
        let out = read_bounded(stdout);
        let err = read_bounded(stderr);
        let deadline = Instant::now() + INSPECT_WAIT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out.join();
                    let _ = err.join();
                    return Err(RootStageTransportError::TimedOut);
                }
            }
        };
        let stdout = out
            .join()
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        let stderr = err
            .join()
            .map_err(|_| RootStageTransportError::SshUnavailable)?;
        if !status.success() {
            return Err(RootStageTransportError::RemoteRefused);
        }
        if !stderr.is_empty() || !matching_readback(&stdout, bundle.package_sha256()) {
            return Err(RootStageTransportError::UnexpectedRemoteReply);
        }
        Ok(RootStageRemoteReport::RemoteReportedReinspectedHold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_readonly_command_is_authorized() {
        let source = format!(
            "PINNED_BOOTSTRAP_ED25519_HEX = \"{}\"\n# {}",
            "72".repeat(32),
            "reviewed source only".repeat(20),
        );
        let command = fixed_inspect_command(&source).unwrap();
        assert!(command.starts_with("exec /usr/bin/sudo -n -- /usr/bin/python3 -I -c '"));
        assert!(command.ends_with("' --inspect-hold"));
        assert!(!command.contains("--install-node"));
        assert!(fixed_inspect_command("").is_err());
    }

    #[test]
    fn readback_is_digest_bound_and_not_a_root_receipt() {
        let mut output = INSPECT_REPLY_PREFIX.to_vec();
        output.extend_from_slice("aa".repeat(32).as_bytes());
        output.push(b'\n');
        assert!(matching_readback(&output, [0xaa; 32]));
        assert!(!matching_readback(&output, [0xbb; 32]));
        assert!(!matching_readback(&output[..output.len() - 1], [0xaa; 32]));
        output.push(b'!');
        assert!(!matching_readback(&output, [0xaa; 32]));
    }
}
