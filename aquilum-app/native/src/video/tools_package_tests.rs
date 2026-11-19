use super::*;
use serde_json::json;

#[test]
fn publisher_key_is_not_authorized_by_video_lab_or_stock_updater() {
    assert_eq!(APPROVED_VIDEO_TOOLS_KEY, "");
    assert!(!shipping_admission_enabled());
    assert!(verify_signed(Path::new("/not/a/tools/package"), b"{}", "fake", "")
        .unwrap_err().contains("not approved"));
}

#[test]
fn exact_names_version_and_manifest_schema_reject_ambiguous_payloads() {
    let root = tempfile::tempdir().unwrap();
    let raw = || json!({
        "schema": 1, "product": "Akula Video Tools", "platform": "windows-x86_64",
        "version": "0.1.0", "source_commit": "a".repeat(40),
        "cargo_lock_sha256": "b".repeat(64),
        "binaries": NAMES.iter().map(|name| json!({
            "name": name, "size": 64, "sha256": "c".repeat(64)
        })).collect::<Vec<_>>()
    });
    let changed = [
        ("schema", json!(2)), ("product", json!("Akula")),
        ("platform", json!("linux-x86_64")),
        ("version", json!("0.01.0")), ("source_commit", json!("not-a-commit")),
        ("cargo_lock_sha256", json!("d")),
    ];
    for (key, value) in changed {
        let mut document = raw();
        document[key] = value;
        let error = inspect_authenticated(root.path(), &serde_json::to_vec(&document).unwrap())
            .unwrap_err();
        assert!(error.contains("identity"), "{key}: {error}");
    }
    for changed in ["akula-app.exe", "../akula-video-mux.exe", "akula-yt-dlp.exe"] {
        let mut document = raw();
        document["binaries"][0]["name"] = json!(changed);
        let error = inspect_authenticated(root.path(), &serde_json::to_vec(&document).unwrap())
            .unwrap_err();
        assert!(error.contains("wrong signed component name"), "{changed}: {error}");
    }
    let mut document = raw();
    document["unexpected"] = json!(true);
    assert!(inspect_authenticated(root.path(), &serde_json::to_vec(&document).unwrap())
        .unwrap_err().contains("unknown-field"));
    assert!(!version("0.0.9")); // stale standalone package below the approved floor
    assert!(!version("1.2"));
    assert!(!version("1.2.3.4"));
    assert!(!version("1.2.-3"));
    assert!(!version("1.2.4294967296"));
}

#[test]
#[ignore = "CI supplies independent, real ephemeral Minisign signatures and a synthetic three-file fixture"]
fn ephemeral_minisign_authenticates_exact_three_component_bytes() {
    let root = std::path::PathBuf::from(
        std::env::var("AKULA_VIDEO_TOOLS_TEST_ROOT")
            .expect("ephemeral tools root must be set by the signed PR acceptance step"),
    );
    let package = root.join("package");
    let raw = fs::read(package.join("manifest.json")).unwrap();
    let sig = fs::read_to_string(package.join("manifest.minisig")).unwrap();
    let key = fs::read_to_string(root.join("publisher.pub")).unwrap();
    let wrong_key = fs::read_to_string(root.join("other.pub")).unwrap();

    let attested = verify_signed(&package, &raw, &sig, &key)
        .expect("real Minisign-signed, pinned three-file package must pass");
    assert_eq!(attested.version, "0.1.0");
    assert_eq!(attested.digests.len(), 3);
    assert!(attested.digests.iter().all(|digest| hex(digest, 64)));

    assert!(verify_signed(&package, &raw, &sig, &wrong_key)
        .unwrap_err().contains("signature failed"));
    let mut changed_raw = raw.clone();
    changed_raw.push(b' ');
    assert!(verify_signed(&package, &changed_raw, &sig, &key)
        .unwrap_err().contains("signature failed"));

    let exe = package.join(NAMES[1]);
    let original = fs::read(&exe).unwrap();
    let mut tampered = original.clone();
    tampered[4] ^= 1;
    fs::write(&exe, tampered).unwrap();
    assert!(verify_signed(&package, &raw, &sig, &key)
        .unwrap_err().contains("hash differs"));
    fs::write(&exe, original).unwrap();

    let alien = package.join("akula-app.exe");
    fs::write(&alien, b"this is a forbidden fourth executable").unwrap();
    assert!(verify_signed(&package, &raw, &sig, &key)
        .unwrap_err().contains("unknown file"));
    fs::remove_file(&alien).unwrap();

    let mut altered_on_disk = raw.clone();
    altered_on_disk[2] ^= 1;
    fs::write(package.join("manifest.json"), altered_on_disk).unwrap();
    assert!(verify_signed(&package, &raw, &sig, &key)
        .unwrap_err().contains("disk metadata differs"));
    fs::write(package.join("manifest.json"), raw.clone()).unwrap();

    fs::write(package.join("manifest.json"), vec![b'X'; MAX_MANIFEST + 1]).unwrap();
    assert!(verify_signed(&package, &raw, &sig, &key)
        .unwrap_err().contains("oversized"));
    fs::write(package.join("manifest.json"), raw.clone()).unwrap();

    assert!(verify_signed(&package, &raw, &sig, &key).is_ok());
    println!("VIDEO_TOOLS_01B_REAL_MINISIGN_PASS signed_manifest=1 wrong_key=0 tampered_manifest=0 altered_exe=0 fourth_exe=0 release=0");
}

#[cfg(windows)]
#[test]
fn native_tools_discovery_never_authorizes_without_publisher() {
    assert_ne!(probe_installed_video_tools(), ToolsDiscovery::ReadOnlyAttested);
    assert!(!shipping_admission_enabled());
    assert!(APPROVED_VIDEO_TOOLS_KEY.is_empty());
}
