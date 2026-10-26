//! Immutable, signed release review; no installation or service authority.
use super::*;
/// Read-only KEX/preflight and independent release review errors.
/// Neither variant authorizes remote installation or sudo.
#[cfg(feature = "server-install-review")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallReviewProbeError {
    Host(B0Error),
    Release(crate::server_release_lab::review::ReviewError),
}

impl WindowsOpenSshPinnedProbe {
    /// Verify actual OS SSH KEX and typed Debian/Ubuntu preflight, then
    /// independently verify B1a signed Node bytes for an immutable UI plan.
    ///
    /// A fingerprint-only settings profile can discover a candidate key via
    /// OS ssh-keyscan; it is accepted ONLY if SHA256 matches the independently
    /// pinned fingerprint and OpenSSH authenticates a signed KEX.
    ///
    /// Neither this call nor the returned PreparedReview is owner-signed
    /// B1b1b approval, remote installation authority or systemd activation.
    #[cfg(feature = "server-install-review")]
    pub fn prepare_signed_node_review<R: Read>(
        input: &B0Input,
        independently_pinned_host_public: Option<&str>,
        release: &crate::server_release_lab::SignedRelease,
        trusted_signer: &crate::server_release_lab::TrustedReleaseSigner,
        independently_persisted_floor: crate::server_release_lab::ReleaseFloor,
        node_package: &mut R,
        now_unix: u64,
        cancelled: impl Fn() -> bool,
    ) -> Result<crate::server_release_lab::review::PreparedReview, InstallReviewProbeError> {
        Self::prepare_signed_node_review_scope(
            input, independently_pinned_host_public, release, trusted_signer,
            independently_persisted_floor, node_package, None,
            now_unix, cancelled,
        )
    }

    #[cfg(feature = "server-install-review")]
    #[allow(clippy::too_many_arguments)]
    fn prepare_signed_node_review_scope<R: Read>(
        input: &B0Input,
        independently_pinned_host_public: Option<&str>,
        release: &crate::server_release_lab::SignedRelease,
        trusted_signer: &crate::server_release_lab::TrustedReleaseSigner,
        independently_persisted_floor: crate::server_release_lab::ReleaseFloor,
        node_package: &mut R,
        helper_sha256: Option<[u8; 32]>,
        now_unix: u64,
        cancelled: impl Fn() -> bool,
    ) -> Result<crate::server_release_lab::review::PreparedReview, InstallReviewProbeError> {
        if cancelled() {
            return Err(InstallReviewProbeError::Host(B0Error::Cancelled));
        }
        let public = match independently_pinned_host_public {
            Some(key) => key.to_string(),
            None => scan::discover_matching_ed25519(&input.target)
                .map_err(InstallReviewProbeError::Host)?,
        };
        let mut probe = Self::new(input, &public)
            .map_err(InstallReviewProbeError::Host)?;
        let plan = match helper_sha256 {
            Some(helper_hash) => crate::server_bootstrap_lab::preview_node_v3(
                input, helper_hash, &mut probe, &cancelled,
            ),
            None => crate::server_bootstrap_lab::preview_node_v2(
                input, &mut probe, &cancelled,
            ),
        }.map_err(InstallReviewProbeError::Host)?;
        if cancelled() {
            return Err(InstallReviewProbeError::Host(B0Error::Cancelled));
        }
        crate::server_release_lab::review::prepare_signed_review(
            plan, release, trusted_signer, independently_persisted_floor,
            node_package, now_unix,
        ).map_err(InstallReviewProbeError::Release)
    }


    /// B0 v3 explicitly binds the separately signed helper SHA into the
    /// immutable review plan, in addition to the Node daemon package SHA.
    /// The installed UI does not call this experimental method yet.
    #[cfg(feature = "server-root-stage-ssh-lab")]
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_signed_node_review_with_bootstrap<R: Read>(
        input: &B0Input,
        independently_pinned_host_public: Option<&str>,
        release: &crate::server_release_lab::SignedRelease,
        trusted_signer: &crate::server_release_lab::TrustedReleaseSigner,
        independently_persisted_floor: crate::server_release_lab::ReleaseFloor,
        node_package: &mut R,
        helper: &crate::server_first_root_stage::VerifiedRootStageBundle,
        now_unix: u64,
        cancelled: impl Fn() -> bool,
    ) -> Result<crate::server_release_lab::review::PreparedReview, InstallReviewProbeError> {
        Self::prepare_signed_node_review_scope(
            input, independently_pinned_host_public, release, trusted_signer,
            independently_persisted_floor, node_package,
            Some(helper.package_sha256()), now_unix, cancelled,
        )
    }

}
