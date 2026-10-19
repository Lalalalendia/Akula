//! D2 staging: two-layer admission in real Linux subprocess over localhost.
//! Operator admission fixes a VaultId and catalog; one local bearer token
//! receives a narrower read scope. Stable document IDs do NOT authorize access.

#![cfg(all(target_os = "linux", feature = "node-lab"))]

use aquilum_core::node_catalog_lab::ReadCatalog;
use aquilum_core::sync::VaultId;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn token() -> String { "b".repeat(64) }
fn wrong_token() -> String { "a".repeat(64) }
fn vault_id() -> VaultId { VaultId::from_uuid(uuid::Uuid::from_u128(1)) }

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    let state = tmp.path().join("state");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&state).unwrap();
    (tmp, vault, state)
}

fn command(vault: &Path, state: &Path, id: VaultId, reads: u8) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_akula-node-lab"));
    cmd.env("AKULA_NODE_D1_TOKEN", token())
        .arg("--vault-root").arg(vault)
        .arg("--appdata").arg(state)
        .arg("--note").arg("public.md")
        .arg("--vault-id").arg(id.to_string())
        .arg("--catalog-note").arg("public.md")
        .arg("--catalog-note").arg("hidden.md")
        .arg("--reader-note").arg("public.md")
        .arg("--port").arg("0")
        .arg("--max-requests").arg(reads.to_string());
    cmd
}

fn launch(vault: &Path, state: &Path, id: VaultId, reads: u8) -> (Child, String) {
    let mut child = command(vault, state, id, reads)
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let out = child.stdout.take().unwrap();
    let mut line = String::new();
    let n = BufReader::new(out).read_line(&mut line).unwrap();
    assert!(n > 0, "Node rejected before binding");
    let address = line.trim().strip_prefix("AKULA_NODE_D1_BOUND=")
        .expect("explicit loopback witness").to_owned();
    assert!(address.starts_with("127.0.0.1:"), "no network interface exposure");
    (child, address)
}

fn request(address: &str, method: &str, path: &str, bearer: Option<&str>) -> (u16, Value) {
    let mut socket = TcpStream::connect(address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    let auth = bearer.map(|value| format!("Authorization: Bearer {value}\r\n"))
        .unwrap_or_default();
    write!(socket, "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}\r\n").unwrap();
    socket.flush().unwrap();
    let mut wire = Vec::new();
    socket.read_to_end(&mut wire).unwrap();
    let i = wire.windows(4).position(|x| x == b"\r\n\r\n").unwrap();
    let status = std::str::from_utf8(&wire[..i]).unwrap().lines()
        .next().unwrap().split_whitespace().nth(1).unwrap().parse::<u16>().unwrap();
    let body = serde_json::from_slice(&wire[i + 4..]).unwrap();
    (status, body)
}

fn id(catalog: &ReadCatalog, name: &str) -> String {
    catalog.list().into_iter().find(|entry| entry.filename == name).unwrap().id
}

#[test]
fn bearer_scope_is_enforced_before_listing_or_reading_hidden_document() {
    let (_tmp, vault, state) = fixture();
    let public = b"# PUBLIC\r\nDo not normalize CRLF.\r\n";
    let hidden = b"# HIDDEN\r\nPrivate content.\r\n";
    fs::write(vault.join("public.md"), public).unwrap();
    fs::write(vault.join("hidden.md"), hidden).unwrap();
    let catalog = ReadCatalog::open(&vault, vault_id(), &["public.md", "hidden.md"]).unwrap();
    let public_id = id(&catalog, "public.md");
    let hidden_id = id(&catalog, "hidden.md");

    let (child, address) = launch(&vault, &state, vault_id(), 10);
    assert_eq!(request(&address, "GET", "/v1/documents", None).0, 401);
    assert_eq!(request(&address, "GET", "/v1/documents", Some(&wrong_token())).0, 401);

    let (code, listing) = request(&address, "GET", "/v1/documents", Some(&token()));
    assert_eq!(code, 200);
    assert_eq!(listing["vaultId"], vault_id().to_string());
    let docs = listing["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["id"], public_id);
    assert_eq!(docs[0]["filename"], "public.md");
    assert!(!listing.to_string().contains("hidden.md"), "no hidden metadata leak");
    assert_eq!(request(
        &address, "GET", &format!("/v1/documents/{hidden_id}"), Some(&token())
    ).0, 404, "knowing a valid hidden ID must not grant access");

    let (code, note) = request(
        &address, "GET", &format!("/v1/documents/{public_id}"), Some(&token())
    );
    assert_eq!(code, 200);
    assert_eq!(note["id"], public_id);
    assert_eq!(note["content"], std::str::from_utf8(public).unwrap());
    assert_eq!(note["rawBlake3"], blake3::hash(public).to_hex().to_string());

    assert_eq!(request(
        &address, "GET",
        &format!("/v1/documents/{public_id}?rev={}", "0".repeat(64)),
        Some(&token())
    ).0, 409);
    assert_eq!(request(
        &address, "GET",
        &format!("/v1/documents/{public_id}?rev={}", blake3::hash(public).to_hex()),
        Some(&token())
    ).0, 200);
    assert_eq!(request(&address, "GET", "/v1/documents/../../etc/passwd", Some(&token())).0, 404);
    assert_eq!(request(
        &address, "POST", &format!("/v1/documents/{public_id}"), Some(&token())
    ).0, 405);
    assert_eq!(request(
        &address, "GET", &format!("/v1/documents/{}", "0".repeat(64)), Some(&token())
    ).0, 404);

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "server stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(vault.join("public.md")).unwrap(), public);
    assert_eq!(fs::read(vault.join("hidden.md")).unwrap(), hidden);
    assert_eq!(fs::read_dir(&vault).unwrap().count(), 2);
}

#[test]
fn a_different_vault_id_rejects_an_old_object_id_on_the_same_path() {
    let (_tmp, vault, state) = fixture();
    fs::write(vault.join("public.md"), b"one").unwrap();
    fs::write(vault.join("hidden.md"), b"two").unwrap();
    let prior = ReadCatalog::open(&vault, vault_id(), &["public.md", "hidden.md"]).unwrap();
    let stale_id = id(&prior, "public.md");
    let second_vault = VaultId::from_uuid(uuid::Uuid::from_u128(2));
    let current = ReadCatalog::open(&vault, second_vault, &["public.md", "hidden.md"]).unwrap();
    let fresh_id = id(&current, "public.md");
    assert_ne!(stale_id, fresh_id);

    let (child, address) = launch(&vault, &state, second_vault, 2);
    assert_eq!(request(
        &address, "GET", &format!("/v1/documents/{stale_id}"), Some(&token())
    ).0, 404);
    let (code, current) = request(
        &address, "GET", &format!("/v1/documents/{fresh_id}"), Some(&token())
    );
    assert_eq!(code, 200);
    assert_eq!(current["content"], "one");
    assert!(child.wait_with_output().unwrap().status.success());
}

#[test]
fn invalid_reader_scope_refuses_to_bind_and_does_not_create_private_state() {
    let (_tmp, vault, state) = fixture();
    fs::write(vault.join("public.md"), b"one").unwrap();
    fs::write(vault.join("hidden.md"), b"two").unwrap();
    let mut cmd = command(&vault, &state, vault_id(), 1);
    cmd.arg("--reader-note").arg("nonexistent.md");
    let result = cmd.output().unwrap();
    assert!(!result.status.success(), "unauthorized reader list must be denied");
    assert!(result.stdout.is_empty(), "no socket binding on bad reader scope");
    assert!(!state.join("settings.json").exists());
}

#[test]
fn approved_document_inode_swap_returns_conflict_without_exposing_new_bytes() {
    let (_tmp, vault, state) = fixture();
    let path = vault.join("public.md");
    fs::write(&path, b"original").unwrap();
    fs::write(vault.join("hidden.md"), b"private").unwrap();
    let catalog = ReadCatalog::open(&vault, vault_id(), &["public.md", "hidden.md"]).unwrap();
    let admitted_id = id(&catalog, "public.md");
    let (child, address) = launch(&vault, &state, vault_id(), 1);
    fs::rename(&path, vault.join("retired.md")).unwrap();
    fs::write(&path, b"substituted new inode").unwrap();

    assert_eq!(request(
        &address, "GET", &format!("/v1/documents/{admitted_id}"), Some(&token())
    ).0, 409);
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(fs::read(vault.join("retired.md")).unwrap(), b"original");
}
