//! CI-only synthetic Ed25519 fixture for a *separately provisioned* protected
//! root policy. Never supplies any secret or owner approval to the helper.
//! Usage: example OUTDIR SEED_FILE nonroot_UID
use aquilum_core::server_installer_ipc_lab::{PreflightClaim, SignedPreflight};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const DOMAIN: &[u8] = b"AKULA-B1B3B-CI-TRUST-V1\0";

fn go() -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 4 { return Err("usage: fixture OUTPUT_DIRECTORY SEED_FILE UID".into()); }
    let out = Path::new(&args[1]);
    let md = fs::symlink_metadata(out).map_err(|e| e.to_string())?;
    if !md.is_dir() || md.file_type().is_symlink() {
        return Err("output must be pre-created private test directory".into());
    }
    let uid: u32 = args[3].parse().map_err(|_|"invalid user id")?;
    if uid == 0 { return Err("CI client must be nonroot".into()); }
    let seed: [u8;32] = fs::read(&args[2]).map_err(|e|e.to_string())?
        .try_into().map_err(|_|"seed must be exactly 32 bytes")?;
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_|"test seed is invalid")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e|e.to_string())?.as_secs();
    let claim = PreflightClaim {
        install_id: [0x11;16],
        plan_digest: [0x22;32],
        ssh_host_sha256: [0x33;32],
        signed_manifest_fingerprint: [0x44;32],
        package_sha256: [0x55;32],
        release_sequence: 37,
        witness_revision: 7,
        approval_nonce: [0x66;16],
        issued_at_unix: now.saturating_sub(60),
        expires_at_unix: now.checked_add(600).ok_or("invalid epoch")?,
    };
    fn sign(key: &Ed25519KeyPair, c: PreflightClaim) -> SignedPreflight {
        SignedPreflight {
            signature: key.sign(&c.canonical_signed_bytes()).as_ref().try_into().unwrap(),
            claim:c,
        }
    }
    let mut policy = Vec::new();
    policy.extend_from_slice(DOMAIN);
    policy.extend_from_slice(&uid.to_be_bytes());
    policy.extend_from_slice(pair.public_key().as_ref());
    policy.extend_from_slice(&claim.install_id);
    policy.extend_from_slice(&claim.plan_digest);
    policy.extend_from_slice(&claim.ssh_host_sha256);
    policy.extend_from_slice(&claim.signed_manifest_fingerprint);
    policy.extend_from_slice(&claim.package_sha256);
    policy.extend_from_slice(&claim.release_sequence.to_be_bytes());
    policy.extend_from_slice(&claim.witness_revision.to_be_bytes());
    fs::write(out.join("policy.bin"),policy).map_err(|e|e.to_string())?;

    let correct=sign(&pair, claim).wire_bytes();
    fs::write(out.join("valid.frame"),&correct).map_err(|e|e.to_string())?;
    let mut changed = correct.clone();
    // Corrupt the exact canonical plan digest, not the unsigned signature.
    changed[ b"Akula B1b3a helper signed preflight v1\0".len() + 2 + 16 + 4] ^= 1;
    fs::write(out.join("tampered.frame"),changed).map_err(|e|e.to_string())?;
    let mut wrong = claim;
    wrong.ssh_host_sha256[0] ^= 1;
    fs::write(out.join("wrong-host.frame"),sign(&pair,wrong).wire_bytes())
        .map_err(|e|e.to_string())?;
    let mut trailing=correct.clone();
    trailing.extend_from_slice(b"RUN /bin/sh");
    fs::write(out.join("trailing.frame"),trailing).map_err(|e|e.to_string())?;
    fs::write(out.join("truncated.frame"),&correct[..correct.len()-1])
        .map_err(|e|e.to_string())?;
    println!("B1B3B_FIXTURE_CREATED_NONROOT_UID={uid}");
    Ok(())
}
fn main() {
    if let Err(e)=go() {
        eprintln!("B1B3B_FIXTURE_REFUSED: {e}");
        std::process::exit(2);
    }
}
