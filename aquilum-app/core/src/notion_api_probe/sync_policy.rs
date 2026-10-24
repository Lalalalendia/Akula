//! Deterministic scheduling policy for future one-way Notion refresh jobs.
//! This does not start a worker, open an HTTP client, or store credentials.
//! The existing manual RefreshSession is still the only network entrypoint.
//!
//! Every enrolled root is initially disabled. Permission/review failures pause
//! automatically, and no code here deletes or modifies an offline snapshot.
pub mod persistence;

use std::collections::BTreeMap;
use uuid::Uuid;

pub const MIN_INTERVAL_MS: u64 = 15 * 60_000;
pub const MAX_ROOTS: usize = 32;
const BASE_BACKOFF_MS: u64 = 2_000;
const MAX_BACKOFF_MS: u64 = 30 * 60_000;
const MAX_TEMPORARY_FAILURES: u32 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hold {
    Authorization,
    Permission,
    NeedsReview,
    TemporaryFailures,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Complete,
    Throttled { retry_after_seconds: Option<u64> },
    TemporarilyUnavailable,
    Unauthorized,
    Forbidden,
    AccessBlocked,
    IncompleteOrUntrusted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootPolicy {
    pub root: String,
    pub enabled: bool,
    pub next_due_ms: u64,
    pub last_verified_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub hold: Option<Hold>,
}

pub struct SyncQueue {
    roots: BTreeMap<String, RootPolicy>,
    active_root: Option<String>,
    interval_ms: u64,
}

impl SyncQueue {
    pub fn new(interval_ms: u64) -> Result<Self, &'static str> {
        if interval_ms < MIN_INTERVAL_MS {
            return Err("Notion periodic refresh interval is below safety minimum");
        }
        Ok(Self { roots: BTreeMap::new(), active_root: None, interval_ms })
    }

    /// Merely registering a root NEVER turns periodic synchronization on.
    /// A later native UI flow must obtain explicit user opt-in.
    pub fn enroll(&mut self, root: &str, now_ms: u64) -> Result<(), &'static str> {
        let root = canonical_root(root)?;
        if self.roots.contains_key(&root) {
            return Ok(());
        }
        if self.roots.len() >= MAX_ROOTS {
            return Err("Notion scheduled root count exceeds bound");
        }
        self.roots.insert(root.clone(), RootPolicy {
            root,
            enabled: false,
            next_due_ms: now_ms,
            last_verified_ms: None,
            consecutive_failures: 0,
            hold: None,
        });
        Ok(())
    }

    /// This must only be invoked after a positive user action. It is also
    /// required to resume a root held after an authorization/review failure.
    pub fn set_enabled(
        &mut self, root: &str, enabled: bool, now_ms: u64,
    ) -> Result<(), &'static str> {
        let root = canonical_root(root)?;
        let item = self.roots.get_mut(&root).ok_or("Notion source not enrolled")?;
        if self.active_root.as_deref() == Some(root.as_str()) {
            return Err("cannot modify Notion source while worker is active");
        }
        item.enabled = enabled;
        if enabled {
            item.hold = None;
            item.consecutive_failures = 0;
            // Never discard an existing Retry-After deadline from a prior
            // rate limit solely because the user toggled this setting.
            item.next_due_ms = item.next_due_ms.max(now_ms);
        }
        Ok(())
    }

    pub fn status(&self, root: &str) -> Option<&RootPolicy> {
        let root = canonical_root(root).ok()?;
        self.roots.get(&root)
    }

    /// At most one job can run at a time, across all cached roots. This is an
    /// additional safety gate; the HTTP client also paces each request.
    pub fn claim_due(&mut self, now_ms: u64) -> Option<String> {
        if self.active_root.is_some() {
            return None;
        }
        let selected = self.roots.values()
            .filter(|s| s.enabled && s.hold.is_none() && s.next_due_ms <= now_ms)
            .min_by(|a, b| a.next_due_ms.cmp(&b.next_due_ms).then_with(|| a.root.cmp(&b.root)))?
            .root.clone();
        self.active_root = Some(selected.clone());
        Some(selected)
    }

    /// No retry or success transitions are allowed for a stale or wrong root.
    /// A rate-limit pause always respects the complete Retry-After value,
    /// including workspace-wide limits longer than a minute.
    pub fn finish(
        &mut self, root: &str, result: Outcome, now_ms: u64,
    ) -> Result<(), &'static str> {
        let root = canonical_root(root)?;
        if self.active_root.as_deref() != Some(root.as_str()) {
            return Err("Notion queue completion does not match active root");
        }
        let state = self.roots.get_mut(&root).ok_or("Notion active root missing")?;
        let new_failures = state.consecutive_failures.saturating_add(1);
        match result {
            Outcome::Complete => {
                state.last_verified_ms = Some(now_ms);
                state.consecutive_failures = 0;
                state.next_due_ms = now_ms.saturating_add(self.interval_ms);
                state.hold = None;
            }
            Outcome::Throttled { retry_after_seconds } => {
                state.consecutive_failures = new_failures;
                let delay = retry_after_seconds
                    .map(|seconds| seconds.saturating_mul(1_000))
                    .unwrap_or_else(|| exponential_ms(new_failures))
                    .max(exponential_ms(new_failures));
                state.next_due_ms = now_ms.saturating_add(delay);
                if new_failures >= MAX_TEMPORARY_FAILURES {
                    state.hold = Some(Hold::TemporaryFailures);
                }
            }
            Outcome::TemporarilyUnavailable => {
                state.consecutive_failures = new_failures;
                state.next_due_ms = now_ms.saturating_add(exponential_ms(new_failures));
                if new_failures >= MAX_TEMPORARY_FAILURES {
                    state.hold = Some(Hold::TemporaryFailures);
                }
            }
            Outcome::Unauthorized => state.hold = Some(Hold::Authorization),
            Outcome::Forbidden | Outcome::AccessBlocked =>
                state.hold = Some(Hold::Permission),
            Outcome::IncompleteOrUntrusted =>
                state.hold = Some(Hold::NeedsReview),
        }
        self.active_root = None;
        Ok(())
    }

    /// Retain status but stop background operations immediately when
    /// explicitly disabling a root. Already running jobs must finish through
    /// the normal read-only/atomic refresh contract before being disabled.
    pub fn pending_count(&self) -> usize {
        self.roots.values().filter(|s| s.enabled && s.hold.is_none()).count()
    }
}

fn canonical_root(value: &str) -> Result<String, &'static str> {
    let id = Uuid::parse_str(value).map_err(|_| "invalid Notion root UUID")?.to_string();
    if id != value { return Err("Notion UUID must be canonical"); }
    Ok(id)
}

fn exponential_ms(failures: u32) -> u64 {
    let steps = failures.saturating_sub(1).min(20);
    BASE_BACKOFF_MS.saturating_mul(1_u64 << steps).min(MAX_BACKOFF_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    #[test]
    fn disabled_by_default_requires_explicit_opt_in_and_serializes_roots() {
        let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        q.enroll(A, 0).unwrap();
        q.enroll(B, 0).unwrap();
        assert_eq!(q.claim_due(100), None);
        q.set_enabled(A, true, 100).unwrap();
        q.set_enabled(B, true, 100).unwrap();
        assert_eq!(q.claim_due(100).as_deref(), Some(A));
        assert_eq!(q.claim_due(100), None);
        assert!(q.set_enabled(A, false, 100).is_err());
        assert!(q.finish(B, Outcome::Complete, 101).is_err());
        q.finish(A, Outcome::Complete, 101).unwrap();
        assert_eq!(q.status(A).unwrap().last_verified_ms, Some(101));
        assert_eq!(q.claim_due(101).as_deref(), Some(B));
        q.finish(B, Outcome::Complete, 102).unwrap();
        assert_eq!(q.claim_due(102), None);
        assert_eq!(q.claim_due(MIN_INTERVAL_MS+101).as_deref(), Some(A));
    }

    #[test]
    fn notional_workspace_retry_after_is_never_capped_or_bypassed() {
        let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        q.enroll(A, 0).unwrap();
        q.set_enabled(A, true, 0).unwrap();
        q.claim_due(0).unwrap();
        q.finish(A, Outcome::Throttled { retry_after_seconds: Some(3600) }, 10).unwrap();
        assert_eq!(q.status(A).unwrap().next_due_ms, 3_600_010);
        assert_eq!(q.claim_due(60_000), None);
        // Turning a source off/on cannot bypass server-provided delay.
        q.set_enabled(A, false, 60_000).unwrap();
        q.set_enabled(A, true, 60_000).unwrap();
        assert_eq!(q.claim_due(60_000), None);
        assert_eq!(q.claim_due(3_600_010).as_deref(), Some(A));
        q.finish(A, Outcome::Complete, 3_600_011).unwrap();
    }

    #[test]
    fn authorization_and_partial_data_require_explicit_resume() {
        for (outcome, hold) in [
            (Outcome::Unauthorized, Hold::Authorization),
            (Outcome::Forbidden, Hold::Permission),
            (Outcome::AccessBlocked, Hold::Permission),
            (Outcome::IncompleteOrUntrusted, Hold::NeedsReview),
        ] {
            let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
            q.enroll(A, 0).unwrap();
            q.set_enabled(A, true, 0).unwrap();
            q.claim_due(0).unwrap();
            q.finish(A, outcome, 1).unwrap();
            assert_eq!(q.status(A).unwrap().hold, Some(hold));
            assert_eq!(q.claim_due(60_000_000), None);
            q.set_enabled(A, false, 60_000_000).unwrap();
            q.set_enabled(A, true, 60_000_000).unwrap();
            assert_eq!(q.claim_due(60_000_000).as_deref(), Some(A));
        }
    }

    #[test]
    fn network_failures_backoff_and_stop_after_five_attempts() {
        let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        q.enroll(A, 0).unwrap();
        q.set_enabled(A, true, 0).unwrap();
        let mut now = 0;
        for failure in 1..=MAX_TEMPORARY_FAILURES {
            assert_eq!(q.claim_due(now).as_deref(), Some(A));
            q.finish(A, Outcome::TemporarilyUnavailable, now).unwrap();
            assert_eq!(q.status(A).unwrap().consecutive_failures, failure);
            now = q.status(A).unwrap().next_due_ms;
        }
        assert_eq!(q.status(A).unwrap().hold, Some(Hold::TemporaryFailures));
        assert_eq!(q.claim_due(now+10_000), None);
    }

    #[test]
    fn failed_or_unsafe_attempt_never_advances_success_marker() {
        let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        q.enroll(A, 0).unwrap();
        q.set_enabled(A, true, 0).unwrap();
        q.claim_due(0).unwrap();
        q.finish(A, Outcome::Complete, 1).unwrap();
        let last = q.status(A).unwrap().last_verified_ms;
        q.claim_due(1 + MIN_INTERVAL_MS).unwrap();
        q.finish(A, Outcome::IncompleteOrUntrusted, 1 + MIN_INTERVAL_MS).unwrap();
        assert_eq!(q.status(A).unwrap().last_verified_ms, last);
    }

    #[test]
    fn rejects_invalid_ids_and_unbounded_source_count() {
        assert!(SyncQueue::new(MIN_INTERVAL_MS - 1).is_err());
        let mut q = SyncQueue::new(MIN_INTERVAL_MS).unwrap();
        assert!(q.enroll("../root", 0).is_err());
        assert!(q.enroll("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA", 0).is_err());
        for i in 0..MAX_ROOTS {
            let root = Uuid::from_u128(i as u128).to_string();
            q.enroll(&root, 0).unwrap();
        }
        assert_eq!(q.pending_count(), 0);
        let root = Uuid::from_u128(MAX_ROOTS as u128).to_string();
        assert!(q.enroll(&root, 0).is_err());
    }
}
