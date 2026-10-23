//! Opt-in Windows actual ssh.exe signed KEX against an ephemeral loopback
//! Paramiko SSH server. Independent fixture host A/B identities, no secrets
//! from a real user and no external SSH target.
#![cfg(all(windows, feature = "server-bootstrap-b0-openssh-lab"))]

use aquilum_core::server_bootstrap_lab::{
    preview, B0Error, B0Input, MachineArch, PackagePin, PreviewTransport,
    ServerTarget, SupportedOs,
};
use aquilum_core::server_bootstrap_ssh_windows_lab::WindowsOpenSshPinnedProbe;
use ring::digest;
use uuid::Uuid;

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing disposable fixture {name}"))
}
fn key_wire(public: &str) -> Vec<u8> {
    let mut p = public.split_ascii_whitespace();
    assert_eq!(p.next(), Some("ssh-ed25519"));
    let value = p.next().unwrap();
    assert!(p.next().is_none());
    assert_eq!(value.len(), 68);
    let digit = |x: u8| -> u32 {
        match x {
            b'A'..=b'Z' => (x - b'A') as u32,
            b'a'..=b'z' => (x - b'a' + 26) as u32,
            b'0'..=b'9' => (x - b'0' + 52) as u32,
            b'+' => 62, b'/' => 63, _ => panic!("invalid base64"),
        }
    };
    let mut bytes = Vec::new();
    for c in value.as_bytes().chunks_exact(4) {
        let (a,b,c,d) = (digit(c[0]),digit(c[1]),digit(c[2]),digit(c[3]));
        bytes.extend_from_slice(&[
            ((a<<2)|(b>>4)) as u8, ((b<<4)|(c>>2)) as u8,
            ((c<<6)|d) as u8,
        ]);
    }
    assert_eq!(bytes.len(), 51);
    bytes
}
fn input(public: &str) -> B0Input {
    let host_hash: [u8;32] = digest::digest(
        &digest::SHA256,&key_wire(public)
    ).as_ref().try_into().unwrap();
    B0Input {
        install_id: Uuid::from_u128(0x17),
        target: ServerTarget {
            host: "127.0.0.1".into(),
            ssh_port: var("AKULA_B0C_PORT").parse().unwrap(),
            ssh_user: "ubuntu".into(),
            trusted_key_sha256: host_hash,
        },
        package: PackagePin {
            version: "0.1.0-b0c".into(),
            package_sha256: [0x33;32],
            trusted_signer_id: "akula-ci-test".into(),
        },
    }
}

#[test]
fn real_windows_openssh_strict_signed_host_kex_and_fixed_diagnostics() {
    let host_a = var("AKULA_B0C_PUBLIC_A");
    let request = input(&host_a);
    let mut transport = WindowsOpenSshPinnedProbe::new(&request,&host_a).unwrap();
    let observed = transport.handshake_public_key(&request.target).unwrap();
    assert_eq!(observed, key_wire(&host_a));
    let facts = transport.inspect_host().unwrap();
    // This is the exact typed call the native Settings Verify button will use:
    // no fake package pin or install approval enters a read-only probe.
    let read_only = WindowsOpenSshPinnedProbe::inspect_readonly_target(
        &request.target, &host_a,
    ).expect("real signed host A and bounded B0 facts");
    assert_eq!(read_only, facts);
    // An independently obtained fingerprint alone selects the candidate;
    // only the subsequent signed ssh.exe host KEX grants read-only access.
    let fingerprint_only =
        WindowsOpenSshPinnedProbe::inspect_readonly_fingerprint_target(&request.target)
            .expect("fingerprint-only signed KEX and bounded read-only preflight");
    assert_eq!(fingerprint_only, facts);
    assert_eq!(facts.os,SupportedOs::Ubuntu);
    assert_eq!(facts.arch,MachineArch::X86_64);
    assert!(facts.has_systemd && facts.has_sudo_rights);
    assert!(facts.cpu_count > 0 && facts.ram_mib >= 512);
    let result = preview(&request,&mut transport,||false).unwrap();
    assert!(result.verify_unchanged());
    assert_eq!(result.target.trusted_key_sha256,request.target.trusted_key_sha256);
    println!("B0C_WIN_OPENSSH_REAL_SIGNED_KEX_AND_FIXED_PREVIEW_PASS");
}
#[test]
fn fake_server_a_cannot_pass_independent_pinned_host_b() {
    let host_b = var("AKULA_B0C_PUBLIC_B");
    let request = input(&host_b);
    // Reference B is internally consistent. Server's signed KEX is from A.
    let mut transport = WindowsOpenSshPinnedProbe::new(&request,&host_b).unwrap();
    assert_eq!(transport.handshake_public_key(&request.target).unwrap_err(),
        B0Error::InspectionUnavailable);
    assert_eq!(preview(&request,&mut transport,||false).unwrap_err(),
        B0Error::InspectionUnavailable);
    assert_eq!(
        WindowsOpenSshPinnedProbe::inspect_readonly_target(&request.target, &host_b)
            .unwrap_err(),
        B0Error::InspectionUnavailable,
    );
    assert!(
        WindowsOpenSshPinnedProbe::inspect_readonly_fingerprint_target(&request.target)
            .is_err(),
        "the host A cannot satisfy independently pinned host B fingerprint",
    );
    println!("B0C_WIN_FORGED_SIGNED_KEX_REFUSED_PASS");
}
#[test]
fn bad_input_and_key_digest_never_contact_any_ssh_server() {
    let host_a=var("AKULA_B0C_PUBLIC_A");
    let host_b=var("AKULA_B0C_PUBLIC_B");
    let request=input(&host_a);
    assert!(matches!(
        WindowsOpenSshPinnedProbe::new(&request,&host_b),
        Err(B0Error::HostKeyMismatch)
    ));
    assert!(matches!(
        WindowsOpenSshPinnedProbe::inspect_readonly_target(&request.target,&host_b),
        Err(B0Error::HostKeyMismatch)
    ));
    assert!(matches!(
        WindowsOpenSshPinnedProbe::new(&request,
            "-----BEGIN OPENSSH PRIVATE KEY-----"),
        Err(B0Error::KeyUnavailable)
    ));
    let mut malicious=input(&host_a);
    malicious.target.host="-oProxyCommand=bad".into();
    assert!(matches!(
        WindowsOpenSshPinnedProbe::new(&malicious,&host_a),
        Err(B0Error::InvalidTarget)
    ));
    let mut root=input(&host_a);
    root.target.ssh_user="root".into();
    assert!(matches!(
        WindowsOpenSshPinnedProbe::new(&root,&host_a),
        Err(B0Error::InvalidTarget)
    ));
    println!("B0C_WIN_NO_TOFU_SHELL_COMMAND_OR_ROOT_LOGIN_PASS");
}
