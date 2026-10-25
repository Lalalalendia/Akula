//! Read-only proof that the dedicated local IPC account has no usable
//! password. /etc/passwd's "x" alone is NOT evidence of a locked account.
//! Neither password hashes nor account status are accepted from the peer.
use super::*;

const MAX_SHADOW_BYTES: u64 = 1024 * 1024;

fn shadow_mode_allowed(mode: u32) -> bool {
    // Ubuntu/Debian usually use root:shadow 0640; a root-only 0600,
    // 0400 or 0000 file is also supported. Never allow group write,
    // other access, setuid/setgid or an unexpected permission profile.
    matches!(mode, 0o000 | 0o400 | 0o600 | 0o640)
}

fn root_shadow_ok(meta: &Metadata) -> bool {
    meta.is_file()
        && meta.uid() == 0
        && meta.nlink() == 1
        && meta.len() > 0
        && meta.len() <= MAX_SHADOW_BYTES
        && shadow_mode_allowed(meta.permissions().mode() & 0o7777)
}

/// A locked password field starts with ! or *. An empty field, an active
/// crypt hash or a direct plaintext token must NEVER be mistaken for a lock.
/// Multiple service entries, missing entries and malformed records HOLD.
pub(super) fn parse_locked_shadow(data: &[u8]) -> Result<(), PeerEnrollError> {
    let text = std::str::from_utf8(data).map_err(|_| PeerEnrollError::AccountMismatch)?;
    let mut found = false;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() != 9 {
            return Err(PeerEnrollError::AccountMismatch);
        }
        if fields[0] == SERVICE_ACCOUNT {
            if found
                || !matches!(fields[1].as_bytes().first().copied(), Some(b'!' | b'*'))
                || fields[1].contains(char::is_whitespace)
            {
                return Err(PeerEnrollError::AccountMismatch);
            }
            found = true;
        }
    }
    if !found {
        return Err(PeerEnrollError::AccountMismatch);
    }
    Ok(())
}

/// Fixed /etc dirfd only; open with O_NOFOLLOW and O_NONBLOCK. Validate the
/// same inode before and after a size-bounded read; never expose the bytes.
pub(super) fn fixed_shadow_account_locked(etc: &File) -> Result<(), PeerEnrollError> {
    let file = File::from(
        openat(
            etc,
            "shadow",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    let before = file.metadata()?;
    if !root_shadow_ok(&before) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let mut bytes = Vec::new();
    (&file).take(MAX_SHADOW_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != before.len() || !stable(&before, &file.metadata()?) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    parse_locked_shadow(&bytes)
}

#[cfg(test)]
mod tests;
