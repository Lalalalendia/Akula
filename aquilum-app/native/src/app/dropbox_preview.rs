//! Akula 3.13: explicit, read-only Dropbox test connection from native UI.
//! Never mutates local files, publishes remote data or changes Sync Core state.
#[path = "dropbox_provider.rs"]
mod provider;

use aquilum_core::settings::models::DropboxReadOnlyDraft;
use akula_dropbox_connect_lab::{resume::resume_readonly, ConnectSession, ConnectionStatus};
use akula_dropbox_t0::{read_verify::TrustedBlake3, Scope};
use std::{
    process::{Command, Stdio},
    sync::{mpsc::{channel, Receiver, Sender}, Arc},
    time::Duration,
};

pub(super) struct DropboxPreview {
    pub(super) status: String,
    pub(super) busy: bool,
    generation: u64,
    tx: Sender<(u64, &'static str)>,
    rx: Receiver<(u64, &'static str)>,
}
impl DropboxPreview {
    pub(super) fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            status: "DISCONNECTED / SYNC_DISABLED".to_owned(),
            busy: false,
            generation: 0,
            tx,
            rx,
        }
    }

    pub(super) fn start(
        &mut self,
        draft: DropboxReadOnlyDraft,
        resume: bool,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.busy { return; }
        if !draft.ready() {
            self.status = "INVALID_TEST_CONFIG / SYNC_DISABLED".into();
            return;
        }
        // Test scope and pin are constructed before any browser or network action.
        let Ok(scope) = Scope::new(
            &draft.account_id, &draft.root_namespace_id,
            &draft.vault_id, &draft.profile_id,
        ) else {
            self.status = "INVALID_SCOPE / SYNC_DISABLED".into();
            return;
        };
        let Ok(pin) = TrustedBlake3::parse(&draft.trusted_head_blake3) else {
            self.status = "INVALID_HEAD_PIN / SYNC_DISABLED".into();
            return;
        };
        self.busy = true;
        self.status = if resume {
            "VERIFYING_STORED_READ_ONLY_CONNECTION"
        } else {
            "WAITING_FOR_DROPBOX_BROWSER_CONSENT"
        }.into();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tx = self.tx.clone();
        // Never print the app key, OAuth URL, browser callback, token or HEAD.
        let _ = std::thread::Builder::new()
            .name("akula-dropbox-readonly".into())
            .spawn(move || {
                let outcome = if resume {
                    match resume_readonly(scope, &draft.oauth_app_key, pin) {
                        Ok(candidate) if candidate.status()
                            == ConnectionStatus::RemotePinnedNeedsPublisherApproval =>
                            "REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED",
                        _ => "READ_ONLY_RESUME_REJECTED / SYNC_DISABLED",
                    }
                } else {
                    match ConnectSession::begin(
                        scope, &draft.oauth_app_key, draft.registered_port, pin,
                    ) {
                        Ok(session) if open_browser(session.browser_authorize_url()) => {
                            match session.finish(Duration::from_secs(120)) {
                                Ok(candidate) if candidate.status()
                                    == ConnectionStatus::RemotePinnedNeedsPublisherApproval =>
                                    "REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED",
                                _ => "READ_ONLY_CONNECT_REJECTED / SYNC_DISABLED",
                            }
                        }
                        Ok(_) => "BROWSER_LAUNCH_FAILED / SYNC_DISABLED",
                        Err(_) => "LOGIN_BIND_REJECTED / SYNC_DISABLED",
                    }
                };
                if tx.send((generation, outcome)).is_ok() { wake(); }
            })
            .map_err(|_| {
                self.busy = false;
                self.status = "THREAD_START_FAILED / SYNC_DISABLED".into();
            });
    }

    /// Editing account/root/head invalidates even an in-flight successful reply.
    /// A stale test must not be displayed as proof for newly selected identity.
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.status = "DISCONNECTED / SYNC_DISABLED".into();
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((generation, status)) = self.rx.try_recv() {
            if generation != self.generation { continue; }
            self.status = status.into();
            self.busy = false;
            changed = true;
        }
        changed
    }
}

fn open_browser(url: &str) -> bool {
    let binary = if cfg!(target_os = "windows") {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // Argument is passed directly, without shell interpretation. OAuth URL
    // and children stdout/stderr are never included in application logs.
    Command::new(binary).arg(url)
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_scope_does_not_start_browser_or_network() {
        let mut state = DropboxPreview::new();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        state.start(DropboxReadOnlyDraft::default(), false, wake);
        assert!(!state.busy);
        assert_eq!(state.status, "INVALID_TEST_CONFIG / SYNC_DISABLED");
    }

    #[test]
    fn config_change_rejects_late_success_from_previous_account() {
        let mut state = DropboxPreview::new();
        let original_generation = state.generation;
        state.invalidate();
        assert_ne!(state.generation, original_generation);
        assert!(!state.busy);
        let _ = state.tx.send((
            original_generation,
            "REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED",
        ));
        assert!(!state.poll());
        assert_eq!(state.status, "DISCONNECTED / SYNC_DISABLED");
    }

    #[test]
    fn preview_never_reports_sync_enabled() {
        let state = DropboxPreview::new();
        assert!(state.status.contains("SYNC_DISABLED"));
    }
}
