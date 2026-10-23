//! Explicit non-root, one-time identity enrollment for the signed installer.
//! No root policy, systemd, network listener or service account is created.
use super::identity;
use aquilum_core::node_v1::NodeIdentity;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PublicIdentityPair {
    pub node_public: [u8; 32],
    pub iroh_endpoint: [u8; 32],
}

fn absent(path: &Path) -> Result<(), &'static str> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("enrollment found existing or inaccessible identity"),
    }
}

fn enroll_at(
    state: &Path,
    owner_policy: &Path,
) -> Result<PublicIdentityPair, &'static str> {
    if rustix::process::geteuid().as_raw() == 0 {
        return Err("identity enrollment refuses root");
    }
    // The signed root helper must have already created a private, dedicated
    // service-owned StateDirectory and must separately approve this step.
    let metadata = fs::symlink_metadata(state).map_err(|_| "StateDirectory missing")?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o7777 != 0o700
    {
        return Err("unsafe enrollment StateDirectory");
    }
    // A previous enrollment, partial crash, or existing policy must be
    // repaired explicitly. Never silently rotate or fill missing keys.
    absent(owner_policy)?;
    absent(&state.join("node-identity.pk8"))?;
    absent(&state.join("node-iroh-seed.bin"))?;

    let node = NodeIdentity::enroll_first(state)
        .map_err(|_| "Node signing identity enrollment failed")?;
    let iroh = identity::enroll_first(state)?;
    let witness = NodeIdentity::open_provisioned(state)
        .map_err(|_| "Node signing identity readback failed")?;
    let iroh_witness = identity::load_provisioned(state)?;
    let result = PublicIdentityPair {
        node_public: node.public_key(),
        iroh_endpoint: *iroh.public().as_bytes(),
    };
    if result.node_public != witness.public_key()
        || result.iroh_endpoint != *iroh_witness.public().as_bytes()
    {
        return Err("enrolled public identity readback mismatch");
    }
    Ok(result)
}

pub(super) fn run_fixed() -> Result<(), &'static str> {
    let result = enroll_at(
        Path::new("/var/lib/akula-node"),
        Path::new("/etc/akula-node/owner-policy.json"),
    )?;
    let mut public_hex = String::with_capacity(64);
    for octet in result.node_public {
        use std::fmt::Write as _;
        write!(&mut public_hex, "{octet:02x}")
            .map_err(|_| "Node public identity encoding failed")?;
    }
    // Public values only. The signed root helper must independently pin
    // both in a v3 owner policy before it can activate systemd.
    let mut iroh_hex = String::with_capacity(64);
    for octet in result.iroh_endpoint {
        use std::fmt::Write as _;
        write!(&mut iroh_hex, "{octet:02x}")
            .map_err(|_| "Iroh public endpoint encoding failed")?;
    }
    println!("AKULA_NODE_V1_ENROLLED node={public_hex} iroh={iroh_hex}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_enrollment_is_persistent_and_never_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let policy = dir.path().join("owner-policy.json");
        let enrolled = enroll_at(&state, &policy).unwrap();
        assert_ne!(enrolled.node_public, [0; 32]);
        assert_ne!(enrolled.iroh_endpoint, [0; 32]);
        assert!(enroll_at(&state, &policy).is_err());
        assert_eq!(
            NodeIdentity::open_provisioned(&state).unwrap().public_key(),
            enrolled.node_public,
        );
        assert_eq!(
            *identity::load_provisioned(&state).unwrap().public().as_bytes(),
            enrolled.iroh_endpoint,
        );
        fs::remove_file(state.join("node-iroh-seed.bin")).unwrap();
        assert!(enroll_at(&state, &policy).is_err());
        assert!(!state.join("node-iroh-seed.bin").exists());
    }

    #[test]
    fn existing_policy_or_unsafe_state_refuses_any_key_creation() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let policy = dir.path().join("owner-policy.json");
        fs::write(&policy, b"existing policy").unwrap();
        assert!(enroll_at(&state, &policy).is_err());
        assert!(!state.join("node-identity.pk8").exists());
        fs::remove_file(&policy).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(enroll_at(&state, &policy).is_err());
        assert!(!state.join("node-iroh-seed.bin").exists());
    }
}
