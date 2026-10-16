use super::{DeviceId, DocumentId, VaultId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self { major: PROTOCOL_MAJOR, minor: PROTOCOL_MINOR }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolCompatibility {
    SafeRewrite,
    ReadOnly,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ProtocolError {
    InvalidJson,
    UnsupportedMajor,
    UnsupportedCriticalFeature(String),
    InvalidIdentity,
    DuplicateCriticalFeature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireEnvelope {
    pub protocol: ProtocolVersion,
    #[serde(default)]
    pub required_features: Vec<String>,
    pub payload: Value,
}

impl WireEnvelope {
    pub fn compatibility(&self) -> Result<ProtocolCompatibility, ProtocolError> {
        if self.protocol.major != PROTOCOL_MAJOR {
            return Err(ProtocolError::UnsupportedMajor);
        }
        let known = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for feature in &self.required_features {
            if !seen.insert(feature.as_str()) {
                return Err(ProtocolError::DuplicateCriticalFeature);
            }
            if !known.contains(feature.as_str()) {
                return Err(ProtocolError::UnsupportedCriticalFeature(feature.clone()));
            }
        }
        Ok(if self.protocol.minor > PROTOCOL_MINOR {
            ProtocolCompatibility::ReadOnly
        } else {
            ProtocolCompatibility::SafeRewrite
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRecord {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub created_by: DeviceId,
}

pub type IdentityEnvelope = WireEnvelope;

/// Reject unknown authority fields rather than accidentally dropping them on rewrite.
/// A future minor revision remains readable as raw JSON but is never rewritten by v1.0.
pub fn decode_identity_for_write(data: &[u8]) -> Result<IdentityRecord, ProtocolError> {
    let envelope: WireEnvelope =
        serde_json::from_slice(data).map_err(|_| ProtocolError::InvalidJson)?;
    if envelope.compatibility()? != ProtocolCompatibility::SafeRewrite {
        return Err(ProtocolError::UnsupportedMajor);
    }
    serde_json::from_value(envelope.payload).map_err(|_| ProtocolError::InvalidIdentity)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncProvider {
    S3,
    YandexDisk,
    GoogleDrive,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncConfig {
    pub vault_id: VaultId,
    pub device_id: DeviceId,
    pub provider: SyncProvider,
    pub profile_id: String,
    pub encrypted_remote: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> IdentityRecord {
        IdentityRecord {
            vault_id: VaultId::new(),
            document_id: DocumentId::new(),
            created_by: DeviceId::new(),
        }
    }

    fn wire(version: ProtocolVersion, payload: Value, features: Vec<String>) -> Vec<u8> {
        serde_json::to_vec(&WireEnvelope {
            protocol: version,
            required_features: features,
            payload,
        }).unwrap()
    }

    #[test]
    fn schema_round_trip_and_known_version_is_writable() {
        let original = identity();
        let encoded = wire(ProtocolVersion::default(), serde_json::to_value(&original).unwrap(), vec![]);
        assert_eq!(decode_identity_for_write(&encoded).unwrap(), original);
    }

    #[test]
    fn future_versions_cannot_be_silently_rewritten() {
        let payload = serde_json::to_value(identity()).unwrap();
        let future_minor = wire(ProtocolVersion { major: 1, minor: 1 }, payload.clone(), vec![]);
        let decoded: WireEnvelope = serde_json::from_slice(&future_minor).unwrap();
        assert_eq!(decoded.compatibility().unwrap(), ProtocolCompatibility::ReadOnly);
        assert!(decode_identity_for_write(&future_minor).is_err());
        let future_major = wire(ProtocolVersion { major: 2, minor: 0 }, payload, vec![]);
        assert_eq!(decode_identity_for_write(&future_major), Err(ProtocolError::UnsupportedMajor));
    }

    #[test]
    fn unknown_critical_fields_and_features_fail_closed() {
        let mut payload = serde_json::to_value(identity()).unwrap();
        payload.as_object_mut().unwrap().insert("unknown_field".into(), Value::Bool(true));
        assert_eq!(
            decode_identity_for_write(&wire(ProtocolVersion::default(), payload, vec![])),
            Err(ProtocolError::InvalidIdentity)
        );
        let encoded = wire(ProtocolVersion::default(), serde_json::to_value(identity()).unwrap(),
            vec!["unknown-feature".to_owned()]);
        assert_eq!(decode_identity_for_write(&encoded),
            Err(ProtocolError::UnsupportedCriticalFeature("unknown-feature".to_owned())));
    }

    #[test]
    fn config_does_not_contain_tokens_or_credentials() {
        let config = SyncConfig {
            vault_id: VaultId::new(),
            device_id: DeviceId::new(),
            provider: SyncProvider::S3,
            profile_id: "local-profile".into(),
            encrypted_remote: true,
        };
        let bytes = serde_json::to_vec(&config).unwrap();
        let read: SyncConfig = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(read, config);
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value.as_object_mut().unwrap().insert("access_token".into(), Value::String("secret".into()));
        assert!(serde_json::from_value::<SyncConfig>(value).is_err());
    }
}
