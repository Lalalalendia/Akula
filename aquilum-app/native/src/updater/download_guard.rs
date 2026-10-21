//! Fail-closed size budget for signed updater downloads.
//! This is not a substitute for verifying the exact Akula minisign key.
use std::io::Read;

pub(super) const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;
const IO_BLOCK: usize = 64 * 1024;
const RESERVE_STEP: usize = 4 * 1024 * 1024;

/// Do not accept malformed, zero or over-budget HTTP Content-Length. A
/// missing header is okay, but it does not relax the real stream budget.
pub(super) fn content_length(raw: Option<&str>) -> Result<Option<u64>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let size = raw.parse::<u64>().map_err(|_| "invalid updater Content-Length")?;
    if size == 0 || size > MAX_UPDATE_BYTES {
        return Err("updater Content-Length exceeds the approved package budget".into());
    }
    Ok(Some(size))
}

/// Enforce the *signed* artifact size as the stream budget, not merely the
/// legacy 512 MiB ceiling. An untrusted HTTP Content-Length cannot override it.
pub(super) fn read_signed_payload(
    reader: impl Read,
    total: Option<u64>,
    signed_size: u64,
    on_progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    if signed_size == 0 || signed_size > MAX_UPDATE_BYTES {
        return Err("signed updater payload size is outside the approved budget".into());
    }
    if let Some(reported) = total
        && reported != signed_size
    {
        return Err("updater Content-Length conflicts with signed artifact size".into());
    }
    let payload = read_with_limit(reader, total, signed_size, on_progress)?;
    if payload.len() as u64 != signed_size {
        return Err("downloaded updater payload differs from signed artifact size".into());
    }
    Ok(payload)
}

fn read_with_limit(
    mut reader: impl Read,
    total: Option<u64>,
    maximum: u64,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    if maximum == 0 || maximum > usize::MAX as u64 {
        return Err("unsupported update memory budget".into());
    }
    if let Some(reported) = total
        && (reported == 0 || reported > maximum)
    {
        return Err("reported updater payload exceeds approved size".into());
    }
    let mut data = Vec::new();
    let mut block = [0u8; IO_BLOCK];
    loop {
        let len = reader.read(&mut block).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        let current = data.len() as u64;
        if len as u64 > maximum - current {
            return Err("updater payload stream exceeded approved size limit".into());
        }
        // Reserve only a bounded next increment. Never preallocate the entire
        // untrusted Content-Length (or allow geometric overshoot beyond limit).
        if data.capacity() - data.len() < len {
            let desired = usize::min(RESERVE_STEP, (maximum - current) as usize);
            data.try_reserve_exact(desired)
                .map_err(|_| "unable to reserve bounded updater package buffer")?;
        }
        data.extend_from_slice(&block[..len]);
        on_progress(data.len() as u64, total);
    }
    if data.is_empty() {
        return Err("empty updater package rejected".into());
    }
    if let Some(expected) = total
        && data.len() as u64 != expected
    {
        return Err("updater HTTP Content-Length does not match received bytes".into());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn signed_size_is_a_strict_stream_budget_even_without_http_length() {
        let bytes = Cursor::new(vec![0x42u8; 20]);
        assert_eq!(
            read_signed_payload(bytes, None, 20, |_, _| {}).unwrap().len(),
            20,
        );
        assert!(read_signed_payload(
            Cursor::new(vec![0x42u8; 21]), None, 20, |_, _| {},
        ).is_err(), "a dishonest signed-sized stream must fail");
        assert!(read_signed_payload(
            Cursor::new(vec![0x42u8; 19]), None, 20, |_, _| {},
        ).is_err(), "truncated package without Content-Length must fail");
        assert!(read_signed_payload(
            Cursor::new(vec![0x42u8; 20]), Some(21), 20, |_, _| {},
        ).unwrap_err().contains("conflicts"));
        assert!(read_signed_payload(
            Cursor::new(vec![0x42u8; 20]), None, MAX_UPDATE_BYTES + 1, |_, _| {},
        ).is_err());
    }

    #[test]
    fn exact_size_and_progress_are_preserved() {
        let mut states = Vec::new();
        let payload = read_with_limit(
            Cursor::new(vec![7u8; 96]),
            Some(96),
            96,
            |done, total| states.push((done, total)),
        ).unwrap();
        assert_eq!(payload, vec![7u8; 96]);
        assert_eq!(states.last(), Some(&(96, Some(96))));
    }

    #[test]
    fn absent_content_length_still_enforces_cap() {
        assert!(read_with_limit(Cursor::new(vec![9u8; 97]), None, 96, |_, _| {})
            .unwrap_err().contains("exceeded"));
    }

    #[test]
    fn declared_length_over_budget_fails_before_reading() {
        struct MustNotRead;
        impl Read for MustNotRead {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("declared over-budget package caused a network body read")
            }
        }
        assert!(read_with_limit(MustNotRead, Some(97), 96, |_, _| {}).is_err());
    }

    #[test]
    fn dishonest_short_header_is_not_allowed_to_bypass_stream_limit() {
        assert!(read_with_limit(Cursor::new(vec![1u8; 97]), Some(8), 96, |_, _| {})
            .unwrap_err().contains("exceeded"));
    }

    #[test]
    fn truncated_http_body_is_rejected() {
        assert!(read_with_limit(Cursor::new(vec![1u8; 8]), Some(9), 96, |_, _| {})
            .unwrap_err().contains("does not match"));
    }

    #[test]
    fn unexpected_early_eof_without_header_is_valid_only_if_nonempty() {
        assert_eq!(
            read_with_limit(Cursor::new(b"ok"), None, 96, |_, _| {}).unwrap(),
            b"ok"
        );
        assert!(read_with_limit(Cursor::new(Vec::<u8>::new()), None, 96, |_, _| {})
            .unwrap_err().contains("empty"));
    }

    #[test]
    fn content_length_parser_rejects_invalid_and_zero_headers() {
        assert_eq!(content_length(None).unwrap(), None);
        assert_eq!(content_length(Some("1024")).unwrap(), Some(1024));
        assert!(content_length(Some("0")).is_err());
        assert!(content_length(Some("NaN")).is_err());
        assert!(content_length(Some("42, 42")).is_err());
        assert!(content_length(Some("-1")).is_err());
        assert!(content_length(Some("18446744073709551616")).is_err());
    }

    #[test]
    fn oversized_content_length_is_refused() {
        assert!(content_length(Some(&(MAX_UPDATE_BYTES + 1).to_string())).is_err());
        assert_eq!(content_length(Some(&MAX_UPDATE_BYTES.to_string())).unwrap(), Some(MAX_UPDATE_BYTES));
    }

    #[test]
    fn tiny_budget_and_zero_budget_never_overflow() {
        assert!(read_with_limit(Cursor::new(b"x"), None, 0, |_, _| {}).is_err());
        assert_eq!(
            read_with_limit(Cursor::new(b"x"), Some(1), 1, |_, _| {}).unwrap(),
            b"x"
        );
    }

    #[test]
    fn progress_never_exceeds_limit() {
        let mut last = 0;
        let result = read_with_limit(Cursor::new(vec![2u8; 200]), Some(200), 64, |n, _| {
            assert!(n <= 64);
            last = n;
        });
        assert!(result.is_err());
        assert_eq!(last, 0);
    }
}
