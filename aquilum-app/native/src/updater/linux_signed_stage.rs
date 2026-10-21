//! RELEASE-SIGNED-STAGING-01: internal typed bridge from publisher candidate
//! to an fsync'd on-disk AppImage candidate and a retained OS lock.
//!
//! No production activation is possible until publisher signing authority,
//! installed destination authority and first-GUI ACK have separately passed.
//! All real updater entrypoints remain HOLD and cannot call this to install.
//! The original transaction engine owns renames and cold recovery.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};

use super::linux_transaction::{CANDIDATE, Gate};
use super::release::{self, SignedCandidate};

const TRANSACTION_ACTIVATION_APPROVED: bool = false;

fn hold(reason: &str) -> String {
    format!("AKULA_SIGNED_STAGING_HOLD: {reason}")
}

/// Holds the locked installation directory AND the original signed
/// candidate. This is not a publication authorization, and cannot rename
/// the current installed executable or discard the previous version.
#[allow(dead_code)]
pub(crate) struct PendingSignedStage {
    _gate: Gate,
    stage: PathBuf,
    active: PathBuf,
    candidate: SignedCandidate,
}

impl PendingSignedStage {
    #[allow(dead_code)]
    pub(crate) fn stage_path(&self) -> &Path {
        &self.stage
    }

    #[allow(dead_code)]
    pub(crate) fn rehash(&self) -> Result<(), String> {
        self.candidate.verify_staged_file(&self.stage.join(CANDIDATE))
    }

    #[allow(dead_code)]
    pub(crate) fn version(&self) -> &str {
        self.candidate.version()
    }

    #[allow(dead_code)]
    pub(crate) fn source_commit(&self) -> &str {
        self.candidate.source_commit()
    }

    #[allow(dead_code)]
    pub(crate) fn digest(&self) -> &str {
        self.candidate.sha256()
    }

    /// Test-only integration: actually rename a signed test generation into
    /// the active path, simulate an absent first-frame ACK, and exercise the
    /// SAME recoverable rollback methods that passed SIGKILL process tests.
    /// This method is not present in any installed Akula executable.
    #[cfg(test)]
    pub(crate) fn prove_missing_ack_rollback(&self) -> Result<(), String> {
        self.rehash()?;
        self._gate.backup_old(&self.stage)?;
        self._gate.publish_new(&self.stage)?;
        self.candidate.verify_staged_file(&self.active)?;
        self._gate.rollback_unacknowledged(&self.stage)?;
        Ok(())
    }
}

fn stage_checked(
    active: &Path,
    nonce: &str,
    candidate: SignedCandidate,
) -> Result<PendingSignedStage, String> {
    // The Gate retains one nonblocking OS flock across the entire operation.
    // A future installed supervisor must also prove active refers to the
    // actual signed Akula installation, never a caller-selected path.
    let gate = Gate::acquire(active)?;
    let stage = gate.prepare(nonce, candidate.bytes())?;
    // Verify the actual file on disk, not just the downloaded in-memory bytes.
    // An inode mismatch / link / concurrent tamper is a hard HOLD.
    candidate.verify_staged_file(&stage.join(CANDIDATE))?;
    Ok(PendingSignedStage {
        _gate: gate,
        stage,
        active: active.to_path_buf(),
        candidate,
    })
}

/// Future production entrypoint. Fail closed independently of the public
/// release channel: mere provisioning of a signing key is not consent to
/// installing an unsigned, wrong-path, or unacknowledged generation.
#[allow(dead_code)]
pub(crate) fn stage_signed_candidate(
    active: &Path,
    nonce: &str,
    candidate: SignedCandidate,
) -> Result<PendingSignedStage, String> {
    if !TRANSACTION_ACTIVATION_APPROVED
        || !release::channel_ready()
        || release::PUBLIC_KEY.is_empty()
    {
        let _ = (active, nonce, candidate);
        return Err(hold(
            "publisher root, installed destination authority and live GUI supervisor are not approved",
        ));
    }
    stage_checked(active, nonce, candidate)
}

/// Unit/integration tests only. This alternative is excluded from installed
/// binaries: positive ephemeral CLI Minisign fixtures exercise the exact
/// same disk-staging implementation without authorizing product releases.
#[cfg(test)]
pub(crate) fn stage_verified_for_test(
    active: &Path,
    nonce: &str,
    candidate: SignedCandidate,
) -> Result<PendingSignedStage, String> {
    stage_checked(active, nonce, candidate)
}
