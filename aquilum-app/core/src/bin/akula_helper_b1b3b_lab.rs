//! B1b3b: real UID 0 OS process serving *only* B1b3a inert Preview over
//! AF_UNIX. Policy is pinned from a fixed root-owned file under /run, never
//! supplied by the connecting client. NO privileged installation, systemd
//! mutation, package execution, arbitrary path or command IPC is available.
//!
//! This is deliberately a DISPOSABLE Ubuntu CI LAB helper, not a shipped
//! package. A real production daemon still needs protected owner-consent
//! issuance, external durable CAS and an inode-sealed verified release.
#[cfg(not(target_os = "linux"))]
fn main() { eprintln!("B1B3B_REFUSED: Linux lab only"); std::process::exit(2); }

#[cfg(target_os = "linux")]
fn main() {
    if let Err(e) = linux::run() {
        eprintln!("B1B3B_REFUSED: {e}");
        std::process::exit(2);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use aquilum_core::server_installer_ipc_lab::{
        admit_preflight, RootTrustedPolicy,
    };
    #[cfg(feature = "server-bootstrap-b1b3c-pinned-fd-lab")]
    use aquilum_core::server_installer_pinned_release_lab::{
        verify_root_staged,RootPinnedReleaseTrust,RootVerifiedFd,
    };
    #[cfg(feature = "server-bootstrap-b1b3c-pinned-fd-lab")]
    use aquilum_core::server_release_lab::{
        ReleaseFloor,TrustedReleaseSigner,
    };
    use rustix::fs::{open, openat, Mode, OFlags};
    use std::fs::{self, File, Permissions};
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    const ROOT: &str = "/run/akula-b1b3b-lab";
    const POLICY: &str = "policy.bin";
    const MARKER: &str = "marker";
    const SOCKET: &str = "/run/akula-b1b3b-lab/helper.sock";
    const DOMAIN: &[u8] = b"AKULA-B1B3B-CI-TRUST-V1\0";
    const POLICY_LEN: usize = DOMAIN.len() + 4 + 32 + 16 + 32 * 4 + 8 + 8;
    const MAX_PEERS: usize = 12;

    unsafe extern "C" { fn umask(mask: u32) -> u32; }

    fn strict_file(file: &File, mode: u32, expected_len: Option<u64>)
        -> Result<(), String>
    {
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.uid() != 0 || m.nlink() != 1
            || m.permissions().mode() & 0o7777 != mode
            || expected_len.is_some_and(|n| m.len() != n)
        {
            return Err("root-owned policy/marker permissions, type or size are unsafe".into());
        }
        Ok(())
    }
    fn policy() -> Result<RootTrustedPolicy, String> {
        let root = File::from(open(
            ROOT, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(|e| format!("root policy directory: {e}"))?);
        let meta = root.metadata().map_err(|e| e.to_string())?;
        if !meta.is_dir() || meta.uid() != 0 || meta.nlink() == 0
            || meta.permissions().mode() & 0o7777 != 0o755
        {
            return Err("policy directory must be owned by root and 0755".into());
        }
        let marker = File::from(openat(
            &root, MARKER, OFlags::RDONLY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(|e| format!("lab marker not provisioned: {e}"))?);
        strict_file(&marker, 0o600, Some(20))?;
        let mut marker_body = [0u8; 20];
        (&marker).read_exact(&mut marker_body).map_err(|e| e.to_string())?;
        if &marker_body != b"AKULA-B1B3B-CI-MARK\n" {
            return Err("not a provisioned disposable CI laboratory".into());
        }
        let mut file = File::from(openat(
            &root, POLICY, OFlags::RDONLY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(|e| format!("root-owned policy missing: {e}"))?);
        strict_file(&file, 0o600, Some(POLICY_LEN as u64))?;
        let mut body = [0u8; POLICY_LEN];
        file.read_exact(&mut body).map_err(|e| e.to_string())?;
        strict_file(&file, 0o600, Some(POLICY_LEN as u64))?;

        if !body.starts_with(DOMAIN) {
            return Err("wrong pinned policy version/domain".into());
        }
        let mut p = DOMAIN.len();
        fn take<const N: usize>(wire: &[u8], pos: &mut usize) -> Result<[u8; N], String> {
            let end = pos.checked_add(N).ok_or("policy offset overflow")?;
            let data = wire.get(*pos..end).ok_or("truncated protected policy")?;
            *pos = end;
            data.try_into().map_err(|_| "policy fixed field size".into())
        }
        let uid = u32::from_be_bytes(take::<4>(&body, &mut p)?);
        let k = take::<32>(&body, &mut p)?;
        let install = take::<16>(&body, &mut p)?;
        let plan = take::<32>(&body, &mut p)?;
        let host = take::<32>(&body, &mut p)?;
        let manifest = take::<32>(&body, &mut p)?;
        let pkg = take::<32>(&body, &mut p)?;
        let seq = u64::from_be_bytes(take::<8>(&body, &mut p)?);
        let revision = u64::from_be_bytes(take::<8>(&body, &mut p)?);
        if p != POLICY_LEN || uid == 0 || k == [0;32] || install == [0;16]
            || plan == [0;32] || host == [0;32] || manifest == [0;32]
            || pkg == [0;32] || seq == 0
        {
            return Err("invalid root-trusted policy values".into());
        }
        Ok(RootTrustedPolicy {
            service_euid: 0,
            allowed_peer_uid: uid,
            pinned_approver_ed25519: k,
            install_id: install,
            approved_plan_digest: plan,
            pinned_ssh_host_sha256: host,
            signed_manifest_fingerprint: manifest,
            staged_package_sha256: pkg,
            expected_release_sequence: seq,
            expected_witness_revision: revision,
        })
    }

    #[cfg(feature = "server-bootstrap-b1b3c-pinned-fd-lab")]
    fn pin_b1a_release(trust:&RootTrustedPolicy)->Result<RootVerifiedFd,String>{
        // The B1a release signing key resides in independently root-owned
        // trust state, not in the release directory or a client payload.
        let root=File::from(open(
            ROOT,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|e|format!("root trust directory: {e}"))?);
        let mut key=File::from(openat(
            &root,"release.pub",
            OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|e|format!("pinned release trust missing: {e}"))?);
        strict_file(&key,0o600,Some(32))?;
        let mut bytes=[0u8;32];
        key.read_exact(&mut bytes).map_err(|e|e.to_string())?;
        let now=std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e|e.to_string())?.as_secs();
        let pinned=verify_root_staged(
            Path::new("/run/akula-b1b3b-lab/release"),
            &RootPinnedReleaseTrust{
                trusted_signer:TrustedReleaseSigner {
                    signer_id:"akula-b1b2b-ephemeral-test".into(),
                    ed25519_public:bytes,
                },
                previous_floor:ReleaseFloor::initial(),
                approved_manifest_fingerprint:trust.signed_manifest_fingerprint,
                approved_package_sha256:trust.staged_package_sha256,
                approved_release_sequence:trust.expected_release_sequence,
            },now,
        ).map_err(|e|format!("B1a root pinned candidate rejected: {e:?}"))?;
        println!("B1B3C_ROOT_B1A_SIGNED_FD_PINNED");
        std::io::stdout().flush().map_err(|e|e.to_string())?;
        Ok(pinned)
    }

    pub fn run() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err("OS-owned helper must be an actual UID 0 process".into());
        }
        let trust = policy()?;
        #[cfg(feature = "server-bootstrap-b1b3c-pinned-fd-lab")]
        let verified_fd=pin_b1a_release(&trust)?;

        if Path::new(SOCKET).exists() || Path::new(SOCKET).is_symlink() {
            return Err("existing socket or symlink; refusing replacement".into());
        }
        // SAFETY: only the current, single-threaded lab process sets its
        // creation mask and restores it immediately after the bind operation.
        let old = unsafe { umask(0o177) };
        let listener = UnixListener::bind(SOCKET);
        unsafe { umask(old) };
        let listener = listener.map_err(|e| format!("fixed AF_UNIX bind: {e}"))?;
        fs::set_permissions(SOCKET, Permissions::from_mode(0o600))
            .map_err(|e| format!("private socket: {e}"))?;
        println!("B1B3B_ROOT_SERVICE_BOUND");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        // A bounded one-shot lab; no background resident system service.
        // Policy does not change in response to any incoming connection.
        for _ in 0..MAX_PEERS {
            let (mut client, _) = listener.accept().map_err(|e| e.to_string())?;
            #[cfg(feature = "server-bootstrap-b1b3c-pinned-fd-lab")]
            let still_pinned=verified_fd.stable().is_ok();
            #[cfg(not(feature = "server-bootstrap-b1b3c-pinned-fd-lab"))]
            let still_pinned=true;
            let accepted=still_pinned && admit_preflight(
                &mut client, &trust,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?.as_secs(),
            ).is_ok();
            let _ = client.write_all(&[if accepted { 1 } else { 0 }]);
            println!("B1B3B_{}", if accepted { "PREVIEW_ONLY" } else { "REFUSED" });
        }
        Ok(())
    }
}
