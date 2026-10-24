//! HTTP contract tests using loopback Dropbox-like responses.
use super::*;
    use crate::{DropboxContentHash, DropboxRevision};
    use crate::read_verify::dropbox_content_hash;
    use std::{
        io::{Read, Write}, net::TcpListener,
        thread,
    };

    fn scope() -> Scope {
        Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "home").unwrap()
    }

    // Fake Dropbox replies on loopback. Production cannot select this endpoint.
    fn server(contents: Vec<u8>, wrong_bytes: bool, redirect: bool)
        -> (Endpoints, thread::JoinHandle<()>)
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for i in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut buf = [0_u8; 4096];
                let header_end;
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(p) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = p + 4;
                        break;
                    }
                }
                let header = std::str::from_utf8(&request[..header_end]).unwrap().to_owned();
                assert!(header.starts_with("POST /2/"));
                assert!(header.to_ascii_lowercase().contains("authorization: bearer fake-token"));
                let size: usize = header.lines()
                    .find_map(|row| row.to_ascii_lowercase().strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())).unwrap_or(0);
                while request.len() - header_end < size {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }

                let hash = dropbox_content_hash(&contents);
                let record = json!({
                    ".tag":"file", "id":"id:abc", "rev":"a1c10ce0dd78",
                    "size":contents.len(), "is_downloadable":true,
                    "content_hash":hash,
                }).to_string();
                let (body, extra) = match i {
                    0 => {
                        assert!(header.contains("/2/users/get_current_account"));
                        (br#"{"account_id":"dbid:alice","root_info":{"root_namespace_id":"12345"}}"#.to_vec(), String::new())
                    },
                    1 => {
                        assert!(header.contains("/2/files/get_metadata"));
                        (record.clone().into_bytes(), String::new())
                    },
                    _ => {
                        assert!(header.contains("/2/files/download"));
                        assert!(header.contains("rev:a1c10ce0dd78"));
                        let bytes = if wrong_bytes { b"invalid".to_vec() } else { contents.clone() };
                        (bytes, format!("Dropbox-API-Result: {}\r\n", record))
                    },
                };
                if redirect && i == 2 {
                    let msg = "HTTP/1.1 302 Found\r\nLocation: https://example.org/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    stream.write_all(msg.as_bytes()).unwrap();
                } else {
                    write!(stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
                        body.len(), extra).unwrap();
                    stream.write_all(&body).unwrap();
                }
            }
        });
        let base = format!("http://{addr}");
        (Endpoints { api: base.clone(), content: base }, handle)
    }

    fn connect_test(endpoints: Endpoints) -> DropboxReadOnly {
        DropboxReadOnly::connect_at(scope(),
            AccessToken::new("fake-token".to_owned()).unwrap(), endpoints).unwrap()
    }

    #[test]
    fn list_and_continue_use_only_verified_account_and_read_routes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for i in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(p) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                };
                let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
                assert!(headers.to_ascii_lowercase().contains("authorization: bearer fake-token"));
                let size = headers.lines().find_map(|line|
                    line.to_ascii_lowercase().strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
                while request.len() - header_end < size {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                let body = &request[header_end..header_end + size];
                let reply = match i {
                    0 => {
                        assert!(headers.contains("/2/users/get_current_account"));
                        json!({"account_id":"dbid:alice",
                            "root_info":{"root_namespace_id":"12345"}})
                    }
                    1 => {
                        assert!(headers.contains("/2/files/list_folder "));
                        let payload: Value = serde_json::from_slice(body).unwrap();
                        assert_eq!(payload["recursive"], true);
                        assert_eq!(payload["path"], format!(
                            "/akula-sync/v1/vaults/{}",scope().vault_id()));
                        json!({"entries":[],"cursor":"c1","has_more":true})
                    }
                    _ => {
                        assert!(headers.contains("/2/files/list_folder/continue "));
                        let payload: Value = serde_json::from_slice(body).unwrap();
                        assert_eq!(payload["cursor"], "c1");
                        json!({"entries":[],"cursor":"c2","has_more":false})
                    }
                }.to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),reply).unwrap();
            }
        });
        let base = format!("http://{addr}");
        let client = connect_test(Endpoints { api: base.clone(), content: base });
        let snapshot = client.snapshot().unwrap();
        assert!(snapshot.entries.is_empty());
        assert_eq!(snapshot.cursor, "c2");
        assert!(!snapshot.has_more);
        worker.join().unwrap();
    }

    #[test]
    fn read_exact_revision_with_two_hashes_and_account_proof() {
        let bytes = b"hello akula".to_vec();
        let (endpoints, thread) = server(bytes.clone(), false, false);
        let client = connect_test(endpoints);
        let file = client.metadata(&ObjectKey::head()).unwrap();
        assert_eq!(file.revision, DropboxRevision::parse("a1c10ce0dd78").unwrap());
        assert_eq!(file.dropbox_hash, Some(DropboxContentHash::parse(
            &dropbox_content_hash(&bytes)).unwrap()));
        let trusted = TrustedBlake3::parse(
            &blake3::hash(&bytes).to_hex().to_string()).unwrap();
        let downloaded = client.download_verified(&file, &trusted).unwrap();
        assert_eq!(downloaded.bytes, bytes);
        thread.join().unwrap();
    }

    #[test]
    fn same_length_changed_payload_does_not_verify() {
        let (endpoints, thread) = server(b"invalid".to_vec(), true, false);
        let client = connect_test(endpoints);
        let file = client.metadata(&ObjectKey::head()).unwrap();
        let trusted = TrustedBlake3::parse(
            &blake3::hash(b"original").to_hex().to_string()).unwrap();
        let result = client.download_verified(&file, &trusted);
        assert_eq!(result, Err(ReadError::Verify(VerifyError::TrustedHashMismatch)));
        thread.join().unwrap();
    }

    #[test]
    fn public_read_key_cannot_bypass_scoped_metadata_lookup() {
        let payload = b"protected-by-trusted-anchor".to_vec();
        let (endpoints, server) = server(payload.clone(), false, false);
        let client = connect_test(endpoints);
        let trusted = TrustedBlake3::parse(
            &blake3::hash(&payload).to_hex().to_string()).unwrap();
        let verified = client.read_key_verified(&ObjectKey::head(), &trusted).unwrap();
        assert_eq!(verified.bytes, payload);
        server.join().unwrap();
    }

    #[test]
    fn redirects_are_rejected_without_leaking_token() {
        let (endpoints, thread) = server(b"original".to_vec(), false, true);
        let client = connect_test(endpoints);
        let file = client.metadata(&ObjectKey::head()).unwrap();
        let trusted = TrustedBlake3::parse(
            &blake3::hash(b"original").to_hex().to_string()).unwrap();
        assert!(client.download_verified(&file, &trusted).is_err());
        assert_eq!(format!("{:?}", client.token), "AccessToken(REDACTED)");
        thread.join().unwrap();
    }
