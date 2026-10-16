use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

// Distinct types prevent accidentally substituting device, document, vault and revision IDs.
macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

define_id!(VaultId);
define_id!(DocumentId);
define_id!(DeviceId);
define_id!(RevisionId);
define_id!(OperationId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_ids_round_trip_and_remain_distinct() {
        let vault = VaultId::new();
        let document = DocumentId::new();
        let device = DeviceId::new();
        let revision = RevisionId::new();
        let operation = OperationId::new();

        assert_eq!(VaultId::from_str(&vault.to_string()).unwrap(), vault);
        assert_eq!(DocumentId::from_str(&document.to_string()).unwrap(), document);
        assert_eq!(DeviceId::from_str(&device.to_string()).unwrap(), device);
        assert_eq!(RevisionId::from_str(&revision.to_string()).unwrap(), revision);
        assert_eq!(OperationId::from_str(&operation.to_string()).unwrap(), operation);

        for json in [
            serde_json::to_string(&vault).unwrap(),
            serde_json::to_string(&document).unwrap(),
            serde_json::to_string(&device).unwrap(),
            serde_json::to_string(&revision).unwrap(),
            serde_json::to_string(&operation).unwrap(),
        ] {
            let value: String = serde_json::from_str(&json).unwrap();
            assert!(Uuid::parse_str(&value).is_ok());
        }
        assert_ne!(vault.as_uuid(), document.as_uuid());
    }

    #[test]
    fn invalid_id_is_rejected_on_decode() {
        assert!(serde_json::from_str::<DocumentId>(r#""invalid-id""#).is_err());
    }
}
