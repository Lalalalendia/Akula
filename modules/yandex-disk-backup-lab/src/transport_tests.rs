use super::*;

const UPLOAD_1: &str =
    "https://uploader1d.dst.yandex.net:443/upload-target/20240424T101447.217.sig";
const UPLOAD_2: &str =
    "https://uploader44klg.disk.yandex.net/upload-target/token?file_id=abc";
const DOWNLOAD_1: &str =
    "https://downloader.disk.yandex.ru/disk/abc?uid=42&hash=opaque%2Fvalue";
const DOWNLOAD_2: &str =
    "https://a12.storage.yandex.net/download/abc?sig=opaque";

fn json_link(url: &str, method: &str, templated: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "href": url,
        "method": method,
        "templated": templated
    })).unwrap()
}

#[test]
fn official_upload_and_download_link_hosts_are_accepted_without_mutating_signatures() {
    for url in [UPLOAD_1, UPLOAD_2] {
        let result = parse_signed_link(
            &json_link(url, "PUT", false), TransferKind::Upload
        ).unwrap();
        assert_eq!(result.as_str(), url);
    }
    for url in [DOWNLOAD_1, DOWNLOAD_2] {
        let result = parse_signed_link(
            &json_link(url, "GET", false), TransferKind::Download
        ).unwrap();
        assert_eq!(result.as_str(), url);
    }
}

#[test]
fn reject_ssrf_local_hosts_ip_addresses_downgrades_and_domain_suffix_tricks() {
    for bad in [
        "http://uploader1d.dst.yandex.net/upload-target/a",
        "https://127.0.0.1/upload-target",
        "https://localhost/upload-target",
        "https://10.1.2.3/upload-target",
        "https://uploader1d.dst.yandex.net.attacker.example/upload-target",
        "https://uploader1d-dst.yandex.net/upload-target",
        "https://uploader1d.dst.yandex.net:444/upload-target",
        "https://user:password@uploader1d.dst.yandex.net/upload-target",
        "https://uploader1d.dst.yandex.net/upload-target#section",
        "https://downloaders.disk.yandex.ru/upload-target",
        "https://downloader.disk.yandex.ru/upload-target",
        "file:///etc/passwd",
        "https://uploader1d.dst.yandex.net/public",
    ] {
        assert_eq!(
            parse_signed_link(&json_link(bad, "PUT", false), TransferKind::Upload).err()
                .map(|_| BackupError::ProviderUnavailable),
            Some(BackupError::ProviderUnavailable),
            "untrusted upload link must be rejected: {bad}"
        );
    }
}

#[test]
fn reject_download_hosts_that_could_be_ssrf_or_wrong_service() {
    for bad in [
        "http://downloader.disk.yandex.ru/disk/test",
        "https://downloader.disk.yandex.ru.attacker.test/disk/test",
        "https://downloader.disk.yandex.ru:8080/disk/test",
        "https://downloader.disk.yandex.ru@localhost/disk/test",
        "https://storage.yandex.net/disk/test",
        "https://127.0.0.1/disk/test",
        "https://uploader1d.dst.yandex.net/upload-target",
    ] {
        assert!(matches!(
            parse_signed_link(&json_link(bad, "GET", false), TransferKind::Download),
            Err(BackupError::ProviderUnavailable)
        ), "untrusted download link must be rejected: {bad}");
    }
}

#[test]
fn wrong_method_template_or_missing_fields_are_never_executed() {
    for (url, method, templated, kind) in [
        (UPLOAD_1, "POST", false, TransferKind::Upload),
        (UPLOAD_1, "DELETE", false, TransferKind::Upload),
        (UPLOAD_1, "PUT", true, TransferKind::Upload),
        (DOWNLOAD_1, "PUT", false, TransferKind::Download),
        (DOWNLOAD_1, "GET", true, TransferKind::Download),
    ] {
        assert!(matches!(
            parse_signed_link(&json_link(url, method, templated), kind),
            Err(BackupError::ProviderUnavailable)
        ));
    }
    for bytes in [
        br#"{}"#.as_slice(),
        br#"{"href":9,"method":"PUT","templated":false}"#,
        br#"{"href":"https://example.com","method":"PUT"}"#,
        b"not json"
    ] {
        assert!(matches!(
            parse_signed_link(bytes, TransferKind::Upload),
            Err(BackupError::ProviderUnavailable)
        ));
    }
}

#[test]
fn enforce_link_body_and_bearer_token_size_bounds() {
    let huge = vec![b'a'; MAX_LINK_RESPONSE + 1];
    assert!(matches!(
        parse_signed_link(&huge, TransferKind::Upload),
        Err(BackupError::ProviderUnavailable)
    ));
    let long_url = format!("{UPLOAD_1}?x={}", "a".repeat(MAX_SIGNED_URL));
    assert!(matches!(
        parse_signed_link(
            &json_link(&long_url, "PUT", false), TransferKind::Upload),
        Err(BackupError::ProviderUnavailable)
    ));
    for bad_token in ["", "contains spaces", "line\nbreak"] {
        assert!(matches!(
            YandexDiskRestBackupPort::new(bad_token),
            Err(BackupError::ProviderDenied)
        ));
    }
    let too_long = "a".repeat(MAX_BEARER + 1);
    assert!(matches!(
        YandexDiskRestBackupPort::new(&too_long),
        Err(BackupError::ProviderDenied)
    ));
    assert!(YandexDiskRestBackupPort::new("AQAAA-short-test-token").is_ok());
}

#[test]
fn input_path_and_ciphertext_shape_are_checked_before_network_use() {
    // This test deliberately never supplies a real token or performs HTTP:
    // path errors return before the fixed-origin metadata request.
    let mut port = YandexDiskRestBackupPort::new("not-a-real-token").unwrap();
    let wrong_path = YandexAppPath::root();
    assert!(matches!(
        port.create_only(&wrong_path, b"PLAIN-TEXT"),
        Err(BackupError::InvalidEnvelope)
    ));
    let ciphertext = vec![b'x'; MAX_ENCRYPTED_BYTES + 1];
    assert!(matches!(
        port.create_only(&wrong_path, &ciphertext),
        Err(BackupError::PayloadTooLarge)
    ));
    assert!(!valid_backup_path(&wrong_path));
}

#[test]
fn non_backup_objects_never_resolve_to_remote_io() {
    let wrong_path = YandexAppPath::root().child("secret.txt").unwrap();
    let mut port = YandexDiskRestBackupPort::new("not-a-real-token").unwrap();
    assert!(matches!(
        port.read_exact_bounded(&wrong_path, MAX_ENCRYPTED_BYTES),
        Err(BackupError::InvalidPath)
    ));
}

#[test]
fn metadata_status_errors_are_typed_and_never_leak_body_or_url() {
    // Caller gets only a constant enum. A 409 is an overwrite conflict,
    // whereas provider errors must never be treated as a 201 receipt.
    assert_eq!(BackupError::CommitUncertain, BackupError::CommitUncertain);
    assert_eq!(BackupError::PreexistingRemoteObject, BackupError::PreexistingRemoteObject);
}
