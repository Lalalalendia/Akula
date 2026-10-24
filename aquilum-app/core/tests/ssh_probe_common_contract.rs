#![cfg(feature = "server-bootstrap-b0-openssh-lab")]

use aquilum_core::server_bootstrap_lab;
use server_bootstrap_lab::{B0Error, MachineArch, ReadOnlyHostFacts, SupportedOs};

#[path = "../src/server_bootstrap_ssh_common.rs"]
#[allow(dead_code)]
mod common;

const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcH";
const FACTS: &str = "AKULA_B0_OS=ID=ubuntu\nAKULA_B0_ARCH=x86_64\nAKULA_B0_SYSTEMD=yes\nAKULA_B0_SUDO=no\nAKULA_B0_CPU=8\nAKULA_B0_RAM=8192\nAKULA_B0_DISK=65536\n";

#[test]
fn public_key_wire_keeps_exact_framing_and_rejects_invalid_encoding() {
    let wire = common::decode_ed25519_wire(KEY).unwrap();
    assert_eq!(&wire[..19], b"\0\0\0\x0bssh-ed25519\0\0\0\x20");
    assert_eq!(&wire[19..], &[7; 32]);
    for position in 0..KEY.len() {
        let mut bad = KEY.as_bytes().to_vec();
        bad[position] = b'=';
        assert_eq!(common::decode_ed25519_wire(std::str::from_utf8(&bad).unwrap()), Err(B0Error::KeyUnavailable));
    }
    for bad in [String::new(), format!("{KEY}="), KEY[..67].to_owned(), KEY.replacen('A', "B", 1)] {
        assert_eq!(common::decode_ed25519_wire(&bad), Err(B0Error::KeyUnavailable));
    }
}

#[test]
fn host_facts_keep_value_boundaries_line_order_and_malformed_rejection() {
    let expected = ReadOnlyHostFacts {
        os: SupportedOs::Ubuntu, arch: MachineArch::X86_64,
        has_systemd: true, has_sudo_rights: false, cpu_count: 8,
        ram_mib: 8192, free_disk_mib: 65536,
    };
    for valid in [FACTS.to_owned(), FACTS.replace('\n', "\r\n"), FACTS.lines().rev().collect::<Vec<_>>().join("\n")] {
        assert_eq!(common::parse_facts(&valid).unwrap(), expected);
    }
    let unknown = common::parse_facts(&FACTS.replace("ubuntu", "other").replace("x86_64", "other").replace("CPU=8", "CPU=0")).unwrap();
    assert_eq!((unknown.os, unknown.arch, unknown.cpu_count), (SupportedOs::Other, MachineArch::Other, 0));
    let maximum = common::parse_facts(&FACTS.replace("CPU=8", "CPU=65535").replace("RAM=8192", "RAM=18446744073709551615")).unwrap();
    assert_eq!((maximum.cpu_count, maximum.ram_mib), (u16::MAX, u64::MAX));
    for bad in [
        FACTS.replace("AKULA_B0_SUDO=no\n", ""), format!("{FACTS}AKULA_B0_CPU=8\n"),
        FACTS.replace("AKULA_B0_RAM", "UNKNOWN"), FACTS.replace("SYSTEMD=yes", "SYSTEMD=YES"),
        FACTS.replace("CPU=8", "CPU=65536"), FACTS.replace("RAM=8192", "RAM=-1"),
        FACTS.replace("RAM=8192", "RAM=18446744073709551616"),
        FACTS.replace("ID=ubuntu", &format!("ID={}", "x".repeat(101))), "x".repeat(8193),
    ] {
        assert_eq!(common::parse_facts(&bad), Err(B0Error::InspectionUnavailable));
    }
}
