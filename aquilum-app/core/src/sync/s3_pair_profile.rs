//! Portable NON-SECRET S3 profile selected by the host.
//! Only the protected OS credential target carries key material.

use serde::{Deserialize, Serialize};
use super::{S3ConnectionProfile, S3Keyspace, VaultId};
use super::s3_pair_invite::{S3PairInvite, PairInviteError};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct S3PairProfile {
    pub provider: String,
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub cloud_vault_id: VaultId,
    pub genesis_fingerprint: String,
}

impl S3PairProfile {
    pub fn from_authenticated_invite(invite: &S3PairInvite) -> Self {
        Self {
            provider: invite.provider.clone(),
            endpoint: invite.endpoint.clone(),
            region: invite.region.clone(),
            bucket: invite.bucket.clone(),
            prefix: invite.prefix.clone(),
            cloud_vault_id: invite.cloud_vault_id,
            genesis_fingerprint: invite.genesis_fingerprint.clone(),
        }
    }

    pub fn yandex_first_device(
        bucket: &str,
        cloud_vault_id: VaultId,
        genesis_fingerprint: String,
    ) -> Self {
        Self {
            provider: "yandex".into(),
            endpoint: Some("https://storage.yandexcloud.net".into()),
            region: "ru-central1".into(),
            bucket: bucket.to_owned(),
            prefix: "akula-sync/pair".into(),
            cloud_vault_id,
            genesis_fingerprint,
        }
    }

    pub fn keyspace(&self) -> Result<S3Keyspace, PairInviteError> {
        S3Keyspace::new(&self.prefix, self.cloud_vault_id)
            .map_err(|_| PairInviteError::InvalidScope)
    }

    pub fn connection(&self) -> Result<S3ConnectionProfile, PairInviteError> {
        let profile = S3ConnectionProfile {
            bucket: self.bucket.clone(),
            region: self.region.clone(),
            endpoint: self.endpoint.clone(),
            force_path_style: self.provider != "aws",
        };
        profile.validate().map_err(|_| PairInviteError::InvalidScope)?;
        Ok(profile)
    }

    pub fn with_keys(&self, id: &str, secret: &str)
        -> Result<S3PairInvite, PairInviteError>
    {
        let invite = S3PairInvite {
            provider: self.provider.clone(),
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            bucket: self.bucket.clone(),
            prefix: self.prefix.clone(),
            cloud_vault_id: self.cloud_vault_id,
            genesis_fingerprint: self.genesis_fingerprint.clone(),
            access_key_id: id.to_owned(),
            secret_access_key: secret.to_owned(),
            key_expires_at_ms: 0,
        };
        invite.validate()?;
        Ok(invite)
    }

    pub fn validate(&self) -> Result<(), PairInviteError> {
        self.with_keys("akula-valid-placeholder", "akula-test-valid-secret-123")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_profile_json_has_no_access_keys() {
        let profile = S3PairProfile::yandex_first_device(
            "akula-bucket", VaultId::new(), "a".repeat(64),
        );
        assert!(profile.validate().is_ok());
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("secretAccessKey"));
        assert!(!json.contains("accessKeyId"));
        assert_eq!(serde_json::from_str::<S3PairProfile>(&json).unwrap(), profile);
    }
}
