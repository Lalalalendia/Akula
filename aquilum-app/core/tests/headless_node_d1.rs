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

const TOKEN: &str = "f1a4d32807ae3901b2dcbca07244a9086eef2f9550e7d1c600fd5b19e3cd407";
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
