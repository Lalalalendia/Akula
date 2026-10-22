#![cfg(feature = "process-lab")]
// Test-only fixture. Temporary plaintext keys are NOT production key custody.
use akula_roha_transport::{AllowedPeers, StayaTransport};
use iroh::{EndpointAddr, SecretKey};
use std::{
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ROLE: &str = "STY_DEVICE_PROCESS_LAB_ROLE";
const DIR: &str = "STY_DEVICE_PROCESS_LAB_DIR";

fn read_key(path: &Path) -> SecretKey {
    let bytes: [u8; 32] = fs::read(path).unwrap().try_into().unwrap();
    SecretKey::from_bytes(&bytes)
}

async fn wait_signal(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "no signal: {:?}",
            path
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn remote_address(dir: &Path) -> EndpointAddr {
    let socket: SocketAddr = fs::read_to_string(dir.join("ready"))
        .unwrap()
        .parse()
        .unwrap();
    EndpointAddr::new(read_key(&dir.join("server.key")).public()).with_ip_addr(socket)
}

async fn server(dir: &Path) {
    let expected = read_key(&dir.join("client.key")).public();
    let external_authorized = Arc::new(AtomicBool::new(true));
    let policy = AllowedPeers::new([expected]).with_live_check({
        let external_authorized = Arc::clone(&external_authorized);
        move |authenticated| {
            authenticated == &expected && external_authorized.load(Ordering::Acquire)
        }
    });
    let transport =
        StayaTransport::bind_loopback_lab(read_key(&dir.join("server.key")), policy.clone())
            .await
            .unwrap();
    let addr = transport.loopback_address_lab().unwrap();
    fs::write(
        dir.join("ready"),
        addr.ip_addrs().next().unwrap().to_string(),
    )
    .unwrap();

    let conn = tokio::time::timeout(Duration::from_secs(20), transport.accept_one())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(conn.remote_id(), expected);
    transport.authorize_new_operation(&conn).unwrap();
    let (mut tx, mut rx) = conn.accept_bi().await.unwrap();
    assert_eq!(rx.read_to_end(32).await.unwrap(), b"FIRST");
    tx.write_all(b"ACK").await.unwrap();
    tx.finish().unwrap();
    wait_signal(&dir.join("ack-seen")).await;
    // Emulate a protected OS pin being revoked in a separate authority.
    // The Roha session remains alive and the in-memory peer is still listed;
    // only a FRESH live check denies the second logical operation.
    external_authorized.store(false, Ordering::Release);
    assert!(!policy.contains(&expected));
    assert_eq!(
        transport.authorize_new_operation(&conn),
        Err("peer not admitted for new operation".to_owned())
    );
    // The original process-local kill switch remains valid independently.
    assert!(policy.revoke(&expected));
    fs::write(dir.join("revoked"), b"1").unwrap();
    let (_tx2, _rx2) = tokio::time::timeout(Duration::from_secs(10), conn.accept_bi())
        .await
        .unwrap()
        .unwrap();
    // Do not read or apply the second stream's contents.
    assert!(transport.authorize_new_operation(&conn).is_err());
    fs::write(dir.join("audit"), b"FIRST_IN_MEMORY\nSECOND_DENIED\n").unwrap();
    conn.close(403u32.into(), b"revoked");
    transport.close().await;
}

async fn client(dir: &Path) {
    let expected_server = read_key(&dir.join("server.key")).public();
    let transport = StayaTransport::bind_loopback_lab(
        read_key(&dir.join("client.key")),
        AllowedPeers::new([expected_server]),
    )
    .await
    .unwrap();
    let conn = transport.connect(remote_address(dir)).await.unwrap();
    assert_eq!(conn.remote_id(), expected_server);
    let (mut tx, mut rx) = conn.open_bi().await.unwrap();
    tx.write_all(b"FIRST").await.unwrap();
    tx.finish().unwrap();
    assert_eq!(rx.read_to_end(32).await.unwrap(), b"ACK");
    fs::write(dir.join("ack-seen"), b"1").unwrap();
    wait_signal(&dir.join("revoked")).await;
    let (mut tx2, _rx2) = conn.open_bi().await.unwrap();
    let _ = tx2.write_all(b"SECOND").await;
    let _ = tx2.finish();
    tokio::time::timeout(Duration::from_secs(10), conn.closed())
        .await
        .unwrap();
    transport.close().await;
}

#[test]
fn child_entry() {
    let Ok(role) = env::var(ROLE) else { return };
    let dir = PathBuf::from(env::var(DIR).unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        match role.as_str() {
            "server" => server(&dir).await,
            "client" => client(&dir).await,
            "restart" => {
                let id = read_key(&dir.join("server.key")).public();
                fs::write(dir.join("restarted"), id.to_string()).unwrap();
            }
            _ => panic!("invalid process role"),
        }
    });
}

fn spawn(dir: &Path, role: &str) -> Child {
    Command::new(env::current_exe().unwrap())
        .args(["--exact", "child_entry", "--nocapture"])
        .env(ROLE, role)
        .env(DIR, dir)
        .spawn()
        .unwrap()
}

fn finish(mut child: Child) {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "child process failed: {status}");
            return;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child process timed out");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn two_os_processes_keep_identity_and_deny_after_live_revoke() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = env::temp_dir().join(format!("akula-iroh-process-{}-{stamp}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let _cleanup = TempDir(dir.clone());
    let receiver_key = SecretKey::generate();
    let client_key = SecretKey::generate();
    let receiver_id = receiver_key.public();
    fs::write(dir.join("server.key"), receiver_key.to_bytes()).unwrap();
    fs::write(dir.join("client.key"), client_key.to_bytes()).unwrap();

    let receiver = spawn(&dir, "server");
    let start = Instant::now();
    while !dir.join("ready").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "server not ready"
        );
        thread::sleep(Duration::from_millis(25));
    }
    finish(spawn(&dir, "client"));
    finish(receiver);
    assert_eq!(
        fs::read(dir.join("audit")).unwrap(),
        b"FIRST_IN_MEMORY\nSECOND_DENIED\n"
    );

    // A separate restarted process loads the same test-only 32-byte key.
    finish(spawn(&dir, "restart"));
    assert_eq!(
        fs::read_to_string(dir.join("restarted")).unwrap(),
        receiver_id.to_string()
    );
}
