//! G1-01: a TEST-ONLY, machine-readable envelope around the existing cold
//! inspector. It is NOT another cold verifier and never carries write authority.
//! This report is consumed by disposable fault-injection/VM laboratories.

use super::s3_reserved_replace::{InertColdUnifiedReview, InertColdUnifiedStatus};
use super::OperationId;
use crate::files::error::FileCommandError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColdOracleVerdict {
    LocallyConsistentHold,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColdOracleReason {
    CorrelatedButNotDurable,
    EvidenceRejected,
    WrongOperation,
    UnknownReviewStatus,
}

impl ColdOracleReason {
    fn code(self) -> &'static str {
        match self {
            Self::CorrelatedButNotDurable => "CORRELATED_NOT_DURABLE",
            Self::EvidenceRejected => "EVIDENCE_REJECTED",
            Self::WrongOperation => "WRONG_OPERATION",
            Self::UnknownReviewStatus => "UNKNOWN_REVIEW_STATUS",
        }
    }
}

/// The report deliberately omits path strings, raw original bytes, arbitrary
/// errors and any capability handle. A caller cannot turn HOLD into an ACK.
#[must_use = "forensic outcome is not a native receipt or S3 apply permission"]
#[derive(Debug)]
pub(crate) struct ColdOracleReport {
    operation_id: String,
    pub(crate) verdict: ColdOracleVerdict,
    pub(crate) reason: ColdOracleReason,
}

impl ColdOracleReport {
    /// Bind the report to the result of the EXISTING read-only inspector.
    /// Never reinterpret a verification error as a successful publication.
    pub(crate) fn from_existing_inspector(
        operation_id: OperationId,
        inspection: &Result<InertColdUnifiedReview, FileCommandError>,
    ) -> Self {
        let (verdict, reason) = match inspection {
            Ok(review) if review.operation_id != operation_id =>
                (ColdOracleVerdict::Deny, ColdOracleReason::WrongOperation),
            Ok(review) if review.status == InertColdUnifiedStatus::LocallyConsistentHold =>
                (ColdOracleVerdict::LocallyConsistentHold,
                 ColdOracleReason::CorrelatedButNotDurable),
            Ok(_) => (ColdOracleVerdict::Deny, ColdOracleReason::UnknownReviewStatus),
            Err(_) => (ColdOracleVerdict::Deny, ColdOracleReason::EvidenceRejected),
        };
        Self {
            operation_id: operation_id.to_string(),
            verdict,
            reason,
        }
    }

    pub(crate) fn to_json(&self) -> String {
        // A diagnostic schema for external labs, NOT a signed evidence record.
        serde_json::json!({
            "schema": "akula.sync.cold-oracle.v1",
            "operation_id": self.operation_id,
            "verdict": match self.verdict {
                ColdOracleVerdict::LocallyConsistentHold => "LOCALLY_CONSISTENT_HOLD",
                ColdOracleVerdict::Deny => "DENY",
            },
            "reason_code": self.reason.code(),
            "receipt_issued": false,
            "automatic_apply_allowed": false,
            "remote_ack_allowed": false,
            "baseline_promotion_allowed": false,
            "authority": "none",
        }).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g1_oracle_contract_fails_closed_and_never_grants_authority() {
        let operation = OperationId::new();
        let good = Ok(InertColdUnifiedReview {
            operation_id: operation,
            status: InertColdUnifiedStatus::LocallyConsistentHold,
        });
        let positive = ColdOracleReport::from_existing_inspector(operation, &good);
        assert_eq!(positive.verdict, ColdOracleVerdict::LocallyConsistentHold);
        assert_eq!(positive.reason, ColdOracleReason::CorrelatedButNotDurable);
        let json: serde_json::Value =
            serde_json::from_str(&positive.to_json()).unwrap();
        assert_eq!(json["schema"], "akula.sync.cold-oracle.v1");
        assert_eq!(json["operation_id"], operation.to_string());
        assert_eq!(json["verdict"], "LOCALLY_CONSISTENT_HOLD");
        assert_eq!(json["reason_code"], "CORRELATED_NOT_DURABLE");
        for key in ["receipt_issued", "automatic_apply_allowed",
                    "remote_ack_allowed", "baseline_promotion_allowed"] {
            assert_eq!(json[key], false, "{key} must remain denied");
        }
        assert_eq!(json["authority"], "none");

        let unexpected = ColdOracleReport::from_existing_inspector(
            OperationId::new(), &good,
        );
        assert_eq!(unexpected.verdict, ColdOracleVerdict::Deny);
        assert_eq!(unexpected.reason, ColdOracleReason::WrongOperation);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&unexpected.to_json())
                .unwrap()["automatic_apply_allowed"], false,
        );

        let rejected: Result<InertColdUnifiedReview, FileCommandError> =
            Err(FileCommandError::Task { message: "corrupted V8".to_owned() });
        let negative = ColdOracleReport::from_existing_inspector(
            operation, &rejected,
        );
        assert_eq!(negative.verdict, ColdOracleVerdict::Deny);
        assert_eq!(negative.reason, ColdOracleReason::EvidenceRejected);
        assert!(!negative.to_json().contains("corrupted V8"),
            "filesystem and database errors must not leak into public reports");
    }
}
