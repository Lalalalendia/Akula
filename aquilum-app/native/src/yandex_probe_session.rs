//! YC-S3-T01c-WIN-BRIDGE-0: inert GUI-thread coordinator for a future,
//! user-triggered, read-only S3 check. No S3 client, credential provider,
//! Settings button, remote Vault trust or Sync activation exists here.
//! The real worker must be wired only after PRs #388 and #408 pass acceptance.
//!
//! The only values retained are a nonsecret bucket name, monotonic tokens
//! and fixed status categories. The worker must retrieve keys itself.

use std::sync::{mpsc::{self, Receiver, Sender}, Arc};
use std::thread;

use aquilum_core::settings::models::YandexS3Onboarding;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeFailure {
    MissingKeys,
    InvalidStoredKeys,
    CredentialStoreUnavailable,
    KeysChangedDuringCheck,
    CredentialsRejected,
    AccessDenied,
    BucketNotFound,
    ClockSkew,
    WrongRegion,
    Timeout,
    NetworkOrTls,
    ServiceUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    ReadOnlyListVerified,
    Failed(ProbeFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeView {
    NotChecked,
    Checking,
    // No remote Vault ownership or Sync authority is implied.
    ReadOnlyListVerified,
    Failed(ProbeFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartError {
    InvalidBucket,
    MissingKeys,
    Busy,
    SequenceExhausted,
    WorkerUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Ticket {
    sequence: u64,
    epoch: u64,
    bucket: String,
}

struct Completion {
    ticket: Ticket,
    outcome: ProbeOutcome,
}

/// Single-flight, fail-closed background-worker coordinator.
/// Invalidation clears success immediately, but does not free the network
/// slot before the previous operation actually finishes.
pub struct ProbeSession {
    bucket: String,
    epoch: u64,
    sequence: u64,
    in_flight: Option<Ticket>,
    view: ProbeView,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl ProbeSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            bucket: String::new(), epoch: 0, sequence: 0,
            in_flight: None, view: ProbeView::NotChecked,
            tx, rx, wake,
        }
    }

    pub fn set_bucket(&mut self, bucket: &str) {
        if self.bucket != bucket {
            self.bucket = bucket.to_owned();
            self.invalidate();
        }
    }

    /// Save, replacement, deletion, or any other observed credentials change.
    pub fn credentials_changed(&mut self) {
        self.invalidate();
    }

    /// A closed Settings dialog cannot reopen with a previous success badge.
    pub fn settings_closed(&mut self) {
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.epoch = self.epoch.checked_add(1).expect("S3 probe epoch exhausted");
        self.view = ProbeView::NotChecked;
        // Do not release the slot: an old request may still be on the wire.
    }

    pub fn view_for(&self, bucket: &str) -> ProbeView {
        if self.bucket == bucket { self.view } else { ProbeView::NotChecked }
    }

    /// Call only on an explicit user action; never from render or startup.
    /// The operation closure executes on a dedicated background OS thread.
    /// It must fetch protected keys within the thread and return ONLY a
    /// nonsecret ProbeOutcome. The coordinator never takes credential strings.
    pub fn start(
        &mut self,
        credentials_present: bool,
        operation: impl FnOnce() -> ProbeOutcome + Send + 'static,
    ) -> Result<(), StartError> {
        if self.in_flight.is_some() {
            return Err(StartError::Busy);
        }
        // The onboarding draft validator accepts empty fields; network
        // admission must reject them explicitly.
        if self.bucket.is_empty() || !YandexS3Onboarding::valid_bucket(&self.bucket) {
            return Err(StartError::InvalidBucket);
        }
        if !credentials_present {
            return Err(StartError::MissingKeys);
        }
        let sequence = self.sequence.checked_add(1).ok_or(StartError::SequenceExhausted)?;
        let ticket = Ticket { sequence, epoch: self.epoch, bucket: self.bucket.clone() };
        self.sequence = sequence;
        self.view = ProbeView::Checking;
        self.in_flight = Some(ticket.clone());
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let started = thread::Builder::new()
            .name("akula-s3-readonly-check".into())
            .spawn(move || {
                // A worker panic must not wedge the single-flight gate.
                // Never include credential values in any worker panic.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                    .unwrap_or(ProbeOutcome::Failed(ProbeFailure::ServiceUnavailable));
                if tx.send(Completion { ticket, outcome }).is_ok() {
                    wake();
                }
            });
        if started.is_err() {
            self.in_flight = None;
            self.view = ProbeView::NotChecked;
            return Err(StartError::WorkerUnavailable);
        }
        Ok(())
    }

    /// Drain from the GUI thread following wake. A reply cannot change the
    /// visible status unless both token and selected bucket remain current.
    pub fn poll(&mut self) -> bool {
        let mut updated = false;
        while let Ok(completion) = self.rx.try_recv() {
            if self.in_flight.as_ref() != Some(&completion.ticket) {
                continue;
            }
            self.in_flight = None;
            if self.epoch != completion.ticket.epoch
                || self.bucket != completion.ticket.bucket
            {
                continue;
            }
            self.view = match completion.outcome {
                ProbeOutcome::ReadOnlyListVerified => ProbeView::ReadOnlyListVerified,
                ProbeOutcome::Failed(reason) => ProbeView::Failed(reason),
            };
            updated = true;
        }
        updated
    }

    pub fn busy(&self) -> bool {
        self.in_flight.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh() -> (ProbeSession, Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || { let _ = tx.send(()); });
        (ProbeSession::new(wake), rx)
    }

    fn awake(rx: &Receiver<()>) {
        rx.recv_timeout(Duration::from_secs(10)).expect("background worker did not wake UI");
    }

    #[test]
    fn rejects_invalid_empty_and_keyless_settings() {
        let (mut s, _) = fresh();
        assert_eq!(s.start(true, || ProbeOutcome::ReadOnlyListVerified), Err(StartError::InvalidBucket));
        s.set_bucket("https://evil.example");
        assert_eq!(s.start(true, || ProbeOutcome::ReadOnlyListVerified), Err(StartError::InvalidBucket));
        s.set_bucket("akula-private-bucket");
        assert_eq!(s.start(false, || ProbeOutcome::ReadOnlyListVerified), Err(StartError::MissingKeys));
        assert!(!s.busy());
        assert_eq!(s.view_for("akula-private-bucket"), ProbeView::NotChecked);
    }

    #[test]
    fn only_a_read_only_status_is_published_and_close_resets_it() {
        let (mut s, wake) = fresh();
        s.set_bucket("akula-private-bucket");
        s.start(true, || ProbeOutcome::ReadOnlyListVerified).unwrap();
        awake(&wake);
        assert!(s.poll());
        assert_eq!(s.view_for("akula-private-bucket"), ProbeView::ReadOnlyListVerified);
        assert_eq!(s.view_for("other-bucket"), ProbeView::NotChecked);
        s.settings_closed();
        assert_eq!(s.view_for("akula-private-bucket"), ProbeView::NotChecked);
    }

    #[test]
    fn key_rotation_discards_inflight_success_without_concurrent_reprobe() {
        let (mut s, wake) = fresh();
        let (release_tx, release_rx) = mpsc::channel();
        s.set_bucket("akula-private-bucket");
        s.start(true, move || {
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            ProbeOutcome::ReadOnlyListVerified
        }).unwrap();
        s.credentials_changed();
        assert_eq!(s.start(true, || ProbeOutcome::ReadOnlyListVerified), Err(StartError::Busy));
        release_tx.send(()).unwrap();
        awake(&wake);
        assert!(!s.poll());
        assert!(!s.busy());
        assert_eq!(s.view_for("akula-private-bucket"), ProbeView::NotChecked);
    }

    #[test]
    fn bucket_switch_makes_old_result_stale_and_allows_safe_retry() {
        let (mut s, wake) = fresh();
        let (release_tx, release_rx) = mpsc::channel();
        s.set_bucket("first-bucket");
        s.start(true, move || {
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            ProbeOutcome::ReadOnlyListVerified
        }).unwrap();
        s.set_bucket("second-bucket");
        assert_eq!(s.start(true, || ProbeOutcome::ReadOnlyListVerified), Err(StartError::Busy));
        release_tx.send(()).unwrap();
        awake(&wake);
        assert!(!s.poll());
        s.start(true, || ProbeOutcome::Failed(ProbeFailure::AccessDenied)).unwrap();
        awake(&wake);
        assert!(s.poll());
        assert_eq!(s.view_for("second-bucket"), ProbeView::Failed(ProbeFailure::AccessDenied));
    }

    #[test]
    fn duplicate_poll_and_closing_during_probe_cannot_restore_success() {
        let (mut s, wake) = fresh();
        s.set_bucket("akula-private-bucket");
        s.start(true, || ProbeOutcome::Failed(ProbeFailure::Timeout)).unwrap();
        awake(&wake);
        assert!(s.poll());
        assert!(!s.poll());
        assert_eq!(s.view_for("akula-private-bucket"), ProbeView::Failed(ProbeFailure::Timeout));
        s.start(true, || ProbeOutcome::ReadOnlyListVerified).unwrap();
        s.settings_closed();
        awake(&wake);
        assert!(!s.poll());
        assert_eq!(s.view_for("akula-private-bucket"), ProbeView::NotChecked);
    }
}
