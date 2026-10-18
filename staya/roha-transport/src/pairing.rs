//! Explicit, out-of-band exchange of public transport identities.
//!
//! A contact file is NOT a signed invitation, PAKE pairing, membership grant,
//! or authorization to access files. The owner must verify the source using an
//! independently trusted channel before promoting a contact to "trusted peer".

use iroh::EndpointId;
use std::str::FromStr;

const CONTACT_MAGIC: &str = "AKULA_STAYA_CONTACT_V1";
const TRUSTED_MAGIC: &str = "AKULA_STAYA_TRUSTED_PEER_V1";
pub const MAX_RECORD_BYTES: usize = 160;

fn serialize_record(marker: &str, id: EndpointId) -> String {
    format!("{marker}\n{id}\n")
}

fn parse_record(text: &str, marker: &str) -> Result<EndpointId, String> {
    if text.len() > MAX_RECORD_BYTES {
        return Err("identity record is too large".into());
    }
    let mut lines = text.split('\n');
    if lines.next() != Some(marker) {
        return Err("identity record has a wrong format/version".into());
    }
    let raw = lines.next().ok_or("missing endpoint identity")?;
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("identity must be exactly 64 lowercase hexadecimal characters".into());
    }
    if lines.next() != Some("") || lines.next().is_some() {
        return Err("unexpected content after endpoint identity".into());
    }
    EndpointId::from_str(raw).map_err(|e| format!("invalid endpoint identity: {e}"))
}

/// This file may be freely copied, but its provenance MUST be verified by an
/// independent trusted channel before any enrollment.
pub fn export_contact(id: EndpointId) -> String {
    serialize_record(CONTACT_MAGIC, id)
}

pub fn parse_contact(text: &str) -> Result<EndpointId, String> {
    parse_record(text, CONTACT_MAGIC)
}

/// An explicit owner's decision to trust an EndpointId at transport level.
/// NOT a permission to read or write files or join any vault.
pub fn export_trusted_peer(id: EndpointId) -> String {
    serialize_record(TRUSTED_MAGIC, id)
}

pub fn parse_trusted_peer(text: &str) -> Result<EndpointId, String> {
    parse_record(text, TRUSTED_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_contact_roundtrip_and_explicit_promotion() {
        let id = iroh::SecretKey::generate().public();
        let contact = export_contact(id);
        assert_eq!(parse_contact(&contact).unwrap(), id);
        assert!(parse_trusted_peer(&contact).is_err());
        let trusted = export_trusted_peer(parse_contact(&contact).unwrap());
        assert_eq!(parse_trusted_peer(&trusted).unwrap(), id);
        assert!(parse_contact(&trusted).is_err());
    }

    #[test]
    fn malformed_or_unsupported_contact_never_enrolls() {
        let id = iroh::SecretKey::generate().public();
        let valid = export_contact(id);
        assert!(parse_contact(&valid.replace("CONTACT_V1", "CONTACT_V2")).is_err());
        assert!(parse_contact(&valid.replace(&id.to_string(), "BAD")).is_err());
        assert!(parse_contact(&valid.replace(&id.to_string(), &id.to_z32())).is_err());
        assert!(parse_contact(&valid.to_uppercase()).is_err());
        assert!(parse_contact(&format!("{valid}garbage")).is_err());
        assert!(parse_contact(&format!("{valid}\n")).is_err());
        assert!(parse_contact(&format!("{}junk", "x".repeat(MAX_RECORD_BYTES))).is_err());
        assert!(parse_contact(&valid.replace('\n', "\r\n")).is_err());
        assert!(parse_contact("").is_err());
    }
}
