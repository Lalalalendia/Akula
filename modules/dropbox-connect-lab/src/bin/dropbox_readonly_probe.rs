#![forbid(unsafe_code)]
//! Explicit native read-only Dropbox QA probe. Never runs in normal Akula.
//! No cloud writes, file mutations, fallback secrets, or auto-sync.

use akula_dropbox_connect_lab::{ConnectSession, ConnectionStatus};
use akula_dropbox_secret_store::CredentialScope;
use akula_dropbox_t0::{read_verify::TrustedBlake3, Scope};
use std::{
    env,
    process::{Command, ExitCode, Stdio},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseError { Missing, Duplicate, Unknown, Invalid }

struct Options {
    scope: Scope,
    oauth_app_key: String,
    registered_port: u16,
    head_pin: TrustedBlake3,
    dry_run: bool,
}

#[derive(Default)]
struct RawOptions {
    app_key: Option<String>,
    port: Option<String>,
    account_id: Option<String>,
    root_namespace: Option<String>,
    vault_id: Option<String>,
    profile_id: Option<String>,
    trusted_head: Option<String>,
    dry_run: bool,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I)
    -> Result<Options, ParseError>
{
    let mut raw = RawOptions::default();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        if flag == "--dry-run" {
            if raw.dry_run { return Err(ParseError::Duplicate); }
            raw.dry_run = true;
            continue;
        }
        let slot = match flag.as_str() {
            "--oauth-app-key" => &mut raw.app_key,
            "--registered-port" => &mut raw.port,
            "--account-id" => &mut raw.account_id,
            "--root-namespace-id" => &mut raw.root_namespace,
            "--vault-id" => &mut raw.vault_id,
            "--profile-id" => &mut raw.profile_id,
            "--trusted-head-blake3" => &mut raw.trusted_head,
            _ => return Err(ParseError::Unknown),
        };
        if slot.is_some() { return Err(ParseError::Duplicate); }
        let value = args.next().ok_or(ParseError::Missing)?;
        if value.is_empty() || value.starts_with("--") {
            return Err(ParseError::Invalid);
        }
        *slot = Some(value);
    }
    let app_key = raw.app_key.ok_or(ParseError::Missing)?;
    let account = raw.account_id.ok_or(ParseError::Missing)?;
    let root = raw.root_namespace.ok_or(ParseError::Missing)?;
    let vault = raw.vault_id.ok_or(ParseError::Missing)?;
    let profile = raw.profile_id.ok_or(ParseError::Missing)?;
    let port: u16 = raw.port.ok_or(ParseError::Missing)?.parse()
        .map_err(|_| ParseError::Invalid)?;
    if port == 0 { return Err(ParseError::Invalid); }
    let head_pin = TrustedBlake3::parse(
        &raw.trusted_head.ok_or(ParseError::Missing)?,
    ).map_err(|_| ParseError::Invalid)?;
    let scope = Scope::new(&account, &root, &vault, &profile)
        .map_err(|_| ParseError::Invalid)?;
    CredentialScope::new(&account, &root, &vault, &profile, &app_key)
        .map_err(|_| ParseError::Invalid)?;
    Ok(Options {
        scope,
        oauth_app_key: app_key,
        registered_port: port,
        head_pin,
        dry_run: raw.dry_run,
    })
}

const USAGE: &str = "\
Offline preflight: add --dry-run. Real mode opens the SYSTEM browser.
Required (non-secret) arguments:
  --oauth-app-key APP_KEY
  --registered-port PORT
  --account-id dbid:ACCOUNT
  --root-namespace-id NAMESPACE_ID
  --vault-id LOWERCASE_UUID
  --profile-id PROFILE
  --trusted-head-blake3 OUT_OF_BAND_BLAKE3
Important: register http://127.0.0.1:PORT/dropbox/callback EXACTLY
in the Dropbox App Console, with App Folder and read-only scopes.
Obtain the expected head digest from an independent trusted source.
Do NOT place OAuth authorization codes, access/refresh tokens, or client
secrets on the command line. No cloud writes or local note changes occur.
";

#[cfg(target_os = "windows")]
fn open_system_browser(url: &str) -> bool {
    // Explicit argv without cmd.exe: URL ampersands cannot be shell code.
    Command::new("explorer.exe")
        .arg(url).stdout(Stdio::null()).stderr(Stdio::null())
        .status().is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
fn open_system_browser(url: &str) -> bool {
    Command::new("open")
        .arg(url).stdout(Stdio::null()).stderr(Stdio::null())
        .status().is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn open_system_browser(url: &str) -> bool {
    Command::new("xdg-open")
        .arg(url).stdout(Stdio::null()).stderr(Stdio::null())
        .status().is_ok_and(|status| status.success())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn open_system_browser(_url: &str) -> bool { false }

fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let opts = match parse_args(args) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("INVALID_PREFLIGHT_CONFIG. No network or credentials used.\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    if opts.dry_run {
        println!("CONFIG_VALID_OFFLINE. No browser, network, credential or note access.");
        return ExitCode::SUCCESS;
    }
    let session = match ConnectSession::begin(
        opts.scope, &opts.oauth_app_key,
        opts.registered_port, opts.head_pin,
    ) {
        Ok(session) => session,
        Err(_) => {
            eprintln!("LOGIN_BIND_REJECTED. Check exact registered loopback port.");
            return ExitCode::FAILURE;
        }
    };
    if !open_system_browser(session.browser_authorize_url()) {
        eprintln!("BROWSER_LAUNCH_FAILED. Login cancelled; URL not logged.");
        return ExitCode::FAILURE;
    }
    println!("Waiting for consent in your system browser (max 120 seconds).");
    match session.finish(Duration::from_secs(120)) {
        Ok(candidate) => {
            if candidate.status() != ConnectionStatus::RemotePinnedNeedsPublisherApproval {
                eprintln!("UNEXPECTED_STATE. Synchronization remains disabled.");
                return ExitCode::FAILURE;
            }
            // Never print the root JSON, OAuth URL/state, token or account id.
            println!("REMOTE_PINNED_READ_OK. Refresh token stored in OS credential vault.");
            println!("PUBLISHER_NOT_APPROVED. No document changes. SYNC_DISABLED.");
            ExitCode::SUCCESS
        }
        Err(_) => {
            eprintln!("READ_ONLY_CONNECT_REJECTED. No document changes. SYNC_DISABLED.");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    if env::args().nth(1).as_deref() == Some("--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    // Read argv once; do not emit arguments to stdout/stderr or diagnostics.
    run(env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_VAULT: &str = "12345678-1234-1234-1234-123456789abc";
    fn basic_args() -> Vec<String> {
        let pairs = [
            ("--oauth-app-key", "app_key_123"),
            ("--registered-port", "47219"),
            ("--account-id", "dbid:alice"),
            ("--root-namespace-id", "12345"),
            ("--vault-id", VALID_VAULT),
            ("--profile-id", "notes"),
            ("--trusted-head-blake3",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        ];
        pairs.into_iter().flat_map(|(a, b)| [a.to_string(), b.to_string()]).collect()
    }

    #[test]
    fn config_preflight_does_not_use_network_or_credentials() {
        let mut args = basic_args();
        args.push("--dry-run".into());
        let parsed = parse_args(args).unwrap();
        assert!(parsed.dry_run);
        assert_eq!(parsed.registered_port, 47219);
        assert_eq!(parsed.scope.vault_id(), VALID_VAULT);
    }

    #[test]
    fn missing_port_and_zero_port_are_refused() {
        let mut args = basic_args();
        let pos = args.iter().position(|x| x == "--registered-port").unwrap();
        args.drain(pos..pos + 2);
        assert!(matches!(parse_args(args), Err(ParseError::Missing)));
        let mut args = basic_args();
        let pos = args.iter().position(|x| x == "--registered-port").unwrap();
        args[pos + 1] = "0".into();
        assert!(matches!(parse_args(args), Err(ParseError::Invalid)));
    }

    #[test]
    fn untrusted_or_missing_digest_is_refused() {
        let mut args = basic_args();
        let pos = args.iter().position(|x| x == "--trusted-head-blake3").unwrap();
        args[pos + 1] = "not-a-hash".into();
        assert!(matches!(parse_args(args), Err(ParseError::Invalid)));
        let mut args = basic_args();
        let pos = args.iter().position(|x| x == "--trusted-head-blake3").unwrap();
        args.drain(pos..pos + 2);
        assert!(matches!(parse_args(args), Err(ParseError::Missing)));
    }

    #[test]
    fn no_token_or_secret_arguments_are_accepted() {
        for forbidden in ["--access-token", "--refresh-token", "--client-secret"] {
            let mut args = basic_args();
            args.push(forbidden.into());
            args.push("secret".into());
            assert!(matches!(parse_args(args), Err(ParseError::Unknown)));
        }
    }

    #[test]
    fn wrong_app_key_or_namespace_is_rejected() {
        let mut args = basic_args();
        let pos = args.iter().position(|x| x == "--oauth-app-key").unwrap();
        args[pos + 1] = "../other-app".into();
        assert!(matches!(parse_args(args), Err(ParseError::Invalid)));
        let mut args = basic_args();
        let pos = args.iter().position(|x| x == "--root-namespace-id").unwrap();
        args[pos + 1] = "../other".into();
        assert!(matches!(parse_args(args), Err(ParseError::Invalid)));
    }

    #[test]
    fn repeated_flags_are_rejected() {
        let mut args = basic_args();
        args.extend(["--oauth-app-key".into(), "another-app".into()]);
        assert!(matches!(parse_args(args), Err(ParseError::Duplicate)));
        let mut args = basic_args();
        args.extend(["--dry-run".into(), "--dry-run".into()]);
        assert!(matches!(parse_args(args), Err(ParseError::Duplicate)));
    }
}
