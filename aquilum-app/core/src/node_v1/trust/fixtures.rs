use super::*;

pub(in crate::node_v1) fn policy_fixture(parent: &Path, node_public: [u8; 32]) -> RootTrust {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(parent).unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    let policy = PolicyFile {
        version: 2,
        node_public,
        owner_public: [11; 32],
        vault_id: VaultId::new(),
        grant_epoch: 7,
        roster_digest: [21; 32],
        grant_digest: [31; 32],
        put_grant_digest: [41; 32],
    };
    let file = parent.join(POLICY_NAME);
    fs::write(&file, serde_json::to_vec(&policy).unwrap()).unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
    RootTrust::load_from(parent, rustix::process::geteuid().as_raw(), node_public).unwrap()
}

pub(in crate::node_v1) fn admitted_fixture(policy: &RootTrust) -> AdmittedRead {
    AdmittedRead {
        peer: [7; 32],
        grant_digest: policy.policy.grant_digest,
        policy_digest: policy.digest,
    }
}
