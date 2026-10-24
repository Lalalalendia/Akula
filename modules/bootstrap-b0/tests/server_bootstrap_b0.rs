//! AKULA-SERVER-BOOTSTRAP B0: no real SSH or host modifications.
//! The FakeProbe asserts that host facts are never requested before a pinned
//! SHA-256 SSH host-key match. No credentials exist in this contract.

use akula_server_bootstrap_b0::{
    preview, B0Error, B0Input, MachineArch, PackagePin, PreviewTransport, ProposedChange,
    ReadOnlyHostFacts, ServerTarget, SupportedOs,
};
use ring::digest;
use std::cell::Cell;
use uuid::Uuid;

const PUBKEY: &[u8] = b"ssh-ed25519 fake-host-public-key test fixture -- this is not a secret";

fn fingerprint() -> [u8; 32] {
    let bytes = digest::digest(&digest::SHA256, PUBKEY);
    bytes.as_ref().try_into().unwrap()
}

fn input() -> B0Input {
    B0Input {
        install_id: Uuid::from_u128(1),
        target: ServerTarget {
            host: "my-server.example.net".into(),
            ssh_port: 22,
            ssh_user: "akulaadmin".into(),
            trusted_key_sha256: fingerprint(),
        },
        package: PackagePin {
            version: "0.1.0-b0".into(),
            package_sha256: [17u8; 32],
            trusted_signer_id: "akula-official-test-key".into(),
        },
    }
}

fn facts() -> ReadOnlyHostFacts {
    ReadOnlyHostFacts {
        os: SupportedOs::Debian,
        arch: MachineArch::X86_64,
        has_systemd: true,
        has_sudo_rights: true,
        cpu_count: 2,
        ram_mib: 2048,
        free_disk_mib: 8192,
    }
}

struct FakeProbe {
    key: Vec<u8>,
    facts: ReadOnlyHostFacts,
    key_calls: usize,
    fact_calls: usize,
    reject_key: bool,
    reject_probe: bool,
}

impl FakeProbe {
    fn valid() -> Self {
        Self {
            key: PUBKEY.to_vec(),
            facts: facts(),
            key_calls: 0,
            fact_calls: 0,
            reject_key: false,
            reject_probe: false,
        }
    }
}

impl PreviewTransport for FakeProbe {
    fn handshake_public_key(
        &mut self,
        _target: &ServerTarget,
    ) -> Result<Vec<u8>, B0Error> {
        self.key_calls += 1;
        if self.reject_key { return Err(B0Error::KeyUnavailable); }
        Ok(self.key.clone())
    }

    fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
        self.fact_calls += 1;
        if self.reject_probe { return Err(B0Error::InspectionUnavailable); }
        Ok(self.facts.clone())
    }
}

#[test]
fn valid_dry_run_is_stable_and_does_not_approve_or_install_anything() {
    let i = input();
    let mut a = FakeProbe::valid();
    let mut b = FakeProbe::valid();
    let first = preview(&i, &mut a, || false).unwrap();
    let second = preview(&i, &mut b, || false).unwrap();
    assert_eq!(first, second);
    assert_eq!(a.key_calls, 1);
    assert_eq!(a.fact_calls, 1);
    assert_eq!(first.plan_blake3.len(), 64);
    assert!(first.verify_unchanged());
    assert_eq!(first.schema_version, 1);
    assert_eq!(first.proposed_changes.len(), 6);
    assert_eq!(first.proposed_changes[0], ProposedChange::VerifySignedPinnedPackage);

    let mut changed = first.clone();
    changed.target.host = "other.example.net".into();
    assert!(!changed.verify_unchanged(), "plan digest must bind target address");
    let mut changed = first.clone();
    changed.package.package_sha256[0] ^= 1;
    assert!(!changed.verify_unchanged(), "plan digest must bind package digest");
    let mut changed = first.clone();
    changed.inspected_host.free_disk_mib += 1;
    assert!(!changed.verify_unchanged(), "plan digest must bind facts");
    let mut changed = first.clone();
    changed.proposed_changes.pop();
    assert!(!changed.verify_unchanged(), "cannot silently omit a promised action");

    // The B0 output is only a plan. There is no 'install', 'sudo',
    // 'write_file' or network bind method in the PreviewTransport API.
    let serialized = serde_json::to_string(&first).unwrap();
    assert!(!serialized.contains("password"));
    assert!(!serialized.contains("private_key"));
}

#[test]
fn fake_mitm_or_new_unapproved_host_key_fails_before_any_remote_inspection() {
    let mut fake = FakeProbe::valid();
    fake.key = b"ssh-ed25519 attacker replaces public key before handshake".to_vec();
    let res = preview(&input(), &mut fake, || false);
    assert_eq!(res.unwrap_err(), B0Error::HostKeyMismatch);
    assert_eq!(fake.key_calls, 1);
    assert_eq!(fake.fact_calls, 0, "never ask remote facts before pinned host key match");

    let mut empty = FakeProbe::valid();
    empty.key.clear();
    assert_eq!(preview(&input(), &mut empty, || false).unwrap_err(),
        B0Error::KeyUnavailable);
    assert_eq!(empty.fact_calls, 0);
}

#[test]
fn input_validation_does_not_even_contact_the_fake_transport() {
    let mut bad = input();
    bad.target.host = "-oProxyCommand=evil".into();
    for target in [
        bad,
        { let mut v = input(); v.target.host = "x;curl".into(); v },
        { let mut v = input(); v.target.host = "../x".into(); v },
        { let mut v = input(); v.target.host = "evil host".into(); v },
        { let mut v = input(); v.target.ssh_user = "root".into(); v },
        { let mut v = input(); v.target.ssh_user = "admin;id".into(); v },
        { let mut v = input(); v.target.ssh_port = 0; v },
        { let mut v = input(); v.target.trusted_key_sha256 = [0;32]; v },
        { let mut v = input(); v.install_id = Uuid::nil(); v },
    ] {
        let mut fake = FakeProbe::valid();
        assert_eq!(preview(&target, &mut fake, || false).unwrap_err(),
            B0Error::InvalidTarget);
        assert_eq!(fake.key_calls, 0);
        assert_eq!(fake.fact_calls, 0);
    }
    let mut valid_ip = input();
    valid_ip.target.host = "2001:db8::1".into();
    assert!(preview(&valid_ip, &mut FakeProbe::valid(), || false).is_ok(),
        "IPv6 literals should be accepted as structured addresses");
}

#[test]
fn package_version_and_signer_must_be_pinned_locally() {
    let bad_packages = [
        { let mut v = input(); v.package.version = "latest;curl".into(); v },
        { let mut v = input(); v.package.version.clear(); v },
        { let mut v = input(); v.package.package_sha256 = [0; 32]; v },
        { let mut v = input(); v.package.trusted_signer_id = "../file".into(); v },
        { let mut v = input(); v.package.trusted_signer_id.clear(); v },
    ];
    for bad in bad_packages {
        let mut fake = FakeProbe::valid();
        assert_eq!(preview(&bad, &mut fake, || false).unwrap_err(),
            B0Error::InvalidPackage);
        assert_eq!(fake.key_calls, 0);
        assert_eq!(fake.fact_calls, 0);
    }
}

#[test]
fn unsupported_remote_observations_fail_closed_without_a_plan() {
    let cases = [
        ({ let mut f = facts(); f.os = SupportedOs::Other; f }, B0Error::UnsupportedOs),
        ({ let mut f = facts(); f.arch = MachineArch::Aarch64; f }, B0Error::UnsupportedArchitecture),
        ({ let mut f = facts(); f.has_systemd = false; f }, B0Error::MissingSystemd),
        ({ let mut f = facts(); f.has_sudo_rights = false; f }, B0Error::MissingPrivilege),
        ({ let mut f = facts(); f.ram_mib = 255; f }, B0Error::InsufficientResources),
        ({ let mut f = facts(); f.free_disk_mib = 100; f }, B0Error::InsufficientResources),
        ({ let mut f = facts(); f.cpu_count = 0; f }, B0Error::InsufficientResources),
    ];
    for (facts, error) in cases {
        let mut fake = FakeProbe::valid();
        fake.facts = facts;
        assert_eq!(preview(&input(), &mut fake, || false).unwrap_err(), error);
        assert_eq!(fake.key_calls, 1);
        assert_eq!(fake.fact_calls, 1);
    }
}

#[test]
fn cancellation_is_respected_before_handshake_after_key_and_after_readonly_facts() {
    for cancel_on_call in [1, 2, 3] {
        let calls = Cell::new(0usize);
        let mut fake = FakeProbe::valid();
        let result = preview(&input(), &mut fake, || {
            calls.set(calls.get()+1);
            calls.get() == cancel_on_call
        });
        assert_eq!(result.unwrap_err(), B0Error::Cancelled);
        assert_eq!(fake.key_calls, usize::from(cancel_on_call > 1));
        assert_eq!(fake.fact_calls, usize::from(cancel_on_call > 2));
    }
}

#[test]
fn probe_failures_do_not_turn_into_success_or_execution_authority() {
    let mut a = FakeProbe::valid();
    a.reject_key = true;
    assert_eq!(preview(&input(), &mut a, || false).unwrap_err(),
        B0Error::KeyUnavailable);
    assert_eq!(a.fact_calls, 0);

    let mut b = FakeProbe::valid();
    b.reject_probe = true;
    assert_eq!(preview(&input(), &mut b, || false).unwrap_err(),
        B0Error::InspectionUnavailable);
    assert_eq!(b.key_calls, 1);
    assert_eq!(b.fact_calls, 1);
}

#[test]
fn changed_target_or_install_id_requires_a_different_approval_digest() {
    let mut original = FakeProbe::valid();
    let a = preview(&input(), &mut original, || false).unwrap();
    let mut other_install = input();
    other_install.install_id = Uuid::from_u128(2);
    let b = preview(&other_install, &mut FakeProbe::valid(), || false).unwrap();
    assert_ne!(a.plan_blake3, b.plan_blake3);
    assert_ne!(a.install_id, b.install_id);

    let mut another_target = input();
    another_target.target.ssh_port = 2222;
    let c = preview(&another_target, &mut FakeProbe::valid(), || false).unwrap();
    assert_ne!(a.plan_blake3, c.plan_blake3);
}
