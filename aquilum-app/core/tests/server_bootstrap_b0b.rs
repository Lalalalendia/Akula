//! Real Linux loopback sshd KEX admission tests, developer CI only.
//! No SSH target is taken from the user or from a remote service.
//! This test REQUIRES the disposable sshd fixture prepared by its workflow.

#![cfg(all(target_os = "linux", feature = "server-bootstrap-b0-openssh-lab"))]

use aquilum_core::server_bootstrap_lab::{
    preview, B0Error, B0Input, PackagePin, PreviewTransport, ServerTarget,
    MachineArch, SupportedOs,
};
use aquilum_core::server_bootstrap_ssh_lab::OpenSshPinnedProbe;
use ring::digest;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn fixture(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required disposable CI fixture missing: {name}"))
}

fn decode_fingerprint(pub_key: &str) -> [u8;32] {
    let base64 = pub_key.split_ascii_whitespace().nth(1).expect("encoded test key");
    assert_eq!(base64.len(), 68);
    let decode = |v: u8| -> u32 {
        match v {
            b'A'..=b'Z' => (v-b'A') as u32,
            b'a'..=b'z' => (v-b'a'+26) as u32,
            b'0'..=b'9' => (v-b'0'+52) as u32,
            b'+' => 62, b'/' => 63,
            _ => panic!("bad key fixture encoding"),
        }
    };
    let mut wire = Vec::new();
    for c in base64.as_bytes().chunks_exact(4) {
        let a=decode(c[0]);let b=decode(c[1]);let d=decode(c[2]);let e=decode(c[3]);
        wire.extend_from_slice(&[
            ((a<<2)|(b>>4)) as u8,
            ((b<<4)|(d>>2)) as u8,
            ((d<<6)|e) as u8,
        ]);
    }
    digest::digest(&digest::SHA256,&wire).as_ref().try_into().unwrap()
}

fn input(public_key: &str) -> B0Input {
    B0Input {
        install_id: Uuid::from_u128(7),
        target: ServerTarget {
            host: "127.0.0.1".into(),
            ssh_port: fixture("AKULA_B0B_SSH_PORT").parse().unwrap(),
            ssh_user: fixture("AKULA_B0B_SSH_USER"),
            trusted_key_sha256: decode_fingerprint(public_key),
        },
        package: PackagePin {
            version: "0.1.0-b0".into(),
            package_sha256: [21u8;32],
            trusted_signer_id: "akula-test-fixture".into(),
        },
    }
}

fn key(name: &str) -> PathBuf { PathBuf::from(fixture(name)) }

fn host_a() -> String { fixture("AKULA_B0B_HOST_A") }
fn host_b() -> String { fixture("AKULA_B0B_HOST_B") }

#[test]
fn authentic_real_ssh_kex_only_precedes_fixed_readonly_diagnostics() {
    let input = input(&host_a());
    let mut probe = OpenSshPinnedProbe::new(
        &input, &host_a(), &key("AKULA_B0B_CLIENT_KEY")
    ).expect("trusted pinned reference and protected identity");
    let negotiated = probe.handshake_public_key(&input.target).expect("real SSH KEX+noop");
    assert_eq!(digest::digest(&digest::SHA256,&negotiated).as_ref(),
        input.target.trusted_key_sha256.as_slice());

    let facts = probe.inspect_host().expect("authenticated SSH fixed read-only probe");
    assert!(matches!(facts.os,SupportedOs::Debian|SupportedOs::Ubuntu));
    assert_eq!(facts.arch, MachineArch::X86_64);
    assert!(facts.cpu_count > 0);
    assert!(facts.ram_mib >= 512);
    assert!(facts.free_disk_mib >= 1024);

    // This calls a second strict KEX and then the same read-only facts.
    // A disposable runner without systemd/sudo MAY be refused as unsupported;
    // it must never claim an installable plan on insufficient host facts.
    let result = preview(&input,&mut probe,||false);
    let failure = result.as_ref().err().copied();
    assert!(result.is_ok() || matches!(
        failure, Some(B0Error::MissingSystemd | B0Error::MissingPrivilege)
    ),"unexpected transport/plan error: {result:?}");
    if let Ok(plan) = result {
        assert!(plan.verify_unchanged());
    }
}

#[test]
fn forged_server_host_key_is_rejected_during_signed_kex_before_auth() {
    // Server sshd is configured with host A. The independent trusted reference
    // is B, so it MUST never become an accepted host just because it is reachable.
    let input = input(&host_b());
    let mut probe = OpenSshPinnedProbe::new(
        &input, &host_b(), &key("AKULA_B0B_CLIENT_KEY")
    ).expect("internally consistent, but WRONG trusted host key");
    assert_eq!(probe.handshake_public_key(&input.target).unwrap_err(),
        B0Error::InspectionUnavailable);
    assert_eq!(preview(&input,&mut probe,||false).unwrap_err(),
        B0Error::InspectionUnavailable);
}

#[test]
fn trusted_key_and_independent_sha256_mismatch_fails_offline() {
    let input = input(&host_a());
    assert!(matches!(
        OpenSshPinnedProbe::new(&input,&host_b(),&key("AKULA_B0B_CLIENT_KEY")),
        Err(B0Error::HostKeyMismatch)
    ));
    assert!(matches!(
        OpenSshPinnedProbe::new(&input,"ssh-rsa AAAA",&key("AKULA_B0B_CLIENT_KEY")),
        Err(B0Error::KeyUnavailable)
    ));
    assert!(matches!(
        OpenSshPinnedProbe::new(&input,"ssh-ed25519 !!!",&key("AKULA_B0B_CLIENT_KEY")),
        Err(B0Error::KeyUnavailable)
    ));
}

#[test]
fn wrong_client_key_cannot_become_an_authenticated_probe() {
    let input = input(&host_a());
    let mut probe = OpenSshPinnedProbe::new(
        &input, &host_a(), &key("AKULA_B0B_WRONG_CLIENT_KEY")
    ).unwrap();
    assert_eq!(probe.handshake_public_key(&input.target).unwrap_err(),
        B0Error::InspectionUnavailable);
}

#[test]
fn direct_probe_constructor_cannot_bypass_b0_target_validation() {
    let mut root_user = input(&host_a());
    root_user.target.ssh_user = "root".into();
    assert!(matches!(
        OpenSshPinnedProbe::new(&root_user,&host_a(),&key("AKULA_B0B_CLIENT_KEY")),
        Err(B0Error::InvalidTarget)
    ));
    let mut malicious_host = input(&host_a());
    malicious_host.target.host = "-oProxyCommand=bad".into();
    assert!(matches!(
        OpenSshPinnedProbe::new(&malicious_host,&host_a(),&key("AKULA_B0B_CLIENT_KEY")),
        Err(B0Error::InvalidTarget)
    ));
    let mut untrusted_release = input(&host_a());
    untrusted_release.package.version = "latest;curl".into();
    assert!(matches!(
        OpenSshPinnedProbe::new(&untrusted_release,&host_a(),&key("AKULA_B0B_CLIENT_KEY")),
        Err(B0Error::InvalidPackage)
    ));
}

#[test]
fn unprotected_private_identity_is_refused_without_network_use() {
    let input = input(&host_a());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wrong_permissions");
    fs::write(&path,b"public fake fixture does not carry a secret").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        OpenSshPinnedProbe::new(&input,&host_a(),&path),
        Err(B0Error::InvalidTarget)
    ));
    let alias = temp.path().join("link");
    std::os::unix::fs::symlink(&key("AKULA_B0B_CLIENT_KEY"), &alias).unwrap();
    assert!(matches!(
        OpenSshPinnedProbe::new(&input,&host_a(),&alias),
        Err(B0Error::InvalidTarget)
    ));
    assert!(Path::new("/usr/bin/ssh").exists());
}
