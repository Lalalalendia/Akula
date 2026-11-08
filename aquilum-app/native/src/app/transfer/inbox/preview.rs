//! User-selected bounded read-only text preview; never render unverified bytes.
use super::*;
use std::io::Read;

/// A deliberately narrow in-app text preview: only checked files up to 4 KiB.
/// Never execute remote bytes, launch handlers, use a sender path or write Vault.
pub(super) fn read_checked_text_preview(root: &Path, id: Uuid) -> Result<Option<String>, String> {
    const MAX_TEXT_BYTES: u64 = 4 * 1024;
    #[cfg(windows)]
    let _pinned = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(root)
        .map_err(|error| format!("Windows Inbox read custody refused: {error:?}"))?;
    let ledger = InboxLedger::open_existing(root)
        .map_err(|error| format!("open checked Inbox: {error:?}"))?
        .ok_or("Inbox does not exist")?;
    let (mut pinned, intent) = ledger
        .open_verified_file(id)
        .map_err(|error| format!("checked preview refused: {error:?}"))?;
    if intent.bytes > MAX_TEXT_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(intent.bytes as usize);
    (&mut pinned)
        .take(MAX_TEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 != intent.bytes
        || pinned.metadata().map_err(|error| error.to_string())?.len() != intent.bytes
        || *blake3::hash(&bytes).as_bytes() != intent.blake3
    {
        return Err("Inbox file changed during preview".into());
    }
    if bytes.contains(&0) {
        return Ok(None);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Ok(None);
    }
    Ok(Some(text))
}
