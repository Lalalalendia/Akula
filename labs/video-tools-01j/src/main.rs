//! VIDEO-TOOLS-01j: real native Akula crypto/discovery/held-handle acceptance.
//! A standalone PR-only WITNESS: never installed, never launches a Video Tool.
//! CI's ephemeral public key is used only here; production keys stay unapproved.
#![allow(dead_code)]
#[path = "../../../aquilum-app/native/src/video/tools_package.rs"]
mod tools_package;
#[path = "../../../aquilum-app/native/src/video/tools_package_probe.rs"]
mod tools_package_probe;
#[path = "../../../aquilum-app/native/src/video/tools_package_pin_windows.rs"]
mod tools_package_pin_windows;
#[path = "../../../aquilum-app/native/src/video/tools_package_pin_admission.rs"]
mod tools_package_pin_admission;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};
use tools_package_probe::{probe_installed_video_tools, ToolsDiscovery};
use tools_package_probe::windows_discovery as os_root;

fn hold(msg: &str) -> String { format!("VIDEO_TOOLS_01J_HOLD: {msg}") }
fn inside(path: &Path, root: &Path) -> Result<bool, String> {
    let object = fs::canonicalize(path).map_err(|_| hold("CI key file unavailable"))?;
    let base = fs::canonicalize(root).map_err(|_| hold("runner temp unavailable"))?;
    Ok(object.starts_with(&base) && object != base)
}
fn sha_of_held(file: &mut fs::File) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|_| hold("pinned PE unreadable"))?;
        if n == 0 { break; }
        count = count.checked_add(n as u64).ok_or_else(|| hold("PE byte count overflow"))?;
        if count > 128 * 1024 * 1024 { return Err(hold("PE oversized")); }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn exact_installed(root: &Path, key: &str) -> Result<String, String> {
    os_root::check_installed_acl(root, &tools_package_pin_windows::SIGNED_NAMES)?;
    let mut held = tools_package_pin_windows::PinnedPackage::open(
        root, &tools_package_pin_windows::SIGNED_NAMES,
    )?;
    let raw = tools_package::bounded_metadata(&root.join("manifest.json"),
        tools_package::MAX_MANIFEST)?;
    let signature = tools_package::bounded_metadata(&root.join("manifest.minisig"),
        tools_package::MAX_SIGNATURE)?;
    let signed = std::str::from_utf8(&signature)
        .map_err(|_| hold("test signature not UTF-8"))?;
    let attested = tools_package::verify_signed(root, &raw, signed, key)?;
    held.inspect(|name, file| {
        if name == "manifest.json" || name == "manifest.minisig" {
            let expected: &[u8] = if name == "manifest.json" { &raw } else { &signature };
            let mut contents = Vec::new();
            file.take(expected.len() as u64 + 1).read_to_end(&mut contents)
                .map_err(|_| hold("held signed metadata unreadable"))?;
            if contents != expected { return Err(hold("held metadata changed")); }
        } else if let Some(index) = [
            "akula-video-mux.exe", "akula-yt-dlp.exe", "akula-quickjs.exe",
        ].iter().position(|candidate| *candidate == name) {
            if sha_of_held(file)? != attested.digests[index] {
                return Err(hold("held PE differs from real Minisign contents"));
            }
        } else if name == "uninstall.exe" {
            let size = file.metadata().map_err(|_| hold("uninstaller unavailable"))?.len();
            if !(64..=128 * 1024 * 1024).contains(&size) {
                return Err(hold("uninstaller size outside bounded policy"));
            }
        } else { return Err(hold("unknown pinned component")); }
        Ok(())
    })?;
    // Native production admission MUST remain disabled even with a genuine
    // cryptographically valid TEST key and actual PE bytes.
    if !tools_package::APPROVED_VIDEO_TOOLS_KEY.is_empty()
        || tools_package::shipping_admission_enabled()
        || tools_package_pin_admission::inspect_owner_approved_package(
            root, &raw, signed,
        ).is_ok()
    {
        return Err(hold("ephemeral CI signer escalated into shipping authority"));
    }
    Ok(attested.version)
}
fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(3);
    }
}
fn run() -> Result<(), String> {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("RUNNER_OS").as_deref() != Ok("Windows")
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
        || std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request") {
        return Err(hold("exact-head hosted Windows pull request only"));
    }
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 || !matches!(args[1].as_str(), "missing"|"valid"|"wrong-key"|"tamper-manifest"|"tamper-signature"|"tamper-pe"|"seventh-file"|"status-attested"|"status-incompatible") {
        return Err(hold("expected mode, known-root and isolated ephemeral public key"));
    }
    let expected = PathBuf::from(&args[2]);
    let root = os_root::installed_root()?;
    if args[1] == "missing" {
        if root.is_some() || probe_installed_video_tools() != ToolsDiscovery::Missing {
            return Err(hold("six-file product survived removal or already existed"));
        }
        println!("VIDEO_TOOLS_01J_MISSING_PASS fixed_product=1 release=0");
        return Ok(());
    }
    let root = root.ok_or_else(|| hold("separate protected six-file product missing"))?;
    if root != expected { return Err(hold("environment-selected root differs from OS-known product")); }
    let production_status = probe_installed_video_tools();
    if args[1] == "seventh-file" {
        if production_status != ToolsDiscovery::Unsafe
            || os_root::check_installed_acl(&root, &tools_package_pin_windows::SIGNED_NAMES).is_ok()
        {
            return Err(hold("seventh installed object was not rejected"));
        }
        println!("VIDEO_TOOLS_01J_NATIVE_REJECT_PASS case=seventh-file release=0");
        return Ok(());
    }
    if production_status != ToolsDiscovery::PublisherNotApproved {
        return Err(hold("production native discovery accepted unapproved CI signer"));
    }
    let temp = PathBuf::from(std::env::var_os("RUNNER_TEMP")
        .ok_or_else(|| hold("isolated runner temp missing"))?);
    let key_file = PathBuf::from(&args[3]);
    if !inside(&key_file, &temp)? { return Err(hold("test key escaped runner temp")); }
    let key = fs::read_to_string(&key_file)
        .map_err(|_| hold("ephemeral CI public key unreadable"))?;
    let result = exact_installed(&root, &key);
    if args[1] == "valid" {
        if result? != "0.1.0" { return Err(hold("unexpected separate tools version")); }
        println!("VIDEO_TOOLS_01J_NATIVE_SIGNED_INSTALLED_PASS six_files=6 native_acl=1 real_minisign=1 held_sha256=3 production_admission=0 release=0");
    } else if args[1] == "status-attested" || args[1] == "status-incompatible" {
        // The real installed package's six files were validated through
        // exact source modules, genuine Minisign and held read-only handles.
        // The ephemeral test key is NEVER passed to production discovery.
        let authenticated_version = result?;
        let state = tools_package_probe::classify_authenticated_version(&authenticated_version);
        let expected = if args[1] == "status-attested" {
            ToolsDiscovery::ReadOnlyAttested
        } else {
            ToolsDiscovery::IncompatibleVersion
        };
        if state != expected
            || (expected == ToolsDiscovery::ReadOnlyAttested && authenticated_version != "0.1.0")
            || (expected == ToolsDiscovery::IncompatibleVersion && authenticated_version != "0.2.0")
            || probe_installed_video_tools() != ToolsDiscovery::PublisherNotApproved
        {
            return Err(hold("signed version, real installed state or production HOLD differs"));
        }
        let label = if state == ToolsDiscovery::ReadOnlyAttested {
            "read-only-attested"
        } else { "incompatible-version" };
        println!("VIDEO_TOOLS_01M_NATIVE_STATE_PASS state={label} real_signature=1 os_acl=1 pinned_six=1 publisher_unapproved=1 release=0");
    } else {
        match result {
            Ok(_) => return Err(hold("negative case was falsely accepted")),
            Err(reason) => {
                if !reason.contains("HOLD") { return Err(hold("unexpected unsafe error classification")); }
                println!("VIDEO_TOOLS_01J_NATIVE_REJECT_PASS case={} release=0",args[1]);
            }
        }
    }
    Ok(())
}
