//! One host-owned entrypoint for the S3 pairing beta. This orchestrates
//! the EXISTING S3 transport, Core read admission and no-replace receiver.
//! The Native shell must obtain credentials from the OS protected store.
//! No operation runs merely because a profile JSON is present at startup.

use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::Path};

use super::{
    AwsS3ProbeClient, RegisteredReadAdmission, S3HeadJournal,
    S3Keyspace, S3PairAccess, SyncRelativePath, VaultId, VaultPortError,
};
use super::s3_pair_bootstrap::{
    bootstrap_new_remote_vault, verify_invited_genesis, S3PairBootstrapError,
};
use super::s3_pair_invite::PairInviteError;
use super::s3_pair_profile::S3PairProfile;
use super::s3_transport::pair_upload::S3PairUploadError;
use super::s3_transport::pair_receive::S3PairReceiveError;
use crate::app_core::Core;

mod custody;
pub use custody::{
    open_existing_cooperative_s3_pair_custody,
    provision_cooperative_s3_pair_custody,
    S3PairCustodyError,
};

pub const PAIR_PROFILE_FILENAME: &str = "s3-pair-profile-v1.json";
const MAX_PROFILE_BYTES: u64 = 4096;
const MAX_SYNC_FILES: usize = 128;

#[derive(Debug)]
pub enum S3PairHostError {
    InvalidLocation,
    ExistingDifferentProfile,
    InvalidProfile(PairInviteError),
    Bootstrap(S3PairBootstrapError),
    Connect(super::S3TransportError),
    Journal(super::S3HeadError),
    Admission(VaultPortError),
    Upload(S3PairUploadError),
    Receive(S3PairReceiveError),
    Binding(super::profile_binding::S3BindingError),
    Io(std::io::Error),
}

impl From<PairInviteError> for S3PairHostError {
    fn from(e: PairInviteError) -> Self { Self::InvalidProfile(e) }
}
impl From<S3PairBootstrapError> for S3PairHostError {
    fn from(e: S3PairBootstrapError) -> Self { Self::Bootstrap(e) }
}
impl From<super::S3TransportError> for S3PairHostError {
    fn from(e: super::S3TransportError) -> Self { Self::Connect(e) }
}
impl From<super::S3HeadError> for S3PairHostError {
    fn from(e: super::S3HeadError) -> Self { Self::Journal(e) }
}
impl From<VaultPortError> for S3PairHostError {
    fn from(e: VaultPortError) -> Self { Self::Admission(e) }
}
impl From<super::profile_binding::S3BindingError> for S3PairHostError {
    fn from(e: super::profile_binding::S3BindingError) -> Self { Self::Binding(e) }
}
impl From<S3PairUploadError> for S3PairHostError {
    fn from(e: S3PairUploadError) -> Self { Self::Upload(e) }
}
impl From<S3PairReceiveError> for S3PairHostError {
    fn from(e: S3PairReceiveError) -> Self { Self::Receive(e) }
}
impl From<std::io::Error> for S3PairHostError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct S3PairSyncOnceReport {
    pub uploaded_new: usize,
    pub received_unconfirmed: usize,
    pub already_in_remote: usize,
    pub needs_review: usize,
}

fn safe_appdata(root: &Path, appdata: &Path) -> Result<(), S3PairHostError> {
    let root = fs::canonicalize(root)?;
    let data = fs::canonicalize(appdata)?;
    if !root.is_dir() || !data.is_dir()
        || data.starts_with(&root) || root.starts_with(&data)
    {
        return Err(S3PairHostError::InvalidLocation);
    }
    Ok(())
}

/// Strict local non-secret record. A JSON file is never a cloud trust
/// authority: every use independently verifies the encrypted invitation pin.
pub fn read_pair_profile(
    root: &Path,
    appdata: &Path,
) -> Result<Option<S3PairProfile>, S3PairHostError> {
    safe_appdata(root, appdata)?;
    let path = appdata.join(PAIR_PROFILE_FILENAME);
    let file = match OpenOptions::new().read(true).open(&path) {
        Ok(f) => f,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_PROFILE_BYTES {
        return Err(S3PairHostError::InvalidLocation);
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(S3PairHostError::InvalidLocation);
    }
    let profile: S3PairProfile = serde_json::from_slice(&bytes)
        .map_err(|_| S3PairHostError::InvalidLocation)?;
    profile.validate()?;
    Ok(Some(profile))
}

pub fn store_new_pair_profile(
    root: &Path,
    appdata: &Path,
    profile: &S3PairProfile,
) -> Result<(), S3PairHostError> {
    safe_appdata(root, appdata)?;
    profile.validate()?;
    if let Some(previous) = read_pair_profile(root, appdata)? {
        return if previous == *profile { Ok(()) }
            else { Err(S3PairHostError::ExistingDifferentProfile) };
    }
    let bytes = serde_json::to_vec(profile)
        .map_err(|_| S3PairHostError::InvalidLocation)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(S3PairHostError::InvalidLocation);
    }
    let path = appdata.join(PAIR_PROFILE_FILENAME);
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    fs::File::open(appdata)?.sync_all()?;
    Ok(())
}

/// First device. A missing S3 root is NOT implicitly trusted; this explicit
/// operation may create genesis only after the host has proved write access.
pub async fn create_first_pair_profile(
    core: &Core,
    root: &Path,
    appdata: &Path,
    bucket: &str,
    access_id: &str,
    access_secret: &str,
) -> Result<S3PairProfile, S3PairHostError> {
    safe_appdata(root, appdata)?;
    RegisteredReadAdmission::open(core, root)?;
    // A failed earlier setup may have committed the remote genesis and
    // profile before Native could install its OS-owned File Gate lock or
    // profile-scoped WinCred entry. Re-check the SAME registered cloud root
    // instead of creating another one (and never capture a foreign root).
    if let Some(existing) = read_pair_profile(root, appdata)? {
        if existing.provider != "yandex" || existing.bucket != bucket {
            return Err(S3PairHostError::ExistingDifferentProfile);
        }
        let ns = existing.keyspace()?;
        let conn = existing.connection()?;
        let client = AwsS3ProbeClient::connect_with_explicit_credentials(
            conn.clone(), ns.clone(), access_id, access_secret,
        ).await?;
        let verified = verify_invited_genesis(
            &client, &ns, &existing.genesis_fingerprint,
        ).await?;
        let admission = RegisteredReadAdmission::open(core, root)?;
        let mut bindings = super::profile_binding::SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("s3-profiles.sqlite"),
        )?;
        bindings.bind_existing_verified_s3(
            &admission, "akula-pair-v1", &conn, &ns, &verified,
        )?;
        return Ok(existing);
    }
    let vault = VaultId::new();
    let profile = S3PairProfile::yandex_first_device(bucket, vault, "0".repeat(64));
    let connection = profile.connection()?;
    let ns = profile.keyspace()?;
    let client = AwsS3ProbeClient::connect_with_explicit_credentials(
        connection.clone(), ns.clone(), access_id, access_secret,
    ).await?;
    if client.probe_pair_read_write().await? != S3PairAccess::ReadWrite {
        return Err(S3PairHostError::Connect(super::S3TransportError::PermissionDenied));
    }
    let mut journal = S3HeadJournal::open(&appdata.join("s3-head.sqlite"))?;
    let trusted = bootstrap_new_remote_vault(&client, &ns, &mut journal).await?;
    let accepted = S3PairProfile::yandex_first_device(
        bucket, trusted.cloud_vault_id, trusted.genesis_fingerprint,
    );
    // First and second PCs must be bound to the same cloud VaultId while
    // retaining distinct local workspace UUIDs and independent SQLite DBs.
    let verified = verify_invited_genesis(
        &client, &ns, &accepted.genesis_fingerprint,
    ).await?;
    let admission = RegisteredReadAdmission::open(core, root)?;
    let mut bindings = super::profile_binding::SqliteRegisteredS3Bindings::open(
        &admission, &appdata.join("s3-profiles.sqlite"),
    )?;
    bindings.bind_existing_verified_s3(
        &admission, "akula-pair-v1", &connection, &ns, &verified,
    )?;
    store_new_pair_profile(root, appdata, &accepted)?;
    Ok(accepted)
}

/// Read-only remote identity preflight BEFORE registering any local
/// workspace or mutating its SQLite/credential records. The imported pin is
/// merely a trust-on-first-share assertion; it is NOT a signing certificate.
pub async fn verify_second_pair_remote(
    profile: &S3PairProfile,
    access_id: &str,
    access_secret: &str,
) -> Result<(), S3PairHostError> {
    profile.validate()?;
    let ns = profile.keyspace()?;
    let connection = profile.connection()?;
    let client = AwsS3ProbeClient::connect_with_explicit_credentials(
        connection, ns.clone(), access_id, access_secret,
    ).await?;
    verify_invited_genesis(&client, &ns, &profile.genesis_fingerprint).await?;
    Ok(())
}

/// Return true only for a previous import bound to this EXACT local
/// registered root and the same selected remote scope. Allows a harmless
/// retry when the directory now contains verified user notes; never grants
/// permission to attach an unrelated non-empty directory.
pub fn already_imported_on_registered_root(
    core: &Core,
    root: &Path,
    appdata: &Path,
    profile: &S3PairProfile,
) -> bool {
    if read_pair_profile(root, appdata).ok().flatten().as_ref() != Some(profile) {
        return false;
    }
    let Ok(admission) = RegisteredReadAdmission::open(core, root) else {
        return false;
    };
    let Ok(store) = super::profile_binding::SqliteRegisteredS3Bindings
        ::open_existing_read_only(&admission, &appdata.join("s3-profiles.sqlite"))
    else {
        return false;
    };
    let Ok(Some(binding)) = store.load_for_registered(&admission) else {
        return false;
    };
    let (Ok(connection), Ok(namespace)) = (profile.connection(), profile.keyspace()) else {
        return false;
    };
    binding.matches_remote(&connection, &namespace)
}

/// Second device. The encrypted invitation carries the genesis trust pin.
/// Bind ONLY to a registered local workspace after verifying the full remote
/// ancestry; never treat an arbitrary user-provided URL as authenticated.
pub async fn accept_second_pair_profile(
    core: &Core,
    root: &Path,
    appdata: &Path,
    profile: &S3PairProfile,
    access_id: &str,
    access_secret: &str,
) -> Result<(), S3PairHostError> {
    safe_appdata(root, appdata)?;
    profile.validate()?;
    // A partial import can be resumed for the same bound profile, but never
    // rebind this installation's recorded local profile to another root.
    if let Some(previous) = read_pair_profile(root, appdata)? {
        if previous != *profile {
            return Err(S3PairHostError::ExistingDifferentProfile);
        }
    }
    let ns = profile.keyspace()?;
    let conn = profile.connection()?;
    let client = AwsS3ProbeClient::connect_with_explicit_credentials(
        conn.clone(), ns.clone(), access_id, access_secret,
    ).await?;
    let trusted = verify_invited_genesis(
        &client, &ns, &profile.genesis_fingerprint,
    ).await?;
    let admission = RegisteredReadAdmission::open(core, root)?;
    let mut bindings = super::profile_binding::SqliteRegisteredS3Bindings::open(
        &admission, &appdata.join("s3-profiles.sqlite"),
    )?;
    bindings.bind_existing_verified_s3(
        &admission, "akula-pair-v1", &conn, &ns, &trusted,
    )?;
    store_new_pair_profile(root, appdata, profile)?;
    Ok(())
}

/// One full user action: upload local top-level new Markdown notes, then
/// download distinct cloud additions through the existing host File Gate.
/// When the installer has not provisioned trusted external lock custody,
/// outgoing additions may succeed, but RECEIVE stays HOLD; no bypass.
pub async fn sync_new_files_once(
    core: &Core,
    root: &Path,
    appdata: &Path,
    external_custody_scope: Option<&Path>,
    profile: &S3PairProfile,
    access_id: &str,
    access_secret: &str,
) -> Result<S3PairSyncOnceReport, S3PairHostError> {
    safe_appdata(root, appdata)?;
    profile.validate()?;
    let ns: S3Keyspace = profile.keyspace()?;
    let connection = profile.connection()?;
    let client = AwsS3ProbeClient::connect_with_explicit_credentials(
        connection.clone(), ns.clone(), access_id, access_secret,
    ).await?;
    verify_invited_genesis(&client, &ns, &profile.genesis_fingerprint).await?;
    let mut cloud_journal = S3HeadJournal::open(&appdata.join("s3-head.sqlite"))?;
    let mut report = S3PairSyncOnceReport::default();
    let mut names = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        if !item.file_type()?.is_file() { continue; }
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            report.needs_review += 1;
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".md") { continue; }
        let path = match SyncRelativePath::parse(name) {
            Ok(v) => v,
            Err(_) => {
                report.needs_review += 1;
                continue;
            }
        };
        names.push(path);
    }
    if names.len() > MAX_SYNC_FILES {
        return Err(S3PairHostError::InvalidLocation);
    }
    for path in names {
        match client.publish_new_registered_s3_file(
            core, root, path, &ns, &profile.genesis_fingerprint,
            &mut cloud_journal,
        ).await {
            Ok(_) => report.uploaded_new += 1,
            Err(S3PairUploadError::AlreadyPublishedSame) => report.already_in_remote += 1,
            Err(S3PairUploadError::ExistingPath) => report.needs_review += 1,
            Err(S3PairUploadError::Local(VaultPortError::DirtyEditorBuffer)) |
            Err(S3PairUploadError::ChangedLocalFile) => report.needs_review += 1,
            Err(error) => return Err(error.into()),
        }
    }
    let Some(scope) = external_custody_scope else {
        // No false \"all synced\" badge; the installed-host setup remains P0.
        report.needs_review += 1;
        return Ok(report);
    };
    let incoming = client.receive_new_registered_s3_files(
        core, root, appdata, scope, "akula-pair-v1",
        &connection, &ns, &profile.genesis_fingerprint,
    ).await?;
    report.received_unconfirmed = incoming.created_unconfirmed;
    report.needs_review += incoming.needs_review;
    Ok(report)
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
