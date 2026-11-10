use super::*;
#[test]
fn cloud_tiles_never_confuse_stored_keys_with_working_sync() {
    let mut ctx = Context::new(1.0);
    let config = AppConfig::default();
    assert_eq!(health(CloudProvider::Yandex, &config, &ctx), Health::NotConnected);
    assert_eq!(health(CloudProvider::AmazonS3, &config, &ctx), Health::Unavailable);
    ctx.yandex_keys = Some(true);
    #[cfg(not(all(target_os = "windows", feature = "s3-pair-beta")))]
    assert_eq!(health(CloudProvider::Yandex, &config, &ctx), Health::NotConnected);
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    {
        use crate::s3_pair_session::PairStatus;
        assert_eq!(health(CloudProvider::Yandex, &config, &ctx), Health::Saved);
        ctx.s3_pair.configured = true;
        ctx.s3_pair.status = PairStatus::Configured;
        ctx.s3_pair.verified = false;
        assert_ne!(health(CloudProvider::Yandex, &config, &ctx), Health::Verified);
        ctx.s3_pair.verified = true;
        assert_eq!(health(CloudProvider::Yandex, &config, &ctx), Health::Verified);
        ctx.s3_pair.status = PairStatus::Transferred {
            sent: 1, received_unconfirmed: 1, needs_review: 0,
        };
        assert_eq!(health(CloudProvider::Yandex, &config, &ctx), Health::Review);
        ctx.s3_pair.status = PairStatus::Failed;
        assert_eq!(health(CloudProvider::Yandex, &config, &ctx), Health::Review);
    }
}

#[test]
fn every_provider_is_navigable_and_only_supported_providers_have_forms() {
    let config = AppConfig::default();
    let ctx = Context::new(1.0);
    let (_, bindings) = catalog(&config, &ctx);
    assert_eq!(bindings.len(), CloudProvider::ALL.len());
    for provider in CloudProvider::ALL {
        assert!(bindings.values().any(|b|
            matches!(b, Binding::Button(Action::OpenCloudProvider(p)) if *p == provider)));
        let built = card(&config, &ctx, provider, &[]);
        let supported = provider == CloudProvider::Yandex
            || (cfg!(feature = "dropbox-readonly-preview") && provider == CloudProvider::Dropbox);
        #[cfg(feature = "notion-offline-preview")]
        if provider == CloudProvider::Notion {
            assert!(built.save.is_none());
            assert!(built.bindings.values().any(|b| matches!(b,
                Binding::Button(Action::NotionConnect))));
            continue;
        }
        assert_eq!(built.save.is_some(), supported);
        assert_eq!(!built.bindings.is_empty(), supported);
    }
}

#[cfg(feature = "dropbox-readonly-preview")]
#[test]
fn dropbox_tile_never_confuses_saved_account_with_trusted_sync() {
    let mut ctx = Context::new(1.0);
    let mut config = AppConfig::default();
    assert_eq!(health(CloudProvider::Dropbox, &config, &ctx), Health::NotConnected);
    config.dropbox_readonly.account_id = "dbid:tester".into();
    assert_eq!(health(CloudProvider::Dropbox, &config, &ctx), Health::Saved);
    ctx.dropbox_preview_status = "ACCOUNT_AUTHORIZED / VAULT_NOT_APPROVED / SYNC_DISABLED".into();
    assert_eq!(health(CloudProvider::Dropbox, &config, &ctx), Health::Saved);
    ctx.dropbox_preview_status = "REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED".into();
    assert_eq!(health(CloudProvider::Dropbox, &config, &ctx), Health::Review);
    ctx.dropbox_preview_status = "DROPBOX_SAVED_CONSENT_REVOKED / SYNC_DISABLED".into();
    assert_eq!(health(CloudProvider::Dropbox, &config, &ctx), Health::Review);
    ctx.dropbox_preview_busy = true;
    assert_eq!(health(CloudProvider::Dropbox, &config, &ctx), Health::Checking);
}
#[cfg(feature = "dropbox-readonly-preview")]
#[test]
fn dropbox_tile_opens_real_oauth_actions_in_modal() {
    let cfg = AppConfig::default();
    let ctx = Context::new(1.0);
    let built = card(&cfg, &ctx, CloudProvider::Dropbox, &[]);
    assert!(built.save.is_some());
    assert!(built.bindings.values().any(|binding|
        matches!(binding, Binding::Button(Action::DropboxLoginAccount))));
    assert!(built.bindings.values().any(|binding|
        matches!(binding, Binding::Button(Action::DropboxResumeAccount))));
    assert!(built.bindings.values().any(|binding|
        matches!(binding, Binding::Button(Action::DropboxForgetAccount))));
}

#[cfg(feature = "dropbox-readonly-preview")]
#[test]
fn switching_dropbox_account_discards_staged_vault_pin() {
    let mut config = AppConfig::default();
    config.dropbox_readonly.account_id = "dbid:alice".into();
    config.dropbox_readonly.root_namespace_id = "123".into();
    let ctx = Context::new(1.0);
    let (_, mut popup) = provider_modal(&config, &ctx, CloudProvider::Dropbox);
    let block = schema::blocks(Section::Sync, &config, &ctx).pop().unwrap();
    let setter = block.rows.iter()
        .find(|row| row.label == "Контрольный BLAKE3 HEAD")
        .and_then(|row| row.setter).unwrap();
    popup.remember_text(setter, &"a".repeat(64));
    popup.discard_draft_if_dropbox_account_changed(&config);
    assert!(popup.has_draft_changes(), "same account preserves the draft");

    config.dropbox_readonly.account_id = "dbid:bob".into();
    config.dropbox_readonly.root_namespace_id = "456".into();
    popup.discard_draft_if_dropbox_account_changed(&config);
    assert!(!popup.has_draft_changes(), "old account's pin must be discarded");
    assert_eq!(popup.dropbox_identity.0, "dbid:bob");
    assert_eq!(popup.dropbox_identity.1, "456");
}
