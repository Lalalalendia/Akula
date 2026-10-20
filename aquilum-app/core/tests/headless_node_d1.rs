//! Real separate-process loopback admission checks for developer-only Node D1.
//! The binary is opt-in and never used in installed Akula.

#![cfg(all(target_os = "linux", feature = "node-lab"))]

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const TOKEN: &str = "f1a4d32807ae3901b2dcbca07244a9086eef2f9550e7d1c600fd5b19e3cd4070";
const BAD_TOKEN: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn program() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_akula-node-lab"));
    cmd.env("AKULA_NODE_D1_TOKEN", TOKEN);
    cmd
}

fn args(cmd: &mut Command, root: &Path, appdata: &Path, note: &str, count: u8) {
    cmd.arg("--vault-root").arg(root)
        .arg("--appdata").arg(appdata)
        .arg("--note").arg(note)
        .arg("--port").arg("0")
        .arg("--max-requests").arg(count.to_string());
}

fn launch(root: &Path, state: &Path, note: &str, count: u8) -> (Child, String) {
    let mut cmd = program();
    args(&mut cmd, root, state, note, count);
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("start node child");
    let stdout = child.stdout.take().expect("stdout");
    let mut lines = BufReader::new(stdout);
    let mut first = String::new();
    assert!(lines.read_line(&mut first).unwrap() > 0, "child refused before bind");
    let address = first.trim().strip_prefix("AKULA_NODE_D1_BOUND=")
        .expect("exact loopback witness").to_owned();
    assert!(address.starts_with("127.0.0.1:"), "never an external network bind");
    (child, address)
}

fn send(address: &str, method: &str, route: &str, auth: Option<&str>, extra: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(address).expect("connect loopback");
    stream.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    let auth = auth.map(|value| format!("Authorization: Bearer {value}\r\n"))
        .unwrap_or_default();
    let request = format!("{method} {route} HTTP/1.1\r\nHost: localhost\r\n{auth}{extra}\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let pos = response.windows(4).position(|w| w == b"\r\n\r\n").expect("response headers");
    let header = std::str::from_utf8(&response[..pos]).unwrap();
    let status: u16 = header.lines().next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = serde_json::from_slice(&response[pos + 4..]).expect("response JSON");
    (status, body)
}

fn create_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let vault = scratch.path().join("vault");
    let state = scratch.path().join("state");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&state).unwrap();
    (scratch, vault, state)
}

#[test]
fn one_allowlisted_note_is_reachable_only_with_auth_and_never_mutated() {
    let (_scratch, vault, state) = create_fixture();
    let note = vault.join("allowed.md");
    let bytes = b"# D1 fixed note\r\nContent **unchanged**.\r\n";
    fs::write(&note, bytes).unwrap();
    let (child, address) = launch(&vault, &state, "allowed.md", 8);
    assert_eq!(send(&address, "GET", "/v1/note", None, "").0, 401);
    assert_eq!(send(&address, "GET", "/v1/note", Some(BAD_TOKEN), "").0, 401);
    assert_eq!(send(&address, "GET", "/health", Some(TOKEN), "").0, 200);
    let (status, body) = send(&address, "GET", "/v1/note", Some(TOKEN), "");
    assert_eq!(status, 200);
    assert_eq!(body["note"], "allowed.md");
    assert_eq!(body["content"], std::str::from_utf8(bytes).unwrap());
    assert_eq!(body["rawBlake3"], blake3::hash(bytes).to_hex().to_string());
    assert_eq!(send(&address, "GET", "/../../secret.md", Some(TOKEN), "").0, 404);
    assert_eq!(send(&address, "POST", "/v1/note", Some(TOKEN), "").0, 405);
    assert_eq!(send(&address, "GET", "/%2e%2e/secret.md", Some(TOKEN), "").0, 404);
    assert_eq!(send(&address, "GET", "/v1/note", Some(TOKEN), "Content-Length: 0\r\n").0, 400);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "node stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(fs::read(&note).unwrap(), bytes);
    assert_eq!(fs::read_dir(&vault).unwrap().count(), 1);
    assert!(state.join("settings.json").exists(), "Core state must be separate from the vault");
}

#[test]
fn symlink_to_file_outside_vault_fails_closed() {
    let (scratch, vault, state) = create_fixture();
    let outside = scratch.path().join("private.md");
    fs::write(&outside, b"SECRET: never serve").unwrap();
    symlink(&outside, vault.join("allowed.md")).unwrap();
    let (child, address) = launch(&vault, &state, "allowed.md", 1);
    assert_eq!(send(&address, "GET", "/v1/note", Some(TOKEN), "").0, 403);
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(outside).unwrap(), b"SECRET: never serve");
}

#[test]
fn overlarge_source_is_refused_before_allocating_unbounded_memory() {
    let (_scratch, vault, state) = create_fixture();
    let bytes = vec![b'z'; 64 * 1024 + 1];
    fs::write(vault.join("allowed.md"), &bytes).unwrap();
    let (child, address) = launch(&vault, &state, "allowed.md", 1);
    assert_eq!(send(&address, "GET", "/v1/note", Some(TOKEN), "").0, 413);
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(vault.join("allowed.md")).unwrap(), bytes);
}

#[test]
fn pinned_directory_is_not_reopened_after_path_replacement() {
    let (scratch, vault, state) = create_fixture();
    let original = b"Original allowed content; only this vault was admitted.";
    fs::write(vault.join("allowed.md"), original).unwrap();
    let (child, address) = launch(&vault, &state, "allowed.md", 1);

    // Replace the *path name* while the server is alive. Its pinned fd must
    // still refer to the initially opened directory, not the new replacement.
    let previous = scratch.path().join("original-vault-moved");
    fs::rename(&vault, &previous).unwrap();
    fs::create_dir(&vault).unwrap();
    fs::write(vault.join("allowed.md"), b"Replacement not permitted").unwrap();

    let (status, body) = send(&address, "GET", "/v1/note", Some(TOKEN), "");
    assert_eq!(status, 200);
    assert_eq!(body["content"], std::str::from_utf8(original).unwrap());
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(previous.join("allowed.md")).unwrap(), original);
    assert_eq!(fs::read(vault.join("allowed.md")).unwrap(), b"Replacement not permitted");
}

#[test]
fn node_restart_after_forced_process_death() {
    let (_scratch, vault, state) = create_fixture();
    let original = b"# A D1 restart does not change the source file.\n";
    fs::write(vault.join("allowed.md"), original).unwrap();

    let (mut first, address) = launch(&vault, &state, "allowed.md", 4);
    assert_eq!(send(&address, "GET", "/v1/note", Some(TOKEN), "").0, 200);
    first.kill().expect("force child process termination");
    assert!(!first.wait_with_output().unwrap().status.success());

    // A fresh OS process recovers its private SQLite/settings state and
    // reopens the same immutable permitted fixture through the pinned root.
    let (second, address) = launch(&vault, &state, "allowed.md", 1);
    let (status, body) = send(&address, "GET", "/v1/note", Some(TOKEN), "");
    assert_eq!(status, 200);
    assert_eq!(body["content"], std::str::from_utf8(original).unwrap());
    assert!(second.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(vault.join("allowed.md")).unwrap(), original);
    assert_eq!(fs::read_dir(&vault).unwrap().count(), 1);
}

#[test]
fn request_deadline_rejects_trickled_incomplete_header() {
    use std::time::Instant;

    let (_scratch, vault, state) = create_fixture();
    fs::write(vault.join("allowed.md"), b"unchanged").unwrap();
    let (child, address) = launch(&vault, &state, "allowed.md", 1);
    let mut socket = TcpStream::connect(&address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    let prefix = format!(
        "GET /v1/note HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nX-Trickle: "
    );
    socket.write_all(prefix.as_bytes()).unwrap();
    let start = Instant::now();
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(600));
        socket.write_all(b"x").unwrap();
    }
    // Without a total deadline, the last byte resets the per-read timeout
    // and this request can monopolize the only executor indefinitely.
    let mut wire = Vec::new();
    socket.read_to_end(&mut wire).expect("bounded HTTP 408 response");
    let headers_end = wire.windows(4).position(|w| w == b"\r\n\r\n").expect("headers");
    assert!(std::str::from_utf8(&wire[..headers_end]).unwrap().starts_with("HTTP/1.1 408 "));
    assert!(start.elapsed() < Duration::from_secs(5),
        "whole-request deadline must not restart after each partial read");
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(vault.join("allowed.md")).unwrap(), b"unchanged");
}

#[test]
fn repeated_reads_have_a_bounded_linux_memory_witness() {
    let (_scratch, vault, state) = create_fixture();
    let original = vec![b'A'; 64 * 1024];
    fs::write(vault.join("allowed.md"), &original).unwrap();
    let (child, address) = launch(&vault, &state, "allowed.md", 16);
    let pid = child.id();
    let mut max_rss_kib = 0u64;
    for _ in 0..16 {
        // Capture while the subprocess must still be alive: it intentionally
        // exits immediately after serving the sixteenth request.
        let status_text = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        let rss_kib = status_text.lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|number| number.parse::<u64>().ok())
            .expect("Linux process VmRSS witness");
        max_rss_kib = max_rss_kib.max(rss_kib);
        let (status, body) = send(&address, "GET", "/v1/note", Some(TOKEN), "");
        assert_eq!(status, 200);
        assert_eq!(body["rawBlake3"], blake3::hash(&original).to_hex().to_string());
    }
    println!("AKULA_NODE_D1_PEAK_OBSERVED_RSS_KIB={max_rss_kib}");
    assert!(max_rss_kib <= 512 * 1024,
        "lab Node unexpectedly exceeded the explicit 512 MiB RSS ceiling");
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(vault.join("allowed.md")).unwrap(), original);
}

#[test]
fn startup_rejects_overlapping_state_and_protected_root_without_writing() {
    let (_scratch, vault, _state) = create_fixture();
    let inside = vault.join("private-state");
    fs::create_dir(&inside).unwrap();
    let mut cmd = program();
    args(&mut cmd, &vault, &inside, "allowed.md", 1);
    let out = cmd.output().unwrap();
    assert!(!out.status.success(), "overlapping appdata incorrectly accepted");
    assert!(!inside.join("settings.json").exists());
    assert!(out.stdout.is_empty(), "must refuse before bind");
}

#[test]
fn startup_rejects_root_symlink_and_traversal_config() {
    let (scratch, vault, state) = create_fixture();
    let alias = scratch.path().join("symlink-vault");
    symlink(&vault, &alias).unwrap();
    let mut linked = program();
    args(&mut linked, &alias, &state, "allowed.md", 1);
    let out = linked.output().unwrap();
    assert!(!out.status.success(), "root symlink accepted");
    assert!(out.stdout.is_empty());

    let mut traversal = program();
    args(&mut traversal, &vault, &state, "../private.md", 1);
    let out = traversal.output().unwrap();
    assert!(!out.status.success(), "operator traversal allowed");
    assert!(out.stdout.is_empty());
}
