//! Local transfer queue and test-only peer model; no network authorization.
use super::*;
use crate::i18n::t;

impl TransferState {
    pub(in crate::app) fn stage(&mut self, paths: Vec<PathBuf>) {
        self.error = None;
        for path in paths {
            if self.drafts.iter().any(|entry| entry.path == path) {
                continue;
            }
            if self.drafts.len() >= MAX_DRAFTS {
                self.error = Some(t("transfer.limit"));
                break;
            }
            // Metadata only. Never read bytes, compute a digest, write a file,
            // make a network call, follow symlinks or claim a verified transfer.
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                self.error = Some(t("transfer.invalidFile"));
                continue;
            };
            if !metadata.file_type().is_file() || path.file_name().is_none() {
                self.error = Some(t("transfer.invalidFile"));
                continue;
            }
            self.drafts.push(Draft { path, bytes: metadata.len(), paused: false });
        }
    }

    pub(in crate::app) fn toggle(&mut self, index: usize) {
        if let Some(item) = self.drafts.get_mut(index) {
            item.paused = !item.paused;
        }
    }

    pub(in crate::app) fn remove(&mut self, index: usize) {
        if index < self.drafts.len() {
            self.drafts.remove(index);
        }
    }

    pub(in crate::app) fn choose_peer(&mut self, binding: (DeviceId, [u8; 32])) {
        // Always match the *exact* displayed DeviceId and key, never a stale
        // list index which might now refer to a completely different peer.
        let matches: Vec<_> = self.peers.iter().filter(|peer| peer.id == binding.0).collect();
        self.selected_peer = if matches.len() == 1
            && matches[0].fingerprint == binding.1
            && binding.1 != [0; 32]
            && matches[0].status == transfer_ui::DeviceStatus::IdentityChecked
        {
            Some(binding)
        } else {
            None
        };
    }

    fn reconcile_selection(&mut self) {
        if let Some(binding) = self.selected_peer {
            let matches: Vec<_> = self.peers.iter().filter(|peer| peer.id == binding.0).collect();
            if matches.len() != 1 || matches[0].fingerprint != binding.1
                || matches[0].fingerprint == [0; 32]
                || matches[0].status != transfer_ui::DeviceStatus::IdentityChecked
            {
                self.selected_peer = None;
            }
        }
    }

    /// Synthetic source seam, compiled only in tests. In product builds a
    /// signed, custody-checked first-party registry adapter is a prerequisite.
    #[cfg(test)]
    pub(in crate::app) fn apply_test_peer_snapshot(&mut self, peers: Vec<TransferPeer>) {
        // Refuse oversize rosters rather than truncating away a conflicting ID.
        if peers.len() > MAX_DEVICE_ROWS {
            self.peers.clear();
            self.selected_peer = None;
            return;
        }
        self.peers = peers.into_iter()
            .filter(|peer| !peer.id.as_uuid().is_nil())
            .collect();
        // Any duplicate DeviceId is ambiguous, even with identical key bytes;
        // no UI selection can authorize a transfer in any case.
        let mut ambiguous = Vec::new();
        for (index, peer) in self.peers.iter().enumerate() {
            if self.peers.iter().enumerate().any(|(other_index, other)| {
                other_index != index && other.id == peer.id
            }) {
                ambiguous.push(index);
            }
        }
        for index in ambiguous {
            self.peers[index].status = transfer_ui::DeviceStatus::Unverified;
        }
        self.reconcile_selection();
    }

    pub(in crate::app) fn device_rows(&self) -> Vec<transfer_ui::DeviceRow> {
        self.peers.iter().take(MAX_DEVICE_ROWS).map(|peer| {
            let name: String = peer.name.chars()
                .filter(|c| !c.is_control()).take(64).collect();
            let name = if name.trim().is_empty() {
                peer.id.to_string()
            } else {
                name
            };
            // Public-key fingerprints are display metadata, not evidence that
            // a device is online, allowed to read, or entitled to receive.
            let fingerprint = peer.fingerprint[..8].iter()
                .map(|byte| format!("{byte:02x}")).collect::<String>();
            transfer_ui::DeviceRow {
                id: peer.id, key: peer.fingerprint, name, fingerprint,
                status: if peer.fingerprint == [0; 32] {
                    transfer_ui::DeviceStatus::Unverified
                } else {
                    peer.status
                },
                selected: self.selected_peer == Some((peer.id, peer.fingerprint)),
            }
        }).collect()
    }

    pub(in crate::app) fn rows(&self) -> Vec<transfer_ui::QueueRow> {
        self.drafts.iter().enumerate().map(|(index, item)| {
            let raw = item.path.file_name().unwrap_or_default().to_string_lossy();
            let mut name: String = raw.chars().filter(|c| !c.is_control()).take(96).collect();
            if raw.chars().count() > 96 {
                name.push('…');
            }
            transfer_ui::QueueRow { index, name, bytes: item.bytes, paused: item.paused }
        }).collect()
    }
}
