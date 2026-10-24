//! Windows Credential Manager for a user-approved, root-scoped Notion token.
//! Never keep tokens in Settings, SQLite, a Vault, UI text widgets or logs.
//! Unsupported OSes fail closed. This is a QA-only native feature.
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveOutcome { Saved(String), Cancelled }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialError { InvalidRoot, InvalidToken, Missing, Unavailable, Unsupported }

fn canonical_root(root: &str) -> Result<String, CredentialError> {
    let canonical = Uuid::parse_str(root)
        .map_err(|_| CredentialError::InvalidRoot)?.to_string();
    if canonical != root { return Err(CredentialError::InvalidRoot); }
    Ok(canonical)
}
fn target(root: &str) -> Result<String, CredentialError> {
    Ok(format!("Akula/NotionRead/v1/{}", canonical_root(root)?))
}

pub fn prompt_and_store() -> Result<SaveOutcome, CredentialError> {
    #[cfg(target_os = "windows")]
    { win::prompt_and_store() }
    #[cfg(not(target_os = "windows"))]
    { Err(CredentialError::Unsupported) }
}

pub fn with_token<R>(
    root: &str,
    callback: impl FnOnce(&str) -> R,
) -> Result<R, CredentialError> {
    canonical_root(root)?;
    #[cfg(target_os = "windows")]
    { win::with_token(root, callback) }
    #[cfg(not(target_os = "windows"))]
    { let _ = callback; Err(CredentialError::Unsupported) }
}

pub fn remove(root: &str) -> Result<(), CredentialError> {
    canonical_root(root)?;
    #[cfg(target_os = "windows")]
    { win::remove(root) }
    #[cfg(not(target_os = "windows"))]
    { Err(CredentialError::Unsupported) }
}

#[cfg(target_os = "windows")]
mod win {
    use super::{canonical_root, target, CredentialError, SaveOutcome};
    use std::ffi::c_void;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{GetLastError, ERROR_CANCELLED, ERROR_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredUIPromptForCredentialsW, CredWriteW,
        CREDENTIALW, CREDUI_INFOW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_GENERIC_CREDENTIALS,
        CREDUI_FLAGS_DO_NOT_PERSIST,
    };
    use zeroize::Zeroizing;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn term_len(buffer: &[u16]) -> usize {
        buffer.iter().position(|x| *x == 0).unwrap_or(buffer.len())
    }
    struct Cred(*mut CREDENTIALW);
    impl Drop for Cred {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CredFree(self.0.cast::<c_void>()) };
            }
        }
    }

    /// CredUI is a real masked Windows system dialog. The "user" field
    /// identifies the requested root UUID, the password is the Notion token.
    /// The actual stored credential is strictly per root, not per workspace.
    pub(super) fn prompt_and_store() -> Result<SaveOutcome, CredentialError> {
        let title = wide("Akula — Notion (только чтение)");
        let description = wide("Логин: UUID корневой страницы Notion. Пароль: токен интеграции. Разрешите интеграции доступ к этой странице в Notion.");
        let prompt_target = wide("Akula/NotionRead/v1");
        let info = CREDUI_INFOW {
            cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
            pszCaptionText: PCWSTR(title.as_ptr()),
            pszMessageText: PCWSTR(description.as_ptr()),
            ..Default::default()
        };
        let mut user = Zeroizing::new(vec![0_u16; 256]);
        let mut pass = Zeroizing::new(vec![0_u16; 256]);
        let result = unsafe {
            CredUIPromptForCredentialsW(
                Some(&info), PCWSTR(prompt_target.as_ptr()), None, 0,
                &mut user[..], &mut pass[..], None,
                CREDUI_FLAGS_GENERIC_CREDENTIALS
                    | CREDUI_FLAGS_ALWAYS_SHOW_UI
                    | CREDUI_FLAGS_DO_NOT_PERSIST,
            )
        };
        if result == ERROR_CANCELLED { return Ok(SaveOutcome::Cancelled); }
        if result != ERROR_SUCCESS { return Err(CredentialError::Unavailable); }
        let user_len = term_len(&user);
        let pass_len = term_len(&pass);
        if user_len != 36 || !(8..=255).contains(&pass_len) {
            return Err(CredentialError::InvalidToken);
        }
        let id = String::from_utf16(&user[..user_len])
            .map_err(|_| CredentialError::InvalidRoot)?;
        let root = canonical_root(&id)?;
        let target = target(&root)?;
        if pass[..pass_len].iter().any(|c| *c == 0 || *c > 127
            || !(*c as u8).is_ascii_graphic())
        {
            return Err(CredentialError::InvalidToken);
        }
        let secret = Zeroizing::new(
            String::from_utf16(&pass[..pass_len])
                .map_err(|_| CredentialError::InvalidToken)?
        );
        let mut blob = Zeroizing::new(secret.as_bytes().to_vec());
        let mut target_wide = wide(&target);
        let mut root_wide = wide(&root);
        let record = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_wide.as_mut_ptr()),
            UserName: PWSTR(root_wide.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        unsafe { CredWriteW(&record, 0) }
            .map_err(|_| CredentialError::Unavailable)?;
        Ok(SaveOutcome::Saved(root))
    }

    pub(super) fn with_token<R>(
        root: &str,
        action: impl FnOnce(&str) -> R,
    ) -> Result<R, CredentialError> {
        let id = target(root)?;
        let key = wide(&id);
        let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
        if unsafe { CredReadW(PCWSTR(key.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) }.is_err() {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Err(CredentialError::Missing)
            } else { Err(CredentialError::Unavailable) };
        }
        let owned = Cred(raw);
        if raw.is_null() { return Err(CredentialError::Unavailable); }
        let record = unsafe { &*owned.0 };
        if record.CredentialBlob.is_null() || !(8..=255).contains(&(record.CredentialBlobSize as usize)) {
            return Err(CredentialError::InvalidToken);
        }
        // Do not trust OS-owned bytes blindly even if Credential Manager is
        // writable only by the current user.
        let bytes = Zeroizing::new(unsafe {
            std::slice::from_raw_parts(record.CredentialBlob, record.CredentialBlobSize as usize).to_vec()
        });
        if !bytes.iter().all(u8::is_ascii_graphic) {
            return Err(CredentialError::InvalidToken);
        }
        let token = Zeroizing::new(
            String::from_utf8(bytes.to_vec()).map_err(|_| CredentialError::InvalidToken)?
        );
        drop(owned);
        Ok(action(&token))
    }

    #[cfg(test)]
    #[test]
    fn disposable_windows_credential_roundtrip_is_root_scoped_and_removable() {
        // This test creates only a synthetic credential for its unique UUID.
        // It never invokes CredUI, Notion API, or a user's actual account.
        let a = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let credential_name = target(&a).unwrap();
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = remove(&self.0); }
        }
        let _cleanup = Cleanup(a.clone());
        let mut target_wide = wide(&credential_name);
        let mut root_wide = wide(&a);
        let mut token = Zeroizing::new(
            format!("synthetic-notion-{}", uuid::Uuid::new_v4()).into_bytes()
        );
        let record = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_wide.as_mut_ptr()),
            UserName: PWSTR(root_wide.as_mut_ptr()),
            CredentialBlobSize: token.len() as u32,
            CredentialBlob: token.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        unsafe { CredWriteW(&record, 0) }.unwrap();
        assert_eq!(super::with_token(&a, |x| x.as_bytes() == token.as_slice()), Ok(true));
        assert_eq!(super::with_token(&other, |_| ()), Err(CredentialError::Missing));
        remove(&a).unwrap();
        assert_eq!(super::with_token(&a, |_| ()), Err(CredentialError::Missing));
    }

    pub(super) fn remove(root: &str) -> Result<(), CredentialError> {
        let id = wide(&target(root)?);
        match unsafe { CredDeleteW(PCWSTR(id.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(_) if unsafe { GetLastError() } == ERROR_NOT_FOUND => Ok(()),
            Err(_) => Err(CredentialError::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_are_isolated_by_canonical_root_uuid() {
        let a="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let b="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        assert_ne!(target(a).unwrap(), target(b).unwrap());
        for bad in ["", "notion", "https://notion.so", "../other", "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"] {
            assert!(target(bad).is_err());
        }
    }
}
