use super::*;
use crate::settings::models::AppConfig;

const V1: &str = "12345678-1234-1234-1234-123456789abc";
const V2: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const APP: &str = "4760187d81bc4b7799476b42b5103713";
const PROFILE: &str = "personal_disk";

fn entry(account: &str, workspace: &str) -> YandexDiskPublicBinding {
    YandexDiskPublicBinding::new(account, APP, workspace, PROFILE).unwrap()
}

#[test]
fn old_settings_default_to_no_account_and_no_sync_authority() {
    let config: AppConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(config.yandex_disk_public.len(), 0);
    assert!(config.yandex_disk_public.is_well_formed());
    assert!(config.yandex_disk_public.unverified_for(V1, APP, PROFILE).is_none());
}

#[test]
fn only_public_identifiers_round_trip_through_settings_json() {
    let mut config = AppConfig::default();
    config.yandex_disk_public.remember_observed(entry("1000034426", V1)).unwrap();
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains("1000034426"));
    assert!(serialized.contains("yandexDiskPublic"));
    for secret in ["refreshToken", "accessToken", "clientSecret",
        "authorizationCode", "codeVerifier", "osCredential"]
    {
        assert!(!serialized.contains(secret), "secret field {secret} must not persist");
    }
    let restored: AppConfig = serde_json::from_str(&serialized).unwrap();
    let binding = restored.yandex_disk_public.unverified_for(V1, APP, PROFILE).unwrap();
    assert_eq!(binding.account_id(), "1000034426");
    assert_eq!(binding.workspace_uuid(), V1);
    assert_eq!(binding.oauth_client_id(), APP);
    assert_eq!(binding.profile_id(), PROFILE);
}

#[test]
fn unknown_token_fields_in_saved_json_do_not_survive_reserialization() {
    let malicious = serde_json::json!({
        "yandexDiskPublic": {
            "accounts": [{
                "accountId": "1000034426",
                "oauthClientId": APP,
                "workspaceUuid": V1,
                "profileId": PROFILE,
                "refreshToken": "LEAK_ME",
                "clientSecret": "ALSO_LEAK_ME"
            }]
        }
    });
    let restored: AppConfig = serde_json::from_value(malicious).unwrap();
    assert!(restored.yandex_disk_public.unverified_for(V1, APP, PROFILE).is_some());
    let encoded = serde_json::to_string(&restored).unwrap();
    assert!(!encoded.contains("LEAK_ME"));
}

#[test]
fn public_settings_do_not_alias_other_vault_or_oauth_app() {
    let mut collection = YandexDiskPublicAccounts::default();
    collection.remember_observed(entry("1000034426", V1)).unwrap();
    collection.remember_observed(entry("1000034427", V2)).unwrap();
    assert_eq!(collection.len(), 2);
    assert!(collection.unverified_for(V1, APP, PROFILE).is_some());
    assert!(collection.unverified_for(V2, APP, PROFILE).is_some());
    assert!(collection.unverified_for(V2, "differentApp", PROFILE).is_none());
    assert!(collection.unverified_for(V1, APP, "other").is_none());
    assert!(collection.unverified_for("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", APP, PROFILE).is_none());
}

#[test]
fn cannot_silently_replace_account_for_existing_workspace_and_profile() {
    let mut collection = YandexDiskPublicAccounts::default();
    let first = entry("1000034426", V1);
    collection.remember_observed(first.clone()).unwrap();
    assert_eq!(collection.remember_observed(first.clone()), Ok(()));
    let conflict = entry("1000034427", V1);
    assert_eq!(
        collection.remember_observed(conflict),
        Err(YandexDiskPublicError::ConflictingAccount)
    );
    assert_eq!(collection.len(), 1);
    assert_eq!(collection.unverified_for(V1, APP, PROFILE), Some(&first));
    assert_eq!(collection.forget_exact(&first), Ok(true));
    assert_eq!(collection.forget_exact(&first), Ok(false));
    assert_eq!(collection.len(), 0);
}

#[test]
fn invalid_identifiers_are_rejected_before_reaching_os_keyring_scope() {
    for (account, app, vault, profile) in [
        ("", APP, V1, PROFILE),
        ("alice", APP, V1, PROFILE),
        ("123", "https://bad.example", V1, PROFILE),
        ("123", APP, "../vault", PROFILE),
        ("123", APP, "12345678-1234-1234-1234-123456789ABC", PROFILE),
        ("123", APP, V1, ""),
        ("123", APP, V1, "../profile"),
    ] {
        assert_eq!(
            YandexDiskPublicBinding::new(account, app, vault, profile),
            Err(YandexDiskPublicError::InvalidRecord)
        );
    }
}

#[test]
fn invalid_or_ambiguous_loaded_records_never_resolve_as_active_hint() {
    let forged = serde_json::json!({
        "accounts": [{
            "accountId": "alice",
            "oauthClientId": APP,
            "workspaceUuid": V1,
            "profileId": PROFILE
        }]
    });
    let invalid: YandexDiskPublicAccounts = serde_json::from_value(forged).unwrap();
    assert!(!invalid.is_well_formed());
    assert!(invalid.unverified_for(V1, APP, PROFILE).is_none());
    assert_eq!(invalid.clone().forget_exact(&entry("1000034426", V1)),
        Err(YandexDiskPublicError::InvalidStoredCollection));

    let duplicate: YandexDiskPublicAccounts = serde_json::from_value(serde_json::json!({
        "accounts": [{
            "accountId":"1000034426", "oauthClientId":APP,
            "workspaceUuid":V1, "profileId":PROFILE
        },{
            "accountId":"1000034427", "oauthClientId":APP,
            "workspaceUuid":V1, "profileId":PROFILE
        }]
    })).unwrap();
    assert!(!duplicate.is_well_formed());
    assert!(duplicate.unverified_for(V1, APP, PROFILE).is_none());
}

#[test]
fn exact_forget_preserves_other_workspace_profile_and_app() {
    let mut collection = YandexDiskPublicAccounts::default();
    let own = entry("1000034426", V1);
    let other = entry("1000034427", V2);
    collection.remember_observed(own.clone()).unwrap();
    collection.remember_observed(other.clone()).unwrap();
    assert_eq!(collection.forget_exact(&own), Ok(true));
    assert!(collection.unverified_for(V1, APP, PROFILE).is_none());
    assert_eq!(collection.unverified_for(V2, APP, PROFILE), Some(&other));
}

#[test]
fn collection_is_bounded_and_cannot_evict_other_accounts() {
    let mut collection = YandexDiskPublicAccounts::default();
    for i in 0..MAX_BINDINGS {
        let uuid = format!("12345678-1234-1234-1234-{i:012x}");
        collection.remember_observed(entry(&format!("10000344{i:02}"), &uuid)).unwrap();
    }
    assert_eq!(collection.len(), MAX_BINDINGS);
    assert_eq!(collection.remember_observed(entry("99999", V2)),
        Err(YandexDiskPublicError::TooManyRecords));
    assert_eq!(collection.len(), MAX_BINDINGS);
}
