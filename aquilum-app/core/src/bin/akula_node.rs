//! Headless Node V1 local process. No public HTTP/QUIC listener, no shell,
//! no arbitrary path requests, and no root privileges. This executable may
//! become a release artifact only after the integrated install/data E2E gate.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_NODE_V1_HOLD: Linux only");
    std::process::exit(78);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("AKULA_NODE_V1_HOLD: {}", linux::error_label(&error));
        std::process::exit(78);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use aquilum_core::node_v1::{NodeHost, NodeIdentity, NodeV1Error};
    use std::env;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    const STATE: &str = "/var/lib/akula-node";
    const RUNTIME: &str = "/run/akula-node";
    const SOCKET: &str = "health.sock";
    const REQUEST_TAG: u8 = 1;

    pub(super) fn error_label(error: &NodeV1Error) -> &'static str {
        match error {
            NodeV1Error::Hold(reason) => reason,
            NodeV1Error::Io(_) => "Node OS preflight failed",
            NodeV1Error::Sqlite(_) => "Node ledger preflight failed",
        }
    }
    fn fail(reason: &'static str) -> NodeV1Error { NodeV1Error::Hold(reason) }

    fn hex(bytes: &[u8]) -> String {
        let mut result = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(&mut result, "{byte:02x}");
        }
        result
    }

    /// The root installer uses a transient, non-root invocation to learn the
    /// public Node key before enrolling a root-owned owner-policy.json.
    fn print_identity() -> Result<(), NodeV1Error> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Err(fail("Node identity must be created by a non-root service"));
        }
        let identity = NodeIdentity::open_or_create(Path::new(STATE))?;
        println!("AKULA_NODE_V1_PUBLIC {}", hex(&identity.public_key()));
        Ok(())
    }

    fn owned_private_dir(path: &Path) -> Result<(), NodeV1Error> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_dir()
            || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.permissions().mode() & 0o7777 != 0o700
        {
            return Err(fail("unsafe Node RuntimeDirectory"));
        }
        Ok(())
    }

    fn prepare_socket() -> Result<(UnixListener, PathBuf), NodeV1Error> {
        let parent = Path::new(RUNTIME);
        owned_private_dir(parent)?;
        let path = parent.join(SOCKET);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !meta.file_type().is_socket()
                    || meta.uid() != rustix::process::geteuid().as_raw()
                {
                    return Err(fail("untrusted existing local health socket"));
                }
                if UnixStream::connect(&path).is_ok() {
                    return Err(fail("Node health socket already has a listener"));
                }
                fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok((listener, path))
    }

    fn respond(mut client: UnixStream, node: &NodeHost) -> Result<(), NodeV1Error> {
        client.set_read_timeout(Some(Duration::from_secs(2)))?;
        client.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut input = [0u8; 33];
        if client.read_exact(&mut input).is_err()
            || input[0] != REQUEST_TAG
            || input[1..].iter().all(|byte| *byte == 0)
        {
            client.write_all(b"AKULA_NODE_V1_HOLD\n")?;
            return Ok(());
        }
        let mut challenge = [0u8; 32];
        challenge.copy_from_slice(&input[1..]);
        match node.sign_local_health(challenge) {
            Ok(signature) => {
                // The caller must independently pin the public key and use
                // a fresh nonce. LOCAL only, never a claim of data-plane READY.
                let line = format!(
                    "AKULA_NODE_V1_LOCAL_ONLY {} {}\n",
                    hex(&node.public_key()), hex(&signature),
                );
                client.write_all(line.as_bytes())?;
            }
            Err(_) => client.write_all(b"AKULA_NODE_V1_HOLD\n")?,
        }
        Ok(())
    }

    fn serve() -> Result<(), NodeV1Error> {
        let node = NodeHost::open_provisioned()?;
        let (listener, path) = prepare_socket()?;
        // Local health intentionally does not become WAN health or a remote
        // grant. systemd decides process lifecycle; no auto-installer exists.
        for client in listener.incoming() {
            let client = client?;
            // A bad or slow local caller is isolated to this bounded read.
            let _ = respond(client, &node);
        }
        let _ = fs::remove_file(path);
        Ok(())
    }

    pub(super) fn run() -> Result<(), NodeV1Error> {
        let mut args = env::args().skip(1);
        let mode = args.next().ok_or(fail("Node invocation requires fixed mode"))?;
        if args.next().is_some() {
            return Err(fail("Node refuses extra command-line arguments"));
        }
        match mode.as_str() {
            "--public-identity" => print_identity(),
            "--serve-local-health" => serve(),
            _ => Err(fail("Node refuses unknown operation")),
        }
    }
}
