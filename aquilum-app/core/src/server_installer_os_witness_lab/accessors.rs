//! Existing witness accessors and isolated root-only test fixture.
use super::*;
impl OsWitnessLab {
    /// Test-only: validate the exact production B1d0 witness marker and key
    /// in a disposable OS-root-owned fixture without exposing a configurable
    /// production path or privileged IPC.
    #[cfg(all(test, feature = "server-root-b0v3-gate-lab"))]
    pub(crate) fn open_provisioned_fixture(
        trust_path: &Path, journal_path: &Path,
    ) -> Result<Self, OsWitnessError> {
        Self::open_enrolled(trust_path, journal_path, PROD_ENROLLMENT, PROD_ID)
    }
    pub fn trusted_witness(&self) -> TrustedWitnessSigner {
        self.trust.clone()
    }
    pub fn current_checkpoint(&self) -> Result<WitnessCheckpoint, OsWitnessError> {
        self.ledger.current_checkpoint().map_err(OsWitnessError::Witness)
    }
}
