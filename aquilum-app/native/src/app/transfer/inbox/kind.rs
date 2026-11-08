//! QA-only *informational* type hint from verified local Inbox bytes.
//! A header is not a trusted MIME type, original filename or permission to open.
//! Never execute content, load system handlers or copy sender-supplied paths.
use super::*;
use std::io::Read;

const MAX_PROBE: u64 = 4 * 1024;
const UNKNOWN: &str = "transfer.inboxKindUnknown";

fn describe(prefix: &[u8], complete: bool) -> &'static str {
    if prefix.starts_with(b"%PDF-") {
        return "transfer.inboxKindPdf";
    }
    if prefix.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "transfer.inboxKindPng";
    }
    if prefix.starts_with(b"\xff\xd8\xff") {
        return "transfer.inboxKindJpeg";
    }
    if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
        return "transfer.inboxKindGif";
    }
    if prefix.starts_with(b"PK\x03\x04") || prefix.starts_with(b"PK\x05\x06")
        || prefix.starts_with(b"PK\x07\x08")
    {
        return "transfer.inboxKindZip";
    }
    // Large text cannot be inferred from a sample alone. Avoid misleading
    // users about the entire payload. No text bytes are exposed in the list.
    if complete
        && !prefix.is_empty()
        && std::str::from_utf8(prefix)
            .is_ok_and(|value| value.chars().all(|ch| {
                !ch.is_control() || matches!(ch, '\n' | '\r' | '\t')
            }))
    {
        return "transfer.inboxKindText";
    }
    UNKNOWN
}

/// Reverify the SAME opened local file before exposing even a type hint.
/// Failures are handled by the caller as NeedsReview, never Verified.
pub(super) fn checked_kind(ledger: &InboxLedger, id: Uuid) -> Result<&'static str, String> {
    let (mut file, expected) = ledger
        .open_verified_file(id)
        .map_err(|error| format!("Inbox type recheck refused: {error:?}"))?;
    let probe_size = expected.bytes.min(MAX_PROBE) as usize;
    let mut prefix = Vec::with_capacity(probe_size);
    (&mut file)
        .take(MAX_PROBE)
        .read_to_end(&mut prefix)
        .map_err(|error| format!("Inbox type recheck read: {error:?}"))?;
    if prefix.len() != probe_size
        || file.metadata().map_err(|error| format!("Inbox type metadata: {error:?}"))?.len()
            != expected.bytes
    {
        return Err("Inbox object changed during type inspection".into());
    }
    Ok(describe(&prefix, expected.bytes <= MAX_PROBE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_local_magic_or_complete_utf8_is_labeled() {
        assert_eq!(describe(b"%PDF-1.7\n", true), "transfer.inboxKindPdf");
        assert_eq!(describe(b"\x89PNG\r\n\x1a\n", true), "transfer.inboxKindPng");
        assert_eq!(describe(b"\xff\xd8\xff\xe0", true), "transfer.inboxKindJpeg");
        assert_eq!(describe(b"GIF89a", true), "transfer.inboxKindGif");
        assert_eq!(describe(b"PK\x03\x04", true), "transfer.inboxKindZip");
        assert_eq!(describe("Привет\n".as_bytes(), true), "transfer.inboxKindText");
        assert_eq!(describe(b"simple text", false), UNKNOWN);
        assert_eq!(describe(b"binary\x00text", true), UNKNOWN);
        assert_eq!(describe(b"\xff\xf0", true), UNKNOWN);
        assert_eq!(describe(b"", true), UNKNOWN);
        assert_eq!(describe(b"spoofed.pdf without PDF signature\x00", true), UNKNOWN);
    }
}
