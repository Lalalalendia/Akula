//! Pinned Windows OpenSSH invocation, shared by read-only and root-staging probes.
use super::*;
impl WindowsOpenSshPinnedProbe {
    /// Share the exact independent host-key pin with the opt-in root stage.
    /// No password, TOFU, ProxyJump, user-supplied shell or key file.
    pub(super) fn pinned_ssh_command(&self, use_stdin: bool) -> Result<Command, B0Error> {
        let known = config_path(&self.known.path)?;
        let mut ssh = Command::new(&self.ssh_exe);
        ssh.stdout(Stdio::piped()).stderr(Stdio::piped())
            .env_remove("SSH_ASKPASS").env_remove("GIT_SSH_COMMAND")
            .env_remove("SSH_AUTH_SOCK")
            .args(["-F", "none", "-T"]);
        if !use_stdin {
            ssh.arg("-n");
        }
        ssh.args(["-o", "BatchMode=yes"])
            .args(["-o", "StrictHostKeyChecking=yes"])
            .args(["-o", "HostKeyAlgorithms=ssh-ed25519"])
            .args(["-o", "GlobalKnownHostsFile=NUL"])
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
            // No user-supplied SSH_AUTH_SOCK: let Windows OpenSSH use its
            // standard local ssh-agent service without reading a key file.
            .args(["-o", "IdentitiesOnly=no"])
            .args(["-o", "CertificateFile=none"])
            .args(["-o", "ControlMaster=no"])
            .args(["-o", "ClearAllForwardings=yes"])
            .args(["-o", "ForwardAgent=no"])
            .args(["-o", "ForwardX11=no"])
            .args(["-o", "PermitLocalCommand=no"])
            .args(["-o", "RequestTTY=no"])
            .args(["-o", "ConnectTimeout=5"])
            .args(["-o", "ConnectionAttempts=1"])
            .args(["-o", "NumberOfPasswordPrompts=0"])
            .args(["-o", "LogLevel=ERROR"])
            .arg("-o").arg(format!("UserKnownHostsFile={known}"))
            .arg("-o").arg(format!("HostKeyAlias=akula-b0c-{}", self.id.simple()))
            .arg("-p").arg(self.target.ssh_port.to_string())
            .arg("-l").arg(&self.target.ssh_user)
            .arg("--").arg(&self.target.host);
        Ok(ssh)
    }

}
