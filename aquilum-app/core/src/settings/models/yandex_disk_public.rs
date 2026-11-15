//! Inert, public-only Yandex Disk account hints saved in Akula settings.
//!
//! These records are untrusted after disk load: a syntactically valid account
//! ID is NOT an authenticated session, a confirmed remote revision or Sync
//! permission. No refresh/access tokens, callback codes or client secrets are
//! accepted as fields. The native host pins the expected registered app ID
//! independently, and the OS keyring remains the only credential store.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_BINDINGS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YandexDiskPublicError {
    InvalidRecord,
    InvalidStoredCollection,
    ConflictingAccount,
    TooManyRecords,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct YandexDiskPublicBinding {
    account_id: String,
    oauth_client_id: String,
    workspace_uuid: String,
    profile_id: String,
}

impl YandexDiskPublicBinding {
    pub fn new(account_id: &str, oauth_client_id: &str, workspace_uuid: &str,
        profile_id: &str) -> Result<Self, YandexDiskPublicError>
    {
        let entry = Self {
            account_id: account_id.to_owned(),
            oauth_client_id: oauth_client_id.to_owned(),
            workspace_uuid: workspace_uuid.to_owned(),
            profile_id: profile_id.to_owned(),
        };
        if !entry.valid() { return Err(YandexDiskPublicError::InvalidRecord); }
        Ok(entry)
    }

    pub fn account_id(&self) -> &str { &self.account_id }
    pub fn oauth_client_id(&self) -> &str { &self.oauth_client_id }
    pub fn workspace_uuid(&self) -> &str { &self.workspace_uuid }
    pub fn profile_id(&self) -> &str { &self.profile_id }

    fn valid(&self) -> bool {
        !self.account_id.is_empty() && self.account_id.len() <= 64
            && self.account_id.bytes().all(|b| b.is_ascii_digit())
            && valid_label(&self.oauth_client_id, 128)
            && valid_uuid(&self.workspace_uuid)
            && valid_label(&self.profile_id, 64)
    }
}

fn valid_label(v: &str, max: usize) -> bool {
    !v.is_empty() && v.len() <= max
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_uuid(v: &str) -> bool {
    v.len() == 36 && v.bytes().enumerate().all(|(i, b)| {
        if [8, 13, 18, 23].contains(&i) { b == b'-' }
        else { b.is_ascii_digit() || (b'a'..=b'f').contains(&b) }
    })
}

/// A bounded list of public identity hints; never grant remote access based
/// on these hints. One account is allowed per (local workspace, profile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct YandexDiskPublicAccounts {
    accounts: Vec<YandexDiskPublicBinding>,
}

impl YandexDiskPublicAccounts {
    pub fn is_well_formed(&self) -> bool {
        if self.accounts.len() > MAX_BINDINGS { return false; }
        let mut unique = BTreeSet::new();
        self.accounts.iter().all(|entry| {
            entry.valid() && unique.insert((&entry.workspace_uuid, &entry.profile_id))
        })
    }

    /// Observation only. The native host must first complete a user-approved
    /// account+app verification; this method does NOT perform that verification.
    /// Reject implicit account switching: the caller must forget the old OS
    /// credential (and its exact public pointer) first.
    pub fn remember_observed(&mut self, entry: YandexDiskPublicBinding)
        -> Result<(), YandexDiskPublicError>
    {
        if !self.is_well_formed() {
            return Err(YandexDiskPublicError::InvalidStoredCollection);
        }
        if !entry.valid() {
            return Err(YandexDiskPublicError::InvalidRecord);
        }
        if let Some(old) = self.accounts.iter().find(|old| {
            old.workspace_uuid == entry.workspace_uuid
                && old.profile_id == entry.profile_id
        }) {
            return if old == &entry { Ok(()) }
                else { Err(YandexDiskPublicError::ConflictingAccount) };
        }
        if self.accounts.len() == MAX_BINDINGS {
            return Err(YandexDiskPublicError::TooManyRecords);
        }
        self.accounts.push(entry);
        Ok(())
    }

    /// A loaded, untrusted public hint is returned only with the exact
    /// installed app ID and local workspace/profile. The caller MUST still
    /// check OS keyring presence, and MUST NOT call it an active connection.
    pub fn unverified_for<'a>(&'a self, workspace_uuid: &str,
        oauth_client_id: &str, profile_id: &str)
        -> Option<&'a YandexDiskPublicBinding>
    {
        if !self.is_well_formed() { return None; }
        self.accounts.iter().find(|entry| {
            entry.workspace_uuid == workspace_uuid
                && entry.oauth_client_id == oauth_client_id
                && entry.profile_id == profile_id
        })
    }

    /// Exact-only removal, after OS keyring verified an absent credential.
    /// Never erase another workspace, account, app, or local profile.
    pub fn forget_exact(&mut self, expected: &YandexDiskPublicBinding)
        -> Result<bool, YandexDiskPublicError>
    {
        if !self.is_well_formed() {
            return Err(YandexDiskPublicError::InvalidStoredCollection);
        }
        if !expected.valid() {
            return Err(YandexDiskPublicError::InvalidRecord);
        }
        let n = self.accounts.len();
        self.accounts.retain(|entry| entry != expected);
        Ok(self.accounts.len() != n)
    }

    pub fn len(&self) -> usize { self.accounts.len() }
}

#[cfg(test)]
#[path = "yandex_disk_public_tests.rs"]
mod tests;
