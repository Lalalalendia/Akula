use staya_filegate_receipt_lab::{
    admit_after_restart, file_path, filegate_publish, node, receipt_path,
    sign_and_persist, target, ReceiptError, TEXT,
};
use lesha_replication::{
    evaluate_protection, plan_bounded_repairs, DurabilityEligibility, FailureDomainId,
    PolicyDeficit, ProtectionState, ReceiptHealth, ReplicaEvidence,
    ReplicaPlacementPolicy, ReplicaTopology, RepairBudget, RepairCandidate,
    RepairCandidateState, RepairPlanState, RepairWorkCandidate, VerifiedReplicaReceipt,
};
use std::fs;
use std::path::Path;
use std::process::Command;

fn child(base: &Path, boundary: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_staya-filegate-child"))
        .arg(base)
        .arg(boundary)
        .output()
        .expect("child subprocess launch")
}

fn provision(base: &Path, tag: u8) -> VerifiedReplicaReceipt {
    filegate_publish(base, tag).expect("real Akula Core File Gate create");
    sign_and_persist(base, tag).expect("persist research receipt");
    admit_after_restart(base, tag).expect("reopen local bytes and receipt")
}

fn domain(tag: u8) -> FailureDomainId {
    FailureDomainId([tag; 16])
}

fn evidence(tag: u8, receipt: VerifiedReplicaReceipt, host: Option<u8>, anchor: bool)
    -> ReplicaEvidence
{
    ReplicaEvidence {
        node: node(tag),
        receipts: vec![receipt],
        receipt_health: ReceiptHealth::CurrentVerified,
        eligibility: DurabilityEligibility::CountsForDurability,
        topology: ReplicaTopology {
            host: host.map(domain),
            storage_device: Some(domain(tag + 20)),
            site: None,
            power: None,
        },
        is_anchor: anchor,
    }
}

fn policy() -> ReplicaPlacementPolicy {
    ReplicaPlacementPolicy {
        full_replica_count: 3,
        min_distinct_hosts: 3,
        min_distinct_storage_devices: 3,
        min_distinct_sites: 0,
        min_distinct_power_domains: 0,
        min_anchor_replicas: 1,
    }
}

#[test]
fn exit_before_real_gate_does_not_publish_or_create_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let out = child(temp.path(), "before_gate");
    assert_eq!(out.status.code(), Some(80), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(!file_path(temp.path(), 1).exists());
    assert!(!receipt_path(temp.path(), 1).exists());
    assert_eq!(admit_after_restart(temp.path(), 1), Err(ReceiptError::MissingReceipt));
}

#[test]
fn crash_after_real_gate_before_receipt_has_bytes_but_zero_durability_credit() {
    let temp = tempfile::tempdir().unwrap();
    let out = child(temp.path(), "after_gate");
    assert_eq!(out.status.code(), Some(81), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(fs::read(file_path(temp.path(), 1)).unwrap(), TEXT.as_bytes());
    assert!(!receipt_path(temp.path(), 1).exists());
    assert_eq!(admit_after_restart(temp.path(), 1), Err(ReceiptError::MissingReceipt));

    // A new verifier process is NOT allowed to turn a filename into a valid
    // receipt. An honest lone store with no receipt counts as zero.
    let empty = evidence_placeholder(1);
    let assessment = evaluate_protection(&target(), policy(), &[empty]).unwrap();
    assert_eq!(assessment.verified_replicas, 0);
    assert_eq!(assessment.state, ProtectionState::DegradedPolicyUnsatisfied);
}

fn evidence_placeholder(tag: u8) -> ReplicaEvidence {
    ReplicaEvidence {
        node: node(tag),
        receipts: vec![],
        receipt_health: ReceiptHealth::Unknown,
        eligibility: DurabilityEligibility::CountsForDurability,
        topology: ReplicaTopology {
            host: Some(domain(tag)),
            storage_device: Some(domain(tag + 20)),
            site: None,
            power: None,
        },
        is_anchor: false,
    }
}

#[test]
fn crash_after_signed_receipt_before_ack_is_recoverable_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let out = child(temp.path(), "after_receipt");
    assert_eq!(out.status.code(), Some(82), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(fs::read(file_path(temp.path(), 1)).unwrap(), TEXT.as_bytes());
    let accepted = admit_after_restart(temp.path(), 1).expect("receipt survived process exit");
    assert_eq!(accepted.destination, node(1));
    let second_read = admit_after_restart(temp.path(), 1).expect("repeat must be safe");
    assert_eq!(accepted, second_read);
    let once = evidence(1, accepted, Some(1), true);
    let mut again = once.clone();
    again.receipts.push(second_read);
    let assessment = evaluate_protection(
        &target(),
        ReplicaPlacementPolicy {
            full_replica_count: 2, ..Default::default()
        },
        &[again],
    ).unwrap();
    assert_eq!(assessment.verified_replicas, 1);
    println!("FILEGATE_RECEIPT_02 crash_after_receipt=reopened duplicate_credit=0");
}

#[test]
fn modification_of_bytes_after_receipt_invalidates_replica() {
    let temp = tempfile::tempdir().unwrap();
    provision(temp.path(), 2);
    fs::write(file_path(temp.path(), 2), b"foreign modified bytes").unwrap();
    assert_eq!(admit_after_restart(temp.path(), 2), Err(ReceiptError::CorruptBytes));
}

#[test]
fn tampered_receipt_signature_never_credits_local_file() {
    let temp = tempfile::tempdir().unwrap();
    provision(temp.path(), 3);
    let path = receipt_path(temp.path(), 3);
    let mut wire = fs::read(&path).unwrap();
    *wire.last_mut().unwrap() ^= 0x3F;
    fs::write(&path, wire).unwrap(); // deliberate attacker mutation in disposable fixture
    assert_eq!(admit_after_restart(temp.path(), 3), Err(ReceiptError::BadSignature));
}

#[test]
fn file_gate_never_replaces_existing_replica_name() {
    let temp = tempfile::tempdir().unwrap();
    provision(temp.path(), 2);
    assert_eq!(filegate_publish(temp.path(), 2), Err(ReceiptError::FileGateDenied));
    assert_eq!(fs::read(file_path(temp.path(), 2)).unwrap(), TEXT.as_bytes());
    admit_after_restart(temp.path(), 2).unwrap();
}

#[test]
fn filegate_copies_feed_real_lesha_planner_but_only_confirmed_d_is_counted() {
    let temp = tempfile::tempdir().unwrap();
    let a = evidence(1, provision(temp.path(), 1), Some(1), true);
    let b = evidence(2, provision(temp.path(), 2), Some(2), false);
    let c = evidence(3, provision(temp.path(), 3), Some(3), false);
    assert_eq!(
        evaluate_protection(&target(), policy(), &[a.clone(), b.clone(), c.clone()]).unwrap().state,
        ProtectionState::Replicated,
    );
    let mut a_failed = a.clone();
    a_failed.receipt_health = ReceiptHealth::Missing;
    let current = [a_failed.clone(), b.clone(), c.clone()];
    assert_eq!(
        evaluate_protection(&target(), policy(), &current).unwrap().verified_replicas,
        2,
    );

    let candidates = [
        RepairWorkCandidate {
            candidate: RepairCandidate {
                node: node(5),
                eligibility: DurabilityEligibility::CountsForDurability,
                topology: ReplicaTopology {
                    host: Some(domain(2)), // same physical host label as B
                    storage_device: Some(domain(25)),
                    site: None,
                    power: None,
                },
                is_anchor: true,
                state: RepairCandidateState::Available,
            },
            expected_bytes: TEXT.len() as u64,
        },
        RepairWorkCandidate {
            candidate: RepairCandidate {
                node: node(4),
                eligibility: DurabilityEligibility::CountsForDurability,
                topology: ReplicaTopology {
                    host: Some(domain(4)),
                    storage_device: Some(domain(24)),
                    site: None,
                    power: None,
                },
                is_anchor: true,
                state: RepairCandidateState::Available,
            },
            expected_bytes: TEXT.len() as u64,
        },
    ];
    let plan = plan_bounded_repairs(
        &target(),
        policy(),
        &current,
        &candidates,
        RepairBudget {
            max_targets: 1,
            max_jobs: 1,
            max_bytes: TEXT.len() as u64,
        },
    ).unwrap();
    assert_eq!(plan.state, RepairPlanState::Complete);
    assert_eq!(plan.scheduled.len(), 1);
    assert_eq!(plan.scheduled[0].node.node_id, node(4).node_id);
    assert_eq!(plan.projected.state, ProtectionState::Replicated);
    assert_eq!(
        evaluate_protection(&target(), policy(), &current).unwrap().state,
        ProtectionState::DegradedPolicyUnsatisfied,
    );

    // This lab writes identical known test bytes via D's real File Gate.
    // It is not a Roha/noq C -> D peer transfer or host-attested topology.
    filegate_publish(temp.path(), 4).unwrap();
    assert_eq!(admit_after_restart(temp.path(), 4), Err(ReceiptError::MissingReceipt));
    let before = evaluate_protection(&target(), policy(), &current).unwrap();
    assert_eq!(before.verified_replicas, 2);
    sign_and_persist(temp.path(), 4).unwrap();
    let d = evidence(4, admit_after_restart(temp.path(), 4).unwrap(), Some(4), true);
    let restored = evaluate_protection(&target(), policy(), &[a_failed, b, c, d]).unwrap();
    assert_eq!(restored.state, ProtectionState::Replicated);
    assert_eq!(restored.verified_replicas, 3);
    assert_eq!(restored.distinct_hosts, 3);
    println!("FILEGATE_RECEIPT_02 A_offline=degraded D_verified_after_receipt=protected");
}

#[test]
fn same_host_label_does_not_claim_failure_domain_diversity() {
    let temp = tempfile::tempdir().unwrap();
    let mut replicas = Vec::new();
    for tag in 1..=3u8 {
        replicas.push(evidence(tag, provision(temp.path(), tag), Some(1), tag == 1));
    }
    let s = evaluate_protection(&target(), policy(), &replicas).unwrap();
    assert_eq!(s.verified_replicas, 3);
    assert_eq!(s.distinct_hosts, 1);
    assert!(s.deficits.contains(&PolicyDeficit::DistinctHosts { required: 3, observed: 1 }));
    assert_eq!(s.state, ProtectionState::DegradedPolicyUnsatisfied);
}

#[test]
fn refused_byte_or_job_budget_cannot_authorize_file_gate_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let mut a = evidence(1, provision(temp.path(), 1), Some(1), true);
    let b = evidence(2, provision(temp.path(), 2), Some(2), false);
    let c = evidence(3, provision(temp.path(), 3), Some(3), false);
    a.receipt_health = ReceiptHealth::Missing;
    let current = [a, b, c];
    let work = [RepairWorkCandidate {
        candidate: RepairCandidate {
            node: node(4),
            eligibility: DurabilityEligibility::CountsForDurability,
            topology: ReplicaTopology {
                host: Some(domain(4)),
                storage_device: Some(domain(24)),
                site: None, power: None,
            },
            is_anchor: true,
            state: RepairCandidateState::Available,
        },
        expected_bytes: TEXT.len() as u64,
    }];
    for budget in [
        RepairBudget {
            max_targets: 1, max_jobs: 0, max_bytes: TEXT.len() as u64
        },
        RepairBudget {
            max_targets: 1, max_jobs: 1, max_bytes: TEXT.len() as u64 - 1
        },
    ] {
        let plan = plan_bounded_repairs(&target(), policy(), &current, &work, budget).unwrap();
        assert_eq!(plan.state, RepairPlanState::Partial);
        assert!(plan.scheduled.is_empty());
        assert_eq!(plan.projected.verified_replicas, 2);
        assert!(!file_path(temp.path(), 4).exists());
    }
}

#[test]
fn receipt_rejected_when_signed_node_identity_is_rebound_to_other_tag() {
    let temp = tempfile::tempdir().unwrap();
    provision(temp.path(), 2);
    let wrong = fs::read(receipt_path(temp.path(), 2)).unwrap();
    let new_path = receipt_path(temp.path(), 3);
    fs::create_dir_all(new_path.parent().unwrap()).unwrap();
    fs::write(new_path, wrong).unwrap(); // synthetic hijacked receipt
    assert_eq!(admit_after_restart(temp.path(), 3), Err(ReceiptError::WrongIdentity));
}
