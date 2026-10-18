//! SYNC-05d2k: optional, AWS-only live account-and-bucket-owner proof.
//!
//! Existing durable V1/V2 observation scopes accept a host-supplied string:
//! such a string is NOT authentication. This independent capability obtains
//! a fresh AWS STS GetCallerIdentity response and verifies the real S3 bucket
//! owner before returning a typed, profile-bound scope for future admissions.
//!
//! It is deliberately unavailable for MinIO/custom S3 endpoints, which need
//! their own independently trusted provider identity protocol. A successful
//! probe is NOT signer authority, a monotonic remote root, a File Gate permit,
//! or an entitlement to perform S3 PUT/DELETE, CAS or baseline advancement.

use super::{AwsS3ProbeClient, S3ConnectionProfile};
use crate::sync::S3RemoteAnchorScope;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3AwsVerifiedAccountScope {
    scope: S3RemoteAnchorScope,
}

impl S3AwsVerifiedAccountScope {
    /// A profile/vault/bucket/endpoint-scoped observation identity. It has no
    /// write capability; future privileged APIs should accept this typed
    /// proof, not merely an arbitrary S3RemoteAnchorScope::new string.
    pub fn scope(&self) -> &S3RemoteAnchorScope { &self.scope }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3AwsAccountProofError {
    InvalidExpectedAccount,
    UnsupportedEndpoint,
    StsUnavailable,
    PrincipalMismatch,
    BucketNotVerified,
    InvalidScope,
}

fn valid_aws_account_id(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|c| c.is_ascii_digit())
}

/// Check that the STS response describes the expected account and that its
/// IAM/assumed-role ARN agrees with the returned account. Keep the ARN out
/// of SQLite, filenames and logs (role session labels may be identifying).
fn matches_sts_principal(
    approved: &str, observed_account: Option<&str>,
    arn: Option<&str>, user_id: Option<&str>,
) -> bool {
    if !valid_aws_account_id(approved)
        || observed_account != Some(approved)
        || !user_id.is_some_and(|value| !value.is_empty() && value.len() <= 256)
    {
        return false;
    }
    let Some(arn) = arn else { return false };
    if arn.len() > 2048 {
        return false;
    }
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    parts.len() == 6
        && parts[0] == "arn"
        && matches!(parts[1], "aws" | "aws-cn" | "aws-us-gov")
        && matches!(parts[2], "iam" | "sts")
        && parts[3].is_empty()
        && parts[4] == approved
        && !parts[5].is_empty()
}

/// Custom S3-compatible endpoints must not be treated as AWS-owned just
/// because their credentials can read a bucket or they return an ETag.
fn require_real_aws_profile(
    profile: &S3ConnectionProfile, approved: &str,
) -> Result<(), S3AwsAccountProofError> {
    if !valid_aws_account_id(approved) {
        return Err(S3AwsAccountProofError::InvalidExpectedAccount);
    }
    if profile.endpoint.is_some() {
        return Err(S3AwsAccountProofError::UnsupportedEndpoint);
    }
    Ok(())
}

impl AwsS3ProbeClient {
    /// Authoritative AWS account observation for an explicitly approved
    /// twelve-digit account ID. STS and S3 use the SAME resolved SDK
    /// credentials configuration. Both clients explicitly clear external
    /// endpoint overrides; neither trusts AWS_ENDPOINT_URL[_STS/_S3].
    ///
    /// HEAD Bucket with x-amz-expected-bucket-owner rejects another AWS
    /// account's bucket even when the principal has cross-account access.
    /// Lack of STS or HeadBucket permission is HOLD, never fallback to a
    /// caller's arbitrary account label. No remote object is downloaded.
    ///
    /// This proof covers the instant of the checks, not future credential
    /// rotation, malicious local SQLite replacement, or independent S3
    /// publishers. A future host must recheck at privileged admission.
    pub async fn verify_aws_account_scope(
        &self,
        profile_id: &str,
        independently_approved_account_id: &str,
    ) -> Result<S3AwsVerifiedAccountScope, S3AwsAccountProofError> {
        require_real_aws_profile(&self.profile, independently_approved_account_id)?;
        let sts = self.sts_client.as_ref()
            .ok_or(S3AwsAccountProofError::UnsupportedEndpoint)?;
        let identity = sts.get_caller_identity().send().await
            .map_err(|_| S3AwsAccountProofError::StsUnavailable)?;
        if !matches_sts_principal(
            independently_approved_account_id, identity.account(),
            identity.arn(), identity.user_id(),
        ) {
            return Err(S3AwsAccountProofError::PrincipalMismatch);
        }

        self.client.head_bucket()
            .bucket(&self.bucket)
            .expected_bucket_owner(independently_approved_account_id)
            .send().await
            .map_err(|_| S3AwsAccountProofError::BucketNotVerified)?;

        // The existing scope binds the same exact S3 profile/keyspace to the
        // verified AWS account. This remains an observation, not write auth.
        let label = format!("aws-account:{}", independently_approved_account_id);
        let scope = S3RemoteAnchorScope::new(
            profile_id, &label, &self.profile, &self.keyspace,
        ).map_err(|_| S3AwsAccountProofError::InvalidScope)?;
        Ok(S3AwsVerifiedAccountScope { scope })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(endpoint: Option<&str>) -> S3ConnectionProfile {
        S3ConnectionProfile {
            bucket: "account-owned-bucket".to_owned(),
            region: "us-east-1".to_owned(),
            endpoint: endpoint.map(str::to_owned),
            force_path_style: false,
        }
    }

    #[test]
    fn rejects_malformed_approved_ids_and_all_custom_endpoints() {
        for id in ["", "123", "12345678901x", "1234567890123", "１２３４５６７８９０１２"] {
            assert_eq!(
                require_real_aws_profile(&profile(None), id),
                Err(S3AwsAccountProofError::InvalidExpectedAccount)
            );
        }
        for endpoint in ["http://127.0.0.1:9000", "https://s3.example.test"] {
            assert_eq!(
                require_real_aws_profile(
                    &profile(Some(endpoint)), "123456789012",
                ), Err(S3AwsAccountProofError::UnsupportedEndpoint)
            );
        }
        assert!(require_real_aws_profile(
            &profile(None), "123456789012",
        ).is_ok());
    }

    #[test]
    fn exact_sts_account_arn_and_user_identity_are_required() {
        let id = "123456789012";
        assert!(matches_sts_principal(id, Some(id),
            Some("arn:aws:iam::123456789012:user/reader"), Some("AIDAEXAMPLE")));
        assert!(matches_sts_principal(id, Some(id),
            Some("arn:aws:sts::123456789012:assumed-role/reader/session"), Some("AROAEXAMPLE")));
        assert!(matches_sts_principal(id, Some(id),
            Some("arn:aws-cn:sts::123456789012:assumed-role/r/s"), Some("AROAEXAMPLE")));
        for (account, arn, user) in [
            (None, Some("arn:aws:iam::123456789012:user/x"), Some("USER")),
            (Some("000000000000"), Some("arn:aws:iam::123456789012:user/x"), Some("USER")),
            (Some(id), Some("arn:aws:iam::999999999999:user/x"), Some("USER")),
            (Some(id), Some("arn:aws:fake::123456789012:resource"), Some("USER")),
            (Some(id), Some("arn:aws:iam::123456789012:user/x"), None),
            (Some(id), None, Some("USER")),
        ] {
            assert!(!matches_sts_principal(id, account, arn, user));
        }
    }
}
