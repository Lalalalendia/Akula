//! B1b4b1 isolated UID0 witness process; IPC exposes READ HEAD only.
//! NO signing, install, shell, path, package/owner approval or grant RPC.
//! No production binaries enable this feature. Only disposable systemd CI.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("B1B4B1_REFUSED: Linux only");
    std::process::exit(2);
}
#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("B1B4B1_REFUSED: {error}");
        std::process::exit(2);
    }
}
#[cfg(target_os = "linux")]
mod linux {
    use aquilum_core::server_installer_ipc_lab::verified_peer_uid;
    use aquilum_core::server_installer_os_witness_lab::OsWitnessLab;
    use aquilum_core::server_witness_lab::WitnessCheckpoint;
    use rustix::fs::{open, openat, Mode, OFlags};
    use std::fs::{self, File, Metadata, Permissions};
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::time::Duration;

    const ROOT: &str = "/run/akula-b1b4b1-lab";
    const TRUST: &str = "/run/akula-b1b4b1-lab/trust";
    const JOURNAL: &str = "/run/akula-b1b4b1-lab/journal";
    const SOCKET: &str = "/run/akula-b1b4b1-lab/witness.sock";
    const REQUEST: &[u8] = b"AKULA-B1B4B1-READ-HEAD-V1\0";
    const REPLY: &[u8] = b"AKULA-B1B4B1-HEAD-V1\0";
    const REFUSED: &[u8] = &[0u8];
    const MAX_PEERS: usize = 12;
    unsafe extern "C" { fn umask(mask: u32) -> u32; }

    fn unchanged(before: &Metadata, after: &Metadata) -> bool {
        before.dev() == after.dev() && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    fn peer_uid() -> Result<u32, String> {
        let dir = File::from(open(
            ROOT, OFlags::RDONLY | OFlags::DIRECTORY |
                OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|e| e.to_string())?);
        let meta = dir.metadata().map_err(|e| e.to_string())?;
        if !meta.is_dir() || meta.uid() != 0 || meta.nlink() == 0
            || meta.permissions().mode() & 0o7777 != 0o711
        {
            return Err("root-controlled IPC directory must be 0711".into());
        }
        let mut file = File::from(openat(
            &dir, "allowed-peer-uid",
            OFlags::RDONLY | OFlags::NOFOLLOW |
                OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|e| e.to_string())?);
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.uid() != 0 || before.nlink() != 1
            || before.permissions().mode() & 0o7777 != 0o600
            || before.len() != 4
        {
            return Err("bad root-owned caller UID policy".into());
        }
        let mut bytes = [0u8; 4];
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        if !unchanged(&before, &file.metadata().map_err(|e| e.to_string())?) {
            return Err("caller UID policy changed".into());
        }
        let uid = u32::from_be_bytes(bytes);
        if uid == 0 || uid == u32::MAX {
            return Err("no nonroot client UID provisioned".into());
        }
        Ok(uid)
    }
    fn checkpoint_bytes(checkpoint: WitnessCheckpoint) -> Vec<u8> {
        let floor = checkpoint.accepted_floor;
        let mut out = Vec::with_capacity(REPLY.len() + 132);
        out.extend_from_slice(REPLY);
        out.extend_from_slice(&checkpoint.revision.to_be_bytes());
        out.extend_from_slice(&checkpoint.record_tip);
        out.extend_from_slice(&floor.generation.to_be_bytes());
        out.extend_from_slice(&floor.chain_tip);
        out.extend_from_slice(&floor.floor.sequence.to_be_bytes());
        for v in [
            floor.floor.version.major, floor.floor.version.minor,
            floor.floor.version.patch,
        ] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&floor.floor.signed_manifest_fingerprint);
        debug_assert_eq!(out.len(), REPLY.len() + 132);
        out
    }
    fn read_only(
        stream: &mut UnixStream,
        actor: &OsWitnessLab,
        allowed_uid: u32,
    ) -> Result<(), String> {
        let observed = verified_peer_uid(stream).map_err(|e| format!("{e:?}"))?;
        if observed != allowed_uid {
            return Err("kernel peer UID not authorized".into());
        }
        stream.set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let mut request = [0u8; REQUEST.len()];
        stream.read_exact(&mut request).map_err(|e| e.to_string())?;
        let mut extra = [0u8; 1];
        if stream.read(&mut extra).map_err(|e| e.to_string())? != 0
            || request.as_slice() != REQUEST
        {
            return Err("unsupported or pipelined witness operation".into());
        }
        let checkpoint = actor.current_checkpoint()
            .map_err(|e| format!("unsafe witness journal: {e:?}"))?;
        stream.write_all(&checkpoint_bytes(checkpoint))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn run() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err("separate witness process requires actual UID 0".into());
        }
        let allowed = peer_uid()?;
        let actor = OsWitnessLab::open(Path::new(TRUST), Path::new(JOURNAL))
            .map_err(|e| format!("OS-owned witness refused: {e:?}"))?;
        match fs::symlink_metadata(SOCKET) {
            Ok(_) => return Err("witness socket already exists".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
        // SAFETY: a single-threaded disposable laboratory sets its mask
        // only around the fixed UnixListener bind.
        let old = unsafe { umask(0o177) };
        let listener = UnixListener::bind(SOCKET);
        unsafe { umask(old) };
        let listener = listener.map_err(|e| e.to_string())?;
        fs::set_permissions(SOCKET, Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        println!("B1B4B1_REAL_ROOT_WITNESS_BOUND");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        // Bounded service, no background daemon, no mutable request type.
        for _ in 0..MAX_PEERS {
            let (mut peer, _) = listener.accept().map_err(|e| e.to_string())?;
            let ok = read_only(&mut peer, &actor, allowed).is_ok();
            if !ok {
                let _ = peer.write_all(REFUSED);
            }
            println!("B1B4B1_{}", if ok { "READ_ONLY" } else { "REFUSED" });
            std::io::stdout().flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
