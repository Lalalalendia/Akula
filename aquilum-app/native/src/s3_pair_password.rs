//! Masked OS-owned pairing password prompt.
//! Never insert pairing passwords into Masonry TextEditor, settings JSON,
//! clipboard or tracing. The returned string is zeroized on drop.
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordPromptError {
    Unsupported,
    Unavailable,
    Invalid,
}

#[cfg(target_os = "windows")]
fn prompt(title: &str, details: &str) -> Result<Option<Zeroizing<String>>, PasswordPromptError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_CANCELLED, ERROR_SUCCESS};
    use windows::Win32::Security::Credentials::{
        CredUIPromptForCredentialsW, CREDUI_INFOW,
        CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
        CREDUI_FLAGS_GENERIC_CREDENTIALS,
    };
    let wide = |v: &str| v.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let caption = wide(title);
    let details = wide(details);
    let target = wide("Akula S3 Pairing / This File Only");
    let info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        pszCaptionText: PCWSTR(caption.as_ptr()),
        pszMessageText: PCWSTR(details.as_ptr()),
        ..Default::default()
    };
    let mut user = Zeroizing::new(vec![0u16; 256]);
    let mut secret = Zeroizing::new(vec![0u16; 1024]);
    for (dst, src) in user.iter_mut().zip("Akula".encode_utf16()) { *dst = src; }
    let rc = unsafe {
        CredUIPromptForCredentialsW(
            Some(&info), PCWSTR(target.as_ptr()), None, 0,
            &mut user[..], &mut secret[..], None,
            CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_ALWAYS_SHOW_UI
                | CREDUI_FLAGS_DO_NOT_PERSIST,
        )
    };
    if rc == ERROR_CANCELLED { return Ok(None); }
    if rc != ERROR_SUCCESS { return Err(PasswordPromptError::Unavailable); }
    let length = secret.iter().position(|c| *c == 0)
        .unwrap_or(secret.len());
    if !(20..=1023).contains(&length) {
        return Err(PasswordPromptError::Invalid);
    }
    let answer = String::from_utf16(&secret[..length])
        .map_err(|_| PasswordPromptError::Invalid)?;
    if answer.as_bytes().len() > 1024 || answer.contains('\0') {
        return Err(PasswordPromptError::Invalid);
    }
    Ok(Some(Zeroizing::new(answer)))
}

#[cfg(not(target_os = "windows"))]
fn prompt(_title: &str, _details: &str)
    -> Result<Option<Zeroizing<String>>, PasswordPromptError>
{
    Err(PasswordPromptError::Unsupported)
}

pub fn ask_import_password() -> Result<Option<Zeroizing<String>>, PasswordPromptError> {
    prompt("Akula S3: Import", "Enter the password for the encrypted .akula-sync file.")
}

pub fn ask_export_password() -> Result<Option<Zeroizing<String>>, PasswordPromptError> {
    let Some(first) = prompt(
        "Akula S3: Share settings",
        "Use a unique passphrase of at least 20 characters. Share it separately.",
    )? else { return Ok(None); };
    let Some(second) = prompt(
        "Akula S3: Confirm password",
        "Enter the same password again to protect the .akula-sync file.",
    )? else { return Ok(None); };
    if first.as_str() != second.as_str() {
        return Err(PasswordPromptError::Invalid);
    }
    Ok(Some(first))
}
