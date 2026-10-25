//! One disposable UID0 transaction proving that a *clean* server's three
//! independently authorized admin ceremonies compose without treating the
//! empty filesystem, a CI witness or SSH traffic as an approval.
use super::{destination_init, enrollment, witness_init, *};
use crate::server_installer_os_witness_lab::OsWitnessLab;
use crate::server_witness_lab::WitnessCheckpoint;
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

fn canonical_test_policy() -> Vec<u8> {
    let owner = Ed25519KeyPair::from_seed_unchecked(&[0x81; 32]).unwrap();
    let publisher = Ed25519KeyPair::from_seed_unchecked(&[0x82; 32]).unwrap();
    let host = digest(&SHA256, b"isolated-ci-pinned-ssh-host");
    let helper = digest(&SHA256, b"isolated-ci-verified-root-helper");
    let mut data = DOMAIN.to_vec();
    data.push(3);
    let owner_name = b"owner-e2e-ci";
    data.push(owner_name.len() as u8);
    data.extend_from_slice(owner_name);
    data.extend_from_slice(owner.public_key().as_ref());
    data.extend_from_slice(host.as_ref());
    data.extend_from_slice(helper.as_ref());
    let publisher_name = b"publisher-e2e-ci";
    data.push(publisher_name.len() as u8);
    data.extend_from_slice(publisher_name);
    data.extend_from_slice(publisher.public_key().as_ref());
    data
}

#[test]
fn clean_root_genesis_to_owner_policy_to_empty_node_destination_persists() {
    if rustix::process::geteuid().as_raw() != 0 {
        // The branch CI repeats this exact test binary as UID0. A nonroot
        // run cannot create an authoritative witness or a local Node.
        return;
    }
    let tmp = tempdir().unwrap();
    let lib = tmp.path().join("var/lib");
    let opt = tmp.path().join("opt");
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir(&opt).unwrap();
    for parent in [&lib, &opt] {
        fs::set_permissions(parent, Permissions::from_mode(0o755)).unwrap();
    }
    let lib_fd = File::open(&lib).unwrap();
    let opt_fd = File::open(&opt).unwrap();
    let trust = lib.join("akula/server-bootstrap/trust");
    let journal = lib.join("akula/server-bootstrap/journal");
    let destination = opt.join("akula-node");
    assert!(!trust.exists() && !journal.exists() && !destination.exists());

    // Step 1: root-generated signing key and explicit genesis, never a
    // caller-supplied test seed and never an implicit unmarked directory.
    witness_init::initialize_once(&lib_fd, witness_init::Stop::Never).unwrap();
    let signer_public = fs::read(trust.join("witness.pub")).unwrap();
    let signer_seed = fs::read(trust.join("witness.seed")).unwrap();
    let generated = Ed25519KeyPair::from_seed_unchecked(&signer_seed).unwrap();
    assert_eq!(generated.public_key().as_ref(), signer_public.as_slice());
    let actor = OsWitnessLab::open_provisioned_fixture(&trust, &journal)
        .expect("real B1d0 signer and protected ledger reopen");
    assert_eq!(actor.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());
    assert!(!destination.exists());

    let trust_fd = File::open(&trust).unwrap();
    assert!(RootEnrolledV3::from_trust_fd(&trust_fd).is_err());

    // Step 2: independently signed owner/publisher public identities
    // supplied by the authorized administrator, not by the remote plan.
    let policy = canonical_test_policy();
    enrollment::enroll_once_from_admin_bytes(&trust_fd, &policy).unwrap();
    let reloaded = RootEnrolledV3::from_trust_fd(&File::open(&trust).unwrap()).unwrap();
    assert_eq!(reloaded.owner_id, "owner-e2e-ci");
    assert_eq!(reloaded.release_signer_id, "publisher-e2e-ci");
    assert_eq!(reloaded.helper_sha256.as_slice(),
        digest(&SHA256, b"isolated-ci-verified-root-helper").as_ref());
    assert_eq!(fs::read(trust.join("b0v3.policy")).unwrap(), policy);

    // Step 3: empty /opt root-owned destination. No ELF, service or CAS.
    destination_init::first_opt_destination(&opt_fd).unwrap();
    assert!(root_dir_ok(&fs::metadata(&destination).unwrap(), 0o755));
    assert_eq!(fs::read(destination.join("enrollment")).unwrap(),
        b"AKULA-B1D0-PROVISIONED-FIRST-INSTALL-V1\n");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    assert_eq!(actor.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());

    // Simulate an independent process reopening after reboot; nothing
    // depends on the original admin File descriptors or in-memory keys.
    drop(actor);
    drop(trust_fd);
    drop(opt_fd);
    drop(lib_fd);
    let reopened = OsWitnessLab::open_provisioned_fixture(&trust, &journal).unwrap();
    assert_eq!(reopened.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());
    assert!(RootEnrolledV3::from_trust_fd(&File::open(&trust).unwrap()).is_ok());

    // Re-executing ANY administrator ceremony is not a grant, not an
    // overwrite and never resets the freshly generated witness identity.
    let lib_fd = File::open(&lib).unwrap();
    let opt_fd = File::open(&opt).unwrap();
    assert!(witness_init::initialize_once(&lib_fd, witness_init::Stop::Never).is_err());
    assert!(enrollment::enroll_once_from_admin_bytes(
        &File::open(&trust).unwrap(), &policy,
    ).is_err());
    assert!(destination_init::first_opt_destination(&opt_fd).is_err());
    assert_eq!(fs::read(trust.join("witness.pub")).unwrap(), signer_public);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    assert_eq!(reopened.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());
    println!("B1_E2E07_CLEAN_ROOT_WITNESS_POLICY_OPT_CHAIN_HOLD_PASS");
}
