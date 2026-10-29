//! One public SSH host identity field. No private key, password or TOFU.
use aquilum_core::settings::models::ServerBootstrapDraft;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use sha2::{Digest, Sha256};

pub(super) fn display(draft: &ServerBootstrapDraft) -> String {
    if draft.host_public_key.is_empty() {
        draft.host_key_fingerprint.clone()
    } else {
        draft.host_public_key.clone()
    }
}

pub(super) fn update(draft: &mut ServerBootstrapDraft, input: &str) {
    let input = input.trim();
    if !input.is_empty()
        && input.starts_with("ssh-ed25519 ")
        && ServerBootstrapDraft::valid_public_key(input)
        && let Some(encoded) = input.split_ascii_whitespace().nth(1)
        && let Ok(wire) = STANDARD_NO_PAD.decode(encoded)
        && wire.len() == 51
    {
        let hash = Sha256::digest(&wire);
        draft.host_public_key = input.to_owned();
        draft.host_key_fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode(hash));
        return;
    }
    if fingerprint_bytes(input).is_some() {
        draft.host_public_key.clear();
        draft.host_key_fingerprint = input.to_owned();
        return;
    }
    // Invalid or secret-looking input invalidates any prior pin. Never
    // silently verify against a stale identity after a rejected paste.
    draft.host_public_key.clear();
    draft.host_key_fingerprint.clear();
}

fn fingerprint_bytes(value: &str) -> Option<[u8; 32]> {
    if !ServerBootstrapDraft::valid_fingerprint(value) || value.is_empty() {
        return None;
    }
    let encoded = value.strip_prefix("SHA256:")?;
    let decoded = STANDARD_NO_PAD.decode(encoded).ok()?;
    if decoded.len() != 32 || STANDARD_NO_PAD.encode(&decoded) != encoded {
        return None;
    }
    decoded.try_into().ok()
}

/// A single independent public identity is sufficient for a real signed SSH
/// KEX, but never constitutes approval to install or trust an unknown host.
pub(crate) fn can_probe(draft: &ServerBootstrapDraft) -> bool {
    !draft.host.is_empty()
        && !draft.ssh_user.is_empty()
        && ServerBootstrapDraft::valid_host(&draft.host)
        && ServerBootstrapDraft::valid_user(&draft.ssh_user)
        && ServerBootstrapDraft::valid_port(draft.ssh_port)
        && fingerprint_bytes(&draft.host_key_fingerprint).is_some()
        && (draft.host_public_key.is_empty() || draft.valid_offline_pin())
}

#[cfg(all(windows, feature = "server-readonly-preflight"))]
pub(crate) fn probe_target(
    draft: &ServerBootstrapDraft,
) -> Option<aquilum_core::server_bootstrap_lab::ServerTarget> {
    use aquilum_core::server_bootstrap_lab::ServerTarget;
    if !can_probe(draft) {
        return None;
    }
    let target = ServerTarget {
        host: draft.host.clone(),
        ssh_port: draft.ssh_port,
        ssh_user: draft.ssh_user.clone(),
        trusted_key_sha256: fingerprint_bytes(&draft.host_key_fingerprint)?,
    };
    target.validate().ok()?;
    Some(target)
}
