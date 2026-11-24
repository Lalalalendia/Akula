use super::*;

const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4f";
const FACTS: &str = concat!(
    "AKULA_B0_OS=ID=ubuntu\n",
    "AKULA_B0_ARCH=x86_64\n",
    "AKULA_B0_SYSTEMD=yes\n",
    "AKULA_B0_SUDO=no\n",
    "AKULA_B0_CPU=8\n",
    "AKULA_B0_RAM=4096\n",
    "AKULA_B0_DISK=32768\n",
);

#[test]
fn shared_ssh_parser_checks_canonical_wire_header_and_length() {
    let mut expected = b"\0\0\0\x0bssh-ed25519\0\0\0\x20".to_vec();
    expected.extend(0u8..32);
    assert_eq!(decode_ed25519_wire(KEY).unwrap(), expected);
    for malformed in [
        KEY[..67].to_owned(), format!("{KEY}="),
        KEY.replacen('A', "!", 1), KEY.replacen('A', "B", 1),
        format!("é{}", &KEY[2..]),
    ] {
        assert!(matches!(decode_ed25519_wire(&malformed), Err(B0Error::KeyUnavailable)));
    }
}

#[test]
fn shared_ssh_parser_preserves_host_facts_and_unknown_platforms() {
    let facts = parse_facts(FACTS).unwrap();
    assert_eq!(facts.os, SupportedOs::Ubuntu);
    assert_eq!(facts.arch, MachineArch::X86_64);
    assert!(facts.has_systemd && !facts.has_sudo_rights);
    assert_eq!((facts.cpu_count, facts.ram_mib, facts.free_disk_mib), (8, 4096, 32768));
    assert_eq!(parse_facts(&FACTS.replace('\n', "\r\n")).unwrap(), facts);
    let other = parse_facts(&FACTS.replace("ubuntu", "unknown").replace("x86_64", "other")).unwrap();
    assert_eq!(other.os, SupportedOs::Other);
    assert_eq!(other.arch, MachineArch::Other);
}

#[test]
fn shared_ssh_parser_refuses_malformed_duplicate_missing_or_oversized_facts() {
    for invalid in [
        FACTS.replace("AKULA_B0_CPU=8\n", ""),
        format!("{FACTS}AKULA_B0_CPU=9\n"),
        FACTS.replace("AKULA_B0_CPU=8", "UNKNOWN=8"),
        FACTS.replace("AKULA_B0_CPU=8", "AKULA_B0_CPU"),
        FACTS.replace("ID=ubuntu", "ubuntu"),
        FACTS.replace("SYSTEMD=yes", "SYSTEMD=maybe"),
        FACTS.replace("CPU=8", "CPU=65536"),
        FACTS.replace("RAM=4096", "RAM=-1"),
        FACTS.replace("DISK=32768", "DISK=18446744073709551616"),
        FACTS.replace("ubuntu", &"x".repeat(101)),
        "x".repeat(MAX_OUTPUT as usize + 1),
    ] {
        assert!(matches!(parse_facts(&invalid), Err(B0Error::InspectionUnavailable)));
    }
}
