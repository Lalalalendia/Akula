use super::*;
use std::collections::HashMap;

#[derive(Default)]
struct FakeTransport {
    objects: HashMap<String, Vec<u8>>,
    call_count: usize,
    reads: usize,
    acknowledgments: Option<UploadAcknowledgment>,
    mode: FakeMode,
}

#[derive(Clone, Copy, Default)]
enum FakeMode {
    #[default]
    Normal,
    Delayed202,
    AcceptedThenLostResponse,
    LostWithoutCommit,
    CorruptAfterUpload,
    OmitReadback,
    OversizedReadback,
    Denied,
}

impl FakeTransport {
    fn new(mode: FakeMode) -> Self {
        Self { mode, ..Default::default() }
    }
}

impl CreateOnlyBackupPort for FakeTransport {
    fn create_only(
        &mut self, path: &YandexAppPath, ciphertext: &[u8],
    ) -> Result<UploadAcknowledgment, BackupError> {
        self.call_count += 1;
        let key = path.as_str().to_owned();
        if self.objects.contains_key(&key) {
            return Err(BackupError::PreexistingRemoteObject);
        }
        if matches!(self.mode, FakeMode::Denied) {
            return Err(BackupError::ProviderDenied);
        }
        if matches!(self.mode, FakeMode::LostWithoutCommit) {
            return Err(BackupError::CommitUncertain);
        }
        let mut saved = ciphertext.to_vec();
        if matches!(self.mode, FakeMode::CorruptAfterUpload) {
            saved[saved.len()-1] ^= 1;
        }
        self.objects.insert(key, saved);
        if matches!(self.mode, FakeMode::AcceptedThenLostResponse) {
            return Err(BackupError::CommitUncertain);
        }
        let ack = if matches!(self.mode, FakeMode::Delayed202) {
            UploadAcknowledgment::Pending202
        } else {
            self.acknowledgments.unwrap_or(UploadAcknowledgment::Created201)
        };
        Ok(ack)
    }

    fn read_exact_bounded(
        &mut self, path: &YandexAppPath, max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, BackupError> {
        self.reads += 1;
        if matches!(self.mode, FakeMode::OmitReadback | FakeMode::Delayed202) {
            return Ok(None);
        }
        if matches!(self.mode, FakeMode::OversizedReadback) {
            return Err(BackupError::RemoteObjectTooLarge);
        }
        let data = self.objects.get(path.as_str()).cloned();
        if data.as_ref().is_some_and(|v| v.len() > max_bytes) {
            return Err(BackupError::RemoteObjectTooLarge);
        }
        Ok(data)
    }
}

fn archive() -> (RecoveryKey, SealedBackup, Vec<u8>) {
    let key = RecoveryKey::generate().unwrap();
    let plain = b"Internal notes, no plaintext remote upload. Test string.".to_vec();
    let sealed = SealedBackup::seal(&key, &plain).unwrap();
    (key, sealed, plain)
}

#[test]
fn fresh_aes_gcm_archive_roundtrips_only_with_correct_recovery_key_and_path() {
    let (key, sealed, plain) = archive();
    assert!(sealed.path().as_str().starts_with("app:/akula-backup-v1-"));
    assert!(sealed.path().as_str().ends_with(".akulaenc"));
    assert!(sealed.encrypted_len() > plain.len());
    assert!(!sealed.encrypted_bytes().windows(plain.len()).any(|w| w == plain));
    let recovered = SealedBackup::open_from_remote(
        &key, sealed.path(), sealed.encrypted_bytes()
    ).unwrap();
    assert_eq!(recovered.as_slice(), plain.as_slice());

    let different = RecoveryKey::generate().unwrap();
    assert!(matches!(
        SealedBackup::open_from_remote(
            &different, sealed.path(), sealed.encrypted_bytes()),
        Err(BackupError::AuthenticationFailed)
    ));

    let wrong_path = YandexAppPath::root().child(
        "akula-backup-v1-00000000000000000000000000000000.akulaenc"
    ).unwrap();
    assert!(matches!(
        SealedBackup::open_from_remote(
            &key, &wrong_path, sealed.encrypted_bytes()),
        Err(BackupError::AuthenticationFailed)
    ));
}

#[test]
fn no_duplicate_names_or_nonces_in_small_batch() {
    let key = RecoveryKey::generate().unwrap();
    let mut names = std::collections::BTreeSet::new();
    let mut sealed_bytes = std::collections::BTreeSet::new();
    for _ in 0..40 {
        let b = SealedBackup::seal(&key, b"same payload").unwrap();
        assert!(names.insert(b.path().as_str().to_owned()));
        assert!(sealed_bytes.insert(b.encrypted_bytes().to_vec()));
    }
}

#[test]
fn corrupted_data_header_nonce_or_tag_never_restores() {
    let (key, sealed, _) = archive();
    let mut bad = sealed.encrypted_bytes().to_vec();
    bad[0] ^= 1;
    assert_eq!(
        SealedBackup::open_from_remote(&key, sealed.path(), &bad).err(),
        Some(BackupError::InvalidEnvelope)
    );
    for i in [MAGIC.len(), sealed.encrypted_len()-1] {
        let mut bad = sealed.encrypted_bytes().to_vec();
        bad[i] ^= 1;
        assert_eq!(
            SealedBackup::open_from_remote(&key, sealed.path(), &bad).err(),
            Some(BackupError::AuthenticationFailed)
        );
    }
    assert_eq!(
        SealedBackup::open_from_remote(&key, &YandexAppPath::root(), sealed.encrypted_bytes()).err(),
        Some(BackupError::InvalidPath)
    );
    assert_eq!(
        SealedBackup::open_from_remote(&key, sealed.path(), &[]).err(),
        Some(BackupError::InvalidEnvelope)
    );
}

#[test]
fn archives_are_bounded_before_crypto_and_bounded_during_restore() {
    let key = RecoveryKey::generate().unwrap();
    assert_eq!(SealedBackup::seal(&key, &[]).err(), Some(BackupError::EmptyPayload));
    let large = vec![17u8; MAX_PLAINTEXT_BYTES + 1];
    assert_eq!(SealedBackup::seal(&key, &large).err(), Some(BackupError::PayloadTooLarge));
    let (_, sealed, _) = archive();
    assert_eq!(
        SealedBackup::open_from_remote(
            &key, sealed.path(), &vec![0; MAX_ENCRYPTED_BYTES + 1]).err(),
        Some(BackupError::RemoteObjectTooLarge)
    );
}

#[test]
fn created_201_still_requires_exact_complete_readback() {
    let (_, sealed, _) = archive();
    let mut provider = FakeTransport::default();
    let receipt = sealed.create_once_and_verify(&mut provider).unwrap();
    assert_eq!(receipt.path, *sealed.path());
    assert_eq!(receipt.encrypted_len, sealed.encrypted_len() as u64);
    assert_eq!(provider.call_count, 1);
    assert_eq!(provider.reads, 1);
    assert!(provider.objects.contains_key(sealed.path().as_str()));
}

#[test]
fn 202_is_pending_until_a_separate_verified_readback() {
    let (_, sealed, _) = archive();
    let mut provider = FakeTransport::new(FakeMode::Delayed202);
    assert_eq!(
        sealed.create_once_and_verify(&mut provider).err(),
        Some(BackupError::NotYetVerified)
    );
    assert_eq!(provider.call_count, 1);
    assert_eq!(provider.reads, 1);
    // Only an explicit readback is retried; the PUT never repeats.
    provider.mode = FakeMode::Normal;
    sealed.verify_existing(&mut provider).unwrap();
    assert_eq!(provider.call_count, 1);
    assert_eq!(provider.reads, 2);
}

#[test]
fn uncertain_network_outcome_must_not_retry_create_and_must_verify_bytes() {
    let (_, sealed, _) = archive();
    let mut committed = FakeTransport::new(FakeMode::AcceptedThenLostResponse);
    sealed.create_once_and_verify(&mut committed).unwrap();
    assert_eq!(committed.call_count, 1);
    let mut missing = FakeTransport::new(FakeMode::LostWithoutCommit);
    assert_eq!(
        sealed.create_once_and_verify(&mut missing).err(),
        Some(BackupError::NotYetVerified)
    );
    assert_eq!(missing.call_count, 1);
}

#[test]
fn conflict_does_not_overwrite_remote_object_or_claim_a_receipt() {
    let (_, sealed, _) = archive();
    let mut provider = FakeTransport::default();
    let first = sealed.create_once_and_verify(&mut provider).unwrap();
    let before = provider.objects.get(sealed.path().as_str()).unwrap().clone();
    assert_eq!(
        sealed.create_once_and_verify(&mut provider).err(),
        Some(BackupError::PreexistingRemoteObject)
    );
    assert_eq!(first.encrypted_len, before.len() as u64);
    assert_eq!(provider.objects.get(sealed.path().as_str()).unwrap(), &before);
    assert_eq!(provider.call_count, 2);
    // The second create never claimed an independent successful backup.
    assert_eq!(provider.reads, 1);
}

#[test]
fn 201_with_corrupted_or_unavailable_readback_does_not_pass() {
    let (_, sealed, _) = archive();
    let mut corrupted = FakeTransport::new(FakeMode::CorruptAfterUpload);
    assert_eq!(
        sealed.create_once_and_verify(&mut corrupted).err(),
        Some(BackupError::RemoteContentMismatch)
    );
    let mut invisible = FakeTransport::new(FakeMode::OmitReadback);
    assert_eq!(
        sealed.create_once_and_verify(&mut invisible).err(),
        Some(BackupError::NotYetVerified)
    );
    let mut oversize = FakeTransport::new(FakeMode::OversizedReadback);
    assert_eq!(
        sealed.create_once_and_verify(&mut oversize).err(),
        Some(BackupError::RemoteObjectTooLarge)
    );
}

#[test]
fn denied_write_stops_without_readback_or_retry() {
    let (_, sealed, _) = archive();
    let mut denied = FakeTransport::new(FakeMode::Denied);
    assert_eq!(
        sealed.create_once_and_verify(&mut denied).err(),
        Some(BackupError::ProviderDenied)
    );
    assert_eq!(denied.call_count, 1);
    assert_eq!(denied.reads, 0);
}

#[test]
fn external_plaintext_or_wrong_file_name_is_not_accepted_as_backup() {
    let (key, sealed, _) = archive();
    let foreign = YandexAppPath::root().child("notes.md").unwrap();
    assert_eq!(
        SealedBackup::open_from_remote(&key, &foreign, sealed.encrypted_bytes()).err(),
        Some(BackupError::InvalidPath)
    );
    let fake = YandexAppPath::parse(
        "app:/akula-backup-v1-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF.akulaenc"
    ).unwrap();
    assert_eq!(
        SealedBackup::open_from_remote(&key, &fake, sealed.encrypted_bytes()).err(),
        Some(BackupError::InvalidPath)
    );
}

#[test]
fn recovery_key_is_never_formatted_and_can_be_exported_explicitly() {
    let key = RecoveryKey::generate().unwrap();
    let raw = key.export_raw_bytes();
    let reconstructed = RecoveryKey::from_raw_bytes(*raw);
    let sealed = SealedBackup::seal(&key, b"restore-key").unwrap();
    let plaintext = SealedBackup::open_from_remote(
        &reconstructed, sealed.path(), sealed.encrypted_bytes()).unwrap();
    assert_eq!(plaintext.as_slice(), b"restore-key");
    // Do not add Debug to RecoveryKey or parse keys from JSON.
}
