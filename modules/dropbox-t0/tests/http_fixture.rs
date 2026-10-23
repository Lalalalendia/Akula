//! Local HTTP wire fixture: writes never leave 127.0.0.1.
//! The production T0 crate has no network client.
use akula_dropbox_t0::{
    classify_http, classify_network_loss, prepare_upload, DropboxRevision, Error, ObjectKey,
    Operation, Scope, WriteGuard,
};
use serde_json::Value;
use std::{io::{Read, Write}, net::TcpListener, sync::{Arc, Mutex}, thread};

fn scope() -> Scope {
    Scope::new("dbid:alice", "12345",
        "12345678-1234-1234-1234-123456789abc", "home").unwrap()
}

#[derive(Default)]
struct Memory {
    version: u64,
    current: Option<Vec<u8>>,
    received: usize,
}

/// One-request local HTTP server, deterministic conflict and deliberately lost ACK.
fn server(store: Arc<Mutex<Memory>>, lose_ack: bool) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let mut bytes = Vec::new();
        let mut temp = [0_u8; 4096];
        let header_end;
        loop {
            let n = stream.read(&mut temp).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&temp[..n]);
            if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                header_end = end + 4;
                break;
            }
        }
        let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let size: usize = header.lines()
            .find_map(|s| s.to_ascii_lowercase().strip_prefix("content-length:")
                .and_then(|v| v.trim().parse().ok()))
            .unwrap();
        while bytes.len() - header_end < size {
            let n = stream.read(&mut temp).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&temp[..n]);
        }
        let request: Value = serde_json::from_slice(&bytes[header_end..header_end + size]).unwrap();
        assert_eq!(request["autorename"], false);
        assert_eq!(request["strict_conflict"], true);
        let mut store = store.lock().unwrap();
        store.received += 1;
        let allowed = match request["mode"].as_str() {
            Some("add") => store.current.is_none(),
            _ => request["mode"][".tag"] == "update"
                && request["mode"]["update"] == format!("R{}", store.version),
        };
        if allowed {
            store.version += 1;
            store.current = Some(request["path"].as_str().unwrap().as_bytes().to_vec());
        }
        if !lose_ack {
            let (status, body) = if allowed {
                ("200 OK", r#"{"id":"id:file","rev":"R1","size":5}"#)
            } else {
                ("409 Conflict", r#"{"error":{".tag":"path","path":{".tag":"conflict"}}}"#)
            };
            write!(stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()).unwrap();
        }
    });
    (addr.to_string(), handle)
}

fn send(url: &str, request: &Value) -> Result<(u16, Vec<u8>), std::io::Error> {
    let mut stream = std::net::TcpStream::connect(url)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    let body = request.to_string();
    write!(stream, "POST /2/files/upload HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    if response.is_empty() {
        return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
    }
    let status = std::str::from_utf8(&response[..12])
        .ok().and_then(|v| v.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok()).unwrap();
    let pos = response.windows(4).position(|x| x == b"\r\n\r\n").unwrap() + 4;
    Ok((status, response[pos..].to_vec()))
}

#[test]
fn two_writers_cannot_both_replace_same_revision() {
    let store = Arc::new(Mutex::new(Memory {
        version: 1, current: Some(vec![1]), received: 0,
    }));
    let proposal = prepare_upload(&scope(), &ObjectKey::head(),
        WriteGuard::ExactRevision(DropboxRevision::parse("R1").unwrap())).unwrap();
    let wire: Value = serde_json::from_str(&proposal.api_arg).unwrap();
    let (address, run) = server(store.clone(), false);
    let (status, body) = send(&address, &wire).unwrap();
    run.join().unwrap();
    assert_eq!(classify_http(Operation::Upload, status, &body, None), Ok(()));

    let (address, run) = server(store.clone(), false);
    let (status, body) = send(&address, &wire).unwrap();
    run.join().unwrap();
    assert_eq!(classify_http(Operation::Upload, status, &body, None),
        Err(Error::PreconditionFailed));
    let store = store.lock().unwrap();
    assert_eq!(store.version, 2);
    assert_eq!(store.received, 2);
}

#[test]
fn ambiguous_ack_requires_readback_and_forbids_blind_retry() {
    let store = Arc::new(Mutex::new(Memory::default()));
    let proposal = prepare_upload(&scope(), &ObjectKey::head(), WriteGuard::Absent).unwrap();
    let wire: Value = serde_json::from_str(&proposal.api_arg).unwrap();
    let (address, run) = server(store.clone(), true);
    assert!(send(&address, &wire).is_err());
    run.join().unwrap();
    assert_eq!(classify_network_loss(Operation::Upload), Error::CommitUncertain);
    assert_eq!(store.lock().unwrap().received, 1);
    assert_eq!(store.lock().unwrap().version, 1);
}

#[test]
fn separate_vaults_get_different_keys() {
    let second = Scope::new("dbid:alice", "12345",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "home").unwrap();
    assert_ne!(scope().object_path(&ObjectKey::head()),
        second.object_path(&ObjectKey::head()));
}
