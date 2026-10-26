//! Internal typed vault identity for the isolated parity laboratory.
//! Core retains its original VaultId; a later adapter must translate using
//! from_uuid/as_uuid at the verified host-custody boundary. This ID is not
//! proof of custody or authorization.
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultId(Uuid);

impl VaultId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    pub fn from_uuid(uuid: Uuid) -> Self { Self(uuid) }
    pub fn as_uuid(self) -> Uuid { self.0 }
}
impl fmt::Display for VaultId {
    fn fmt(&self,f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}
impl FromStr for VaultId {
    type Err=uuid::Error;
    fn from_str(s: &str) -> Result<Self,Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}
