//! Shared, bounded SSH wire and diagnostic protocol for the Linux and Windows probes.

use crate::server_bootstrap_lab::{B0Error, MachineArch, ReadOnlyHostFacts, SupportedOs};
use std::collections::BTreeMap;

pub(super) const MAX_OUTPUT: u64 = 8192;
pub(super) const FACTS_COMMAND: &str = concat!(
    "LC_ALL=C; export LC_ALL; ",
    "printf 'AKULA_B0_OS='; grep -m1 '^ID=' /etc/os-release || printf 'ID=unknown\\n'; ",
    "printf 'AKULA_B0_ARCH='; uname -m; ",
    "printf 'AKULA_B0_SYSTEMD='; if test -d /run/systemd/system; then echo yes; else echo no; fi; ",
    "printf 'AKULA_B0_SUDO='; if sudo -n -l >/dev/null 2>&1; then echo yes; else echo no; fi; ",
    "printf 'AKULA_B0_CPU='; getconf _NPROCESSORS_ONLN; ",
    "printf 'AKULA_B0_RAM='; awk '/^MemTotal:/ { printf \"%d\\n\", $2 / 1024; exit }' /proc/meminfo; ",
    "printf 'AKULA_B0_DISK='; df -Pk / | awk 'NR == 2 { printf \"%d\\n\", $4 / 1024 }'"
);

pub(super) fn decode_ed25519_wire(b64: &str) -> Result<Vec<u8>, B0Error> {
    // Canonical ssh-ed25519 wire blob is exactly 51 bytes = 68 base64 chars,
    // no padding. Keep a tiny, auditable fixed-size decoder.
    if b64.len() != 68 { return Err(B0Error::KeyUnavailable); }
    fn digit(v: u8) -> Option<u32> {
        match v {
            b'A'..=b'Z' => Some((v-b'A') as u32),
            b'a'..=b'z' => Some((v-b'a'+26) as u32),
            b'0'..=b'9' => Some((v-b'0'+52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut wire = Vec::with_capacity(51);
    for q in b64.as_bytes().chunks_exact(4) {
        let [a,b,c,d] = [
            digit(q[0]).ok_or(B0Error::KeyUnavailable)?,
            digit(q[1]).ok_or(B0Error::KeyUnavailable)?,
            digit(q[2]).ok_or(B0Error::KeyUnavailable)?,
            digit(q[3]).ok_or(B0Error::KeyUnavailable)?,
        ];
        wire.extend_from_slice(&[
            ((a<<2)|(b>>4)) as u8,
            ((b<<4)|(c>>2)) as u8,
            ((c<<6)|d) as u8,
        ]);
    }
    if wire.len()!=51 || &wire[..4]!=b"\0\0\0\x0b"
        || &wire[4..15]!=b"ssh-ed25519" || &wire[15..19]!=b"\0\0\0\x20"
    { return Err(B0Error::KeyUnavailable); }
    Ok(wire)
}

pub(super) fn parse_facts(raw: &str) -> Result<ReadOnlyHostFacts, B0Error> {
    if raw.len() > MAX_OUTPUT as usize { return Err(B0Error::InspectionUnavailable); }
    let mut fields = BTreeMap::new();
    for line in raw.lines() {
        let (key,value) = line.split_once('=').ok_or(B0Error::InspectionUnavailable)?;
        if !matches!(key,"AKULA_B0_OS"|"AKULA_B0_ARCH"|"AKULA_B0_SYSTEMD"
            |"AKULA_B0_SUDO"|"AKULA_B0_CPU"|"AKULA_B0_RAM"|"AKULA_B0_DISK")
            || value.len()>100 || fields.insert(key,value).is_some()
        { return Err(B0Error::InspectionUnavailable); }
    }
    if fields.len()!=7 { return Err(B0Error::InspectionUnavailable); }
    let get = |key: &str| fields.get(key).copied().ok_or(B0Error::InspectionUnavailable);
    let raw_os = get("AKULA_B0_OS")?.strip_prefix("ID=")
        .ok_or(B0Error::InspectionUnavailable)?.trim_matches('"');
    let os = match raw_os {
        "ubuntu" => SupportedOs::Ubuntu, "debian" => SupportedOs::Debian,
        _ => SupportedOs::Other,
    };
    let arch = match get("AKULA_B0_ARCH")? {
        "x86_64" => MachineArch::X86_64, "aarch64" => MachineArch::Aarch64,
        _ => MachineArch::Other,
    };
    let yes_no = |s| match s {
        "yes" => Ok(true), "no" => Ok(false),
        _ => Err(B0Error::InspectionUnavailable),
    };
    let number = |key: &str| get(key)?.parse::<u64>()
        .map_err(|_| B0Error::InspectionUnavailable);
    Ok(ReadOnlyHostFacts {
        os, arch,
        has_systemd: yes_no(get("AKULA_B0_SYSTEMD")?)?,
        has_sudo_rights: yes_no(get("AKULA_B0_SUDO")?)?,
        cpu_count: u16::try_from(number("AKULA_B0_CPU")?)
            .map_err(|_| B0Error::InspectionUnavailable)?,
        ram_mib: number("AKULA_B0_RAM")?,
        free_disk_mib: number("AKULA_B0_DISK")?,
    })
}
