use super::*;
use lesha_replication::{
    evaluate_protection, plan_bounded_repairs, BoundedRepairPlan,
    DurabilityEligibility, FailureDomainId, HealthEvidenceSequence,
    PolicyDeficit, ProtectedSubject, ProtectionState, ReceiptHealth,
    ReplicaEvidence, ReplicaHealthObservation, ReplicaHealthObservationKind,
    ReplicaHealthTracker, ReplicaPlacementPolicy, ReplicaSubject, ReplicaTopology,
    RepairBudget, RepairCandidate, RepairCandidateState, RepairPlanState,
    RepairWorkCandidate,
};
use lesha_types::{ChunkId, ClusterId, NodeGeneration, NodeId, NodeRef};

fn node(tag: u8) -> NodeRef {
    NodeRef {
        cluster_id: ClusterId([1; 16]),
        node_id: NodeId([tag; 32]),
        generation: NodeGeneration(1),
    }
}
fn domain(tag: u8) -> FailureDomainId {
    FailureDomainId([tag; 16])
}
fn signer(tag: u8) -> SigningKey {
    SigningKey::from_bytes(&[tag; 32])
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
fn topology(tag: u8, host: Option<u8>) -> ReplicaTopology {
    ReplicaTopology {
        host: host.map(domain),
        storage_device: Some(domain(tag + 20)),
        site: None,
        power: None,
    }
}
fn evidence(
    tag: u8,
    receipt: VerifiedReplicaReceipt,
    host: Option<u8>,
    is_anchor: bool,
) -> ReplicaEvidence {
    ReplicaEvidence {
        node: node(tag),
        receipts: vec![receipt],
        receipt_health: ReceiptHealth::CurrentVerified,
        eligibility: DurabilityEligibility::CountsForDurability,
        topology: topology(tag, host),
        is_anchor,
    }
}
fn candidate(
    tag: u8,
    host: Option<u8>,
    is_anchor: bool,
    expected_bytes: u64,
    state: RepairCandidateState,
) -> RepairWorkCandidate {
    RepairWorkCandidate {
        candidate: RepairCandidate {
            node: node(tag),
            eligibility: DurabilityEligibility::CountsForDurability,
            topology: topology(tag, host),
            is_anchor,
            state,
        },
        expected_bytes,
    }
}
fn budget(bytes: u64, jobs: usize) -> RepairBudget {
    RepairBudget { max_targets: 2, max_jobs: jobs, max_bytes: bytes }
}
fn repair(
    target: &ProtectedSubject,
    existing: &[ReplicaEvidence],
    candidates: &[RepairWorkCandidate],
    max_bytes: u64,
    max_jobs: usize,
) -> BoundedRepairPlan {
    plan_bounded_repairs(target, policy(), existing, candidates, budget(max_bytes, max_jobs))
        .expect("planner must return status, not mutate data")
}

struct Fixture {
    bytes: Vec<u8>,
    target: ProtectedSubject,
    disks: SyntheticDiskCluster,
    gate: SyntheticReceiptGate,
}

impl Fixture {
    fn new() -> Self {
        let bytes = vec![0xA5u8; 1024];
        let hash = sha256(&bytes);
        let target = ProtectedSubject {
            cluster_id: ClusterId([1; 16]),
            subject: ReplicaSubject::Chunk(ChunkId(hash)),
            subject_hash: hash,
        };
        let mut gate = SyntheticReceiptGate::default();
        for tag in 1..=8 {
            gate.enroll(node(tag), signer(tag).verifying_key(), 1);
        }
        Self {
            bytes, target, gate, disks: SyntheticDiskCluster::default(),
        }
    }

    fn announce(&self, tag: u8, committed: bool) -> SyntheticReceipt {
        SyntheticReceipt::sign(
            &signer(tag),
            ReceiptId([tag; 16]),
            node(tag),
            self.target,
            1,
            committed,
        )
    }

    fn certify(&self, tag: u8) -> VerifiedReplicaReceipt {
        self.gate.admit(&self.disks, &self.target, &self.announce(tag, true))
            .expect("only admitted after simulated commit and readback")
    }

    fn provision(&mut self, tag: u8, anchor: bool) -> ReplicaEvidence {
        self.disks.stage(node(tag), 1, self.bytes.clone());
        self.disks.simulated_commit(node(tag));
        evidence(tag, self.certify(tag), Some(tag), anchor)
    }
}

#[test]
fn three_independent_prevalidated_copies_are_protected() {
    let mut f = Fixture::new();
    let a = f.provision(1, true);
    let b = f.provision(2, false);
    let c = f.provision(3, false);
    let s = evaluate_protection(&f.target, policy(), &[a, b, c]).unwrap();
    assert_eq!(s.state, ProtectionState::Replicated);
    assert_eq!(s.verified_replicas, 3);
    assert_eq!(s.distinct_hosts, 3);
    assert_eq!(s.distinct_storage_devices, 3);
    assert_eq!(s.anchor_replicas, 1);
    assert!(s.deficits.is_empty());
    println!("YDB_REPL_01 three_copies=verified_3_homes_3");
}

#[test]
fn a_lost_b_c_survive_d_repair_is_only_projected_until_receipt() {
    let mut f = Fixture::new();
    let mut a = f.provision(1, true);
    let b = f.provision(2, false);
    let c = f.provision(3, false);
    a.receipt_health = ReceiptHealth::Missing;
    let current = [a.clone(), b.clone(), c.clone()];
    let degraded = evaluate_protection(&f.target, policy(), &current).unwrap();
    assert_eq!(degraded.state, ProtectionState::DegradedPolicyUnsatisfied);
    assert_eq!(degraded.verified_replicas, 2);

    // A new ID in B's same host is not independent. D closes host, count,
    // storage and anchor deficits in one bounded copy.
    let choice = repair(
        &f.target,
        &current,
        &[
            candidate(5, Some(2), true, 1024, RepairCandidateState::Available),
            candidate(4, Some(4), true, 1024, RepairCandidateState::Available),
        ],
        1024,
        1,
    );
    assert_eq!(choice.state, RepairPlanState::Complete);
    assert_eq!(choice.scheduled.len(), 1);
    assert_eq!(choice.scheduled[0].node.node_id, node(4).node_id);
    assert_eq!(choice.projected.state, ProtectionState::Replicated);
    // The important guard: planner is prospective, NOT a durable receipt.
    assert_eq!(
        evaluate_protection(&f.target, policy(), &current).unwrap().verified_replicas,
        2,
    );

    f.disks.stage(node(4), 1, f.bytes.clone());
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &f.announce(4, true)),
        Err(AdmissionError::NotCommitted),
    );
    f.disks.simulated_commit(node(4));
    let d = evidence(4, f.certify(4), Some(4), true);
    let restored = evaluate_protection(&f.target, policy(), &[a, b, c, d]).unwrap();
    assert_eq!(restored.state, ProtectionState::Replicated);
    assert_eq!(restored.distinct_hosts, 3);
    assert_eq!(restored.verified_replicas, 3);
    println!("YDB_REPL_01 simulated_loss_A=degraded repair_D=credited_after_readback");
}

#[test]
fn staged_transfer_and_uncommitted_ack_never_count_as_durable_copy() {
    let mut f = Fixture::new();
    f.disks.stage(node(2), 1, f.bytes.clone());
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &f.announce(2, true)),
        Err(AdmissionError::NotCommitted),
        "signed words alone are not a local commit",
    );
    f.disks.simulated_commit(node(2));
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &f.announce(2, false)),
        Err(AdmissionError::NotCommitted),
    );
    let receipt = f.certify(2);
    let mut reported = evidence(2, receipt, Some(2), false);
    reported.receipts.clear(); // gossip hint with no admitted receipt
    let s = evaluate_protection(&f.target, policy(), &[reported]).unwrap();
    assert_eq!(s.verified_replicas, 0);
    assert_eq!(s.state, ProtectionState::DegradedPolicyUnsatisfied);
}

#[test]
fn same_host_and_unknown_site_cannot_satisfy_placement() {
    let mut f = Fixture::new();
    let a = f.provision(1, true);
    let mut b = f.provision(2, false);
    let mut c = f.provision(3, false);
    b.topology.host = Some(domain(1));
    c.topology.site = None;
    let s = evaluate_protection(&f.target, policy(), &[a.clone(), b.clone(), c.clone()]).unwrap();
    assert_eq!(s.verified_replicas, 3);
    assert_eq!(s.distinct_hosts, 2);
    assert!(s.deficits.iter().any(|d| matches!(
        d, PolicyDeficit::DistinctHosts { required: 3, observed: 2 }
    )));
    assert_eq!(s.state, ProtectionState::DegradedPolicyUnsatisfied);

    let requiring_site = ReplicaPlacementPolicy {
        min_distinct_sites: 3, ..policy()
    };
    let s = evaluate_protection(&f.target, requiring_site, &[a, b, c]).unwrap();
    assert_eq!(s.distinct_sites, 0);
    assert_eq!(s.state, ProtectionState::DegradedPolicyUnsatisfied);
}

#[test]
fn corrupt_copy_loses_health_credit_and_can_only_repair_from_good_bytes() {
    let mut f = Fixture::new();
    let a = f.provision(1, true);
    let mut b = f.provision(2, false);
    let c = f.provision(3, false);
    f.disks.corrupt(node(2));
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &f.announce(2, true)),
        Err(AdmissionError::DigestMismatch),
    );

    let mut tracker = ReplicaHealthTracker::new(node(2), 1);
    tracker.apply(ReplicaHealthObservation {
        node: node(2),
        storage_generation: 1,
        sequence: HealthEvidenceSequence(1),
        kind: ReplicaHealthObservationKind::VerifiedHealthy,
    }).unwrap();
    tracker.apply(ReplicaHealthObservation {
        node: node(2),
        storage_generation: 1,
        sequence: HealthEvidenceSequence(2),
        kind: ReplicaHealthObservationKind::DigestMismatch,
    }).unwrap();
    b.receipt_health = tracker.health();
    assert_eq!(b.receipt_health, ReceiptHealth::Corrupt);
    let damaged = [a.clone(), b.clone(), c.clone()];
    let degraded = evaluate_protection(&f.target, policy(), &damaged).unwrap();
    assert_eq!(degraded.verified_replicas, 2);
    let plan = repair(
        &f.target, &damaged,
        &[candidate(4, Some(4), false, 1024, RepairCandidateState::Available)],
        1024, 1,
    );
    assert_eq!(plan.scheduled.len(), 1);
    assert_eq!(plan.scheduled[0].node.node_id, node(4).node_id);

    // A good complete copy is used. Corrupt B never becomes a repair source.
    let trusted_source = f.disks.committed_bytes(node(3)).unwrap().to_vec();
    assert_eq!(sha256(&trusted_source), f.target.subject_hash);
    f.disks.stage(node(4), 1, trusted_source);
    f.disks.simulated_commit(node(4));
    let d = evidence(4, f.certify(4), Some(4), false);
    assert_eq!(
        evaluate_protection(&f.target, policy(), &[a, b, c, d]).unwrap().state,
        ProtectionState::Replicated,
    );
}

#[test]
fn byte_and_job_budgets_do_not_implicitly_commit_repairs() {
    let mut f = Fixture::new();
    let mut a = f.provision(1, true);
    let b = f.provision(2, false);
    let c = f.provision(3, false);
    a.receipt_health = ReceiptHealth::Missing;
    let existing = [a, b, c];
    let work = [candidate(4, Some(4), true, 1024, RepairCandidateState::Available)];
    for (max_bytes, jobs) in [(1023, 1usize), (1024, 0usize)] {
        let plan = repair(&f.target, &existing, &work, max_bytes, jobs);
        assert!(plan.scheduled.is_empty());
        assert_eq!(plan.state, RepairPlanState::Partial);
        assert_eq!(plan.projected.verified_replicas, 2);
    }
    let ready = repair(&f.target, &existing, &work, 1024, 1);
    assert_eq!(ready.scheduled.len(), 1);
    assert_eq!(ready.used_bytes, 1024);
    assert_eq!(ready.used_jobs, 1);
    assert_eq!(
        evaluate_protection(&f.target, policy(), &existing).unwrap().state,
        ProtectionState::DegradedPolicyUnsatisfied,
    );
}

#[test]
fn no_credit_for_bad_signature_wrong_subject_or_storage_generation() {
    let mut f = Fixture::new();
    let _a = f.provision(1, true);
    let mut altered = f.announce(1, true);
    altered.receipt_id = ReceiptId([99; 16]);
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &altered),
        Err(AdmissionError::BadSignature),
    );

    let mut foreign = f.announce(1, true);
    foreign.subject.subject_hash = [77; 32];
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &foreign),
        Err(AdmissionError::WrongSubject),
    );

    f.gate.enroll(node(1), signer(1).verifying_key(), 2);
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &f.announce(1, true)),
        Err(AdmissionError::WrongGeneration),
    );
    f.gate.enroll(
        NodeRef { generation: NodeGeneration(2), ..node(1) },
        signer(1).verifying_key(),
        1,
    );
    assert_eq!(
        f.gate.admit(&f.disks, &f.target, &f.announce(1, true)),
        Err(AdmissionError::WrongGeneration),
    );
}

#[test]
fn duplicated_receipt_and_zero_receipt_cannot_fake_multiple_replicas() {
    let mut f = Fixture::new();
    let a = f.provision(1, true);
    let mut b = f.provision(2, false);
    b.receipts.push(b.receipts[0]);
    let s = evaluate_protection(&f.target, policy(), &[a.clone(), b]).unwrap();
    assert_eq!(s.verified_replicas, 2);
    let mut no_evidence = a;
    no_evidence.receipts.clear();
    let s = evaluate_protection(&f.target, policy(), &[no_evidence]).unwrap();
    assert_eq!(s.verified_replicas, 0);
}
