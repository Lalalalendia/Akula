//! Public contract smoke: compiles using the core crate, not its private test modules.
use aquilum_core::sync::{
    FakeRemoteStore, RemotePrecondition, RemoteStore, RemoteStoreError, SyncRelativePath,
    SYNC_ENGINE_FEATURE_ENABLED,
};

#[test]
fn no_runtime_io_or_network_is_activated_in_the_default_build() {
    #[cfg(not(feature = "sync-engine"))]
    assert!(!SYNC_ENGINE_FEATURE_ENABLED);
    #[cfg(feature = "sync-engine")]
    assert!(SYNC_ENGINE_FEATURE_ENABLED);
}

#[test]
fn public_fake_remote_maintains_strong_preconditions() {
    let local = SyncRelativePath::parse("Заметки/Идея.md").unwrap();
    let mut provider = FakeRemoteStore::new();
    assert!(provider.capabilities().supports_guarded_mutation());

    let first = provider.put(local.as_str(), b"v1", &RemotePrecondition::Absent).unwrap();
    assert_eq!(provider.read(local.as_str()).unwrap(), b"v1");

    assert_eq!(
        provider.delete(local.as_str(), &RemotePrecondition::Matches("stale".into())),
        Err(RemoteStoreError::PreconditionFailed)
    );
    provider
        .delete(local.as_str(), &RemotePrecondition::Matches(first.revision))
        .unwrap();
    assert_eq!(provider.read(local.as_str()), Err(RemoteStoreError::NotFound));
}

#[test]
fn missing_objects_and_incomplete_pagination_do_not_look_identical() {
    let mut provider = FakeRemoteStore::new();
    assert!(provider.list_page(None, 100).unwrap().is_final_verified_page());
    provider.set_incomplete_listing(true);
    assert_eq!(
        provider.list_page(None, 100),
        Err(RemoteStoreError::IncompleteListing)
    );
}
