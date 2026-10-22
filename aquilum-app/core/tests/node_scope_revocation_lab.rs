//! Real Linux policy peers: scoped signed anti-memes, no user file mutation.
#![cfg(all(target_os = "linux", feature = "node-scope-revocation-lab"))]

use aquilum_core::node_catalog_lab::ReadCatalog;
#[path = "../src/node_scope_revocation_lab.rs"]
mod node_scope_revocation_lab;
use node_scope_revocation_lab::{bytes_to_sign, decode_hex, encode_hex};
use aquilum_core::sync::VaultId;
use ring::signature::{self, KeyPair};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const TOKEN: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const CONTENT: &[u8] = b"# Synthetic allowed document\r\noriginal byte sequence.\r\n";

fn admin() -> signature::Ed25519KeyPair {
    signature::Ed25519KeyPair::from_seed_unchecked(&[87u8; 32]).unwrap()
}
fn outsider() -> signature::Ed25519KeyPair {
    signature::Ed25519KeyPair::from_seed_unchecked(&[13u8; 32]).unwrap()
}
fn scope(number: u128) -> VaultId {
    VaultId::from_uuid(uuid::Uuid::from_u128(number))
}
fn sign(
    area: VaultId, document: &str, sequence: u64,
    key: &signature::Ed25519KeyPair,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&decode_hex::<32>(document).unwrap());
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(
        key.sign(&bytes_to_sign(area, document, sequence).unwrap()).as_ref(),
    );
    assert_eq!(bytes.len(), 104);
    encode_hex(&bytes)
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    state: PathBuf,
    id: String,
    area: VaultId,
}
impl Fixture {
    fn new(area: VaultId) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let state = temp.path().join("appdata");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(root.join("public.md"), CONTENT).unwrap();
        let catalog = ReadCatalog::open(&root, area, &["public.md"]).unwrap();
        let id = catalog.list().pop().unwrap().id;
        Self { _temp: temp, root, state, id, area }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_akula-node-policy-lab"));
        cmd.env("AKULA_POLICY_LAB_TOKEN", TOKEN)
            .arg("--vault-root").arg(&self.root)
            .arg("--appdata").arg(&self.state)
            .arg("--vault-id").arg(self.area.to_string())
            .arg("--note").arg("public.md")
            .arg("--admin-key").arg(encode_hex(admin().public_key().as_ref()))
            .arg("--port").arg("0")
            .arg("--max-requests").arg("80");
        cmd
    }
}

struct Running {
    child: Child,
    address: String,
}
impl Running {
    fn spawn(fixture: &Fixture) -> Self {
        let mut child = fixture.command()
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        let count = BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line).unwrap();
        assert!(count > 0, "policy node refused startup");
        let addr = line.trim().strip_prefix("AKULA_SCOPE_LAB_BOUND=")
            .expect("expected loopback binding");
        assert!(addr.starts_with("127.0.0.1:"));
        Self { child, address: addr.to_owned() }
    }
    fn call(&self, request: &str) -> Value {
        self.call_with_token(TOKEN, request)
    }
    fn call_with_token(&self, token: &str, request: &str) -> Value {
        let mut stream = TcpStream::connect(&self.address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(4))).unwrap();
        // An unauthenticated peer must not pipeline an application command:
        // the server rejects immediately after AUTH, deliberately leaving any
        // extra bytes unread. Closing with those bytes can cause TCP RST.
        // Test the actual auth refusal, not a kernel-specific reset.
        write!(stream, "AUTH {token}\n").unwrap();
        if token == TOKEN {
            write!(stream, "{request}\n").unwrap();
        }
        stream.shutdown(Shutdown::Write).unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        serde_json::from_slice(&out).unwrap()
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn three_processes_propagate_a_b_c_then_offline_restart_preserves_suppression() {
    let area_a = scope(1);
    let a = Fixture::new(area_a);
    let b = Fixture::new(area_a);
    let c = Fixture::new(area_a);
    assert_eq!(a.id, b.id);
    assert_eq!(b.id, c.id);
    let peers = [Running::spawn(&a), Running::spawn(&b), Running::spawn(&c)];
    for peer in &peers {
        let result = peer.call(&format!("READ {}", a.id));
        assert_eq!(result["status"], "allowed");
        assert_eq!(result["content"], std::str::from_utf8(CONTENT).unwrap());
    }
    let anti = sign(area_a, &a.id, 2, &admin());
    assert_eq!(peers[0].call(&format!("BAN {anti}"))["status"], "applied");
    assert_eq!(peers[0].call(&format!("READ {}", a.id))["status"], "blocked");
    assert_eq!(peers[1].call(&format!("READ {}", b.id))["status"], "allowed");
    assert_eq!(peers[2].call(&format!("READ {}", c.id))["status"], "allowed");

    let proof_a = peers[0].call(&format!("PROOF {}", a.id));
    assert_eq!(proof_a["proof"], anti);
    assert_eq!(peers[1].call(&format!("BAN {}", proof_a["proof"].as_str().unwrap()))["status"], "applied");
    assert_eq!(peers[1].call(&format!("READ {}", b.id))["status"], "blocked");
    let proof_b = peers[1].call(&format!("PROOF {}", b.id));
    assert_eq!(peers[2].call(&format!("BAN {}", proof_b["proof"].as_str().unwrap()))["status"], "applied");
    assert_eq!(peers[2].call(&format!("READ {}", c.id))["status"], "blocked");

    drop(peers);
    let restarted = Running::spawn(&c);
    assert_eq!(restarted.call(&format!("READ {}", c.id))["status"], "blocked");
    assert_eq!(restarted.call(&format!("PROOF {}", c.id))["proof"], anti);

    let area_b = scope(2);
    let d = Fixture::new(area_b);
    let separate = Running::spawn(&d);
    assert_ne!(d.id, a.id);
    // Even a valid signature on B's exact document ID is ineffective if
    // it was authored for area A.
    let wrong_area = sign(area_a, &d.id, 3, &admin());
    assert_eq!(separate.call(&format!("BAN {wrong_area}"))["status"], "rejected");
    assert_eq!(separate.call(&format!("READ {}", d.id))["status"], "allowed");

    for f in [&a, &b, &c, &d] {
        assert_eq!(fs::read(f.root.join("public.md")).unwrap(), CONTENT);
        assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    }
}

#[test]
fn stale_anti_returns_latest_owner_scope_proof_without_exposing_data() {
    let area = scope(1);
    let f = Fixture::new(area);
    let peer = Running::spawn(&f);
    let latest = sign(area, &f.id, 7, &admin());
    assert_eq!(peer.call(&format!("BAN {latest}"))["status"], "applied");
    assert_eq!(peer.call(&format!("BAN {latest}"))["status"], "duplicate");
    let previous = sign(area, &f.id, 3, &admin());
    let answer = peer.call(&format!("BAN {previous}"));
    assert_eq!(answer["status"], "stale");
    assert_eq!(answer["proof"], latest);
    assert_eq!(peer.call(&format!("BAN {}", sign(area, &f.id, 8, &outsider())))["status"], "rejected");
    assert_eq!(peer.call(&format!("BAN {}", sign(scope(9), &f.id, 8, &admin())))["status"], "rejected");
    assert_eq!(peer.call_with_token(&"0".repeat(64), &format!("READ {}", f.id))["status"], "unauthorized");
    assert_eq!(peer.call(&format!("READ {}", f.id))["status"], "blocked");
    assert_eq!(peer.call(&format!("PROOF {}", f.id))["proof"], latest);
    assert_eq!(fs::read(f.root.join("public.md")).unwrap(), CONTENT);
}

#[test]
fn corrupted_sqlite_proof_refuses_rebind_before_exposing_any_bytes() {
    let f = Fixture::new(scope(1));
    let peer = Running::spawn(&f);
    let proof = sign(f.area, &f.id, 1, &admin());
    assert_eq!(peer.call(&format!("BAN {proof}"))["status"], "applied");
    drop(peer);
    let db = f.state.join("node-scope-revocations.lab.sqlite");
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute("UPDATE lab_revocations SET sig=zeroblob(64)", []).unwrap();
    drop(conn);
    let result = f.command().output().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty(), "failed verification cannot bind");
    assert_eq!(fs::read(f.root.join("public.md")).unwrap(), CONTENT);
}
