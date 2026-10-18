//! Small, isolated FED-GOSSIP-02 laboratory using the actual iroh-gossip
//! 0.101.0 IO-less HyParView and PlumTree protocol state machine.
//! Simulated delivery is NOT signed federation authority or real UDP/QUIC.

pub mod announcement;

use bytes::Bytes;
use iroh_gossip::proto::{
    sim::{LatencyConfig, Network, NetworkConfig, Simulator, SimulatorConfig, BootstrapMode},
    Command, Config, Event, Scope, TopicId,
};
use rand::{rngs::ChaCha12Rng, SeedableRng};

fn topic() -> TopicId { [42u8; 32].into() }

fn network(count: u64, seed: u64) -> Network<u64, ChaCha12Rng> {
    let rng = ChaCha12Rng::seed_from_u64(seed);
    let cfg = NetworkConfig { proto: Config::default(), latency: LatencyConfig::default_static() };
    let mut n = Network::new(cfg, rng);
    for id in 0..count { n.insert(id); }
    n
}

fn count_receivers(n: &mut Network<u64, ChaCha12Rng>) -> usize {
    n.events().filter(|(_, _, ev)| matches!(ev, Event::Received(_))).count()
}

#[test]
fn three_peers_announce_to_both_neighbors() {
    let mut n = network(3, 1);
    n.command(0, topic(), Command::Join(vec![]));
    n.command(1, topic(), Command::Join(vec![0]));
    n.command(2, topic(), Command::Join(vec![0]));
    n.run_trips(15);
    assert!(n.check_synchronicity());
    let _ = n.events().count();
    n.command(1, topic(), Command::Broadcast(Bytes::from_static(b"test-replica-v1"), Scope::Swarm));
    n.run_trips(15);
    assert_eq!(count_receivers(&mut n), 2);
}

#[test]
fn split_network_does_not_claim_full_delivery_then_manual_bridge_repairs_new_messages() {
    let mut n = network(6, 2);
    n.command(0, topic(), Command::Join(vec![]));
    n.command(3, topic(), Command::Join(vec![]));
    for id in [1, 2] { n.command(id, topic(), Command::Join(vec![0])); }
    for id in [4, 5] { n.command(id, topic(), Command::Join(vec![3])); }
    n.run_trips(15);
    let _ = n.events().count();
    n.command(1, topic(), Command::Broadcast(Bytes::from_static(b"first-partitioned-announcement"), Scope::Swarm));
    n.run_trips(15);
    assert_eq!(count_receivers(&mut n), 2);
    n.command(2, topic(), Command::Join(vec![5]));
    n.run_trips(20);
    assert!(n.check_synchronicity());
    let _ = n.events().count();
    n.command(1, topic(), Command::Broadcast(Bytes::from_static(b"second-announcement-new-sequence"), Scope::Swarm));
    n.run_trips(20);
    assert_eq!(count_receivers(&mut n), 5);
}

#[test]
fn removal_of_one_peer_keeps_remaining_subnet_operational() {
    let mut n = network(3, 3);
    n.command(0, topic(), Command::Join(vec![]));
    n.command(1, topic(), Command::Join(vec![0]));
    n.command(2, topic(), Command::Join(vec![0]));
    n.run_trips(15);
    let _ = n.events().count();
    n.remove(&0);
    n.run_trips(15);
    let _ = n.events().count();
    // Direct restart/seed from a known surviving contact is a requirement,
    // not an implicit promise of automatic discovery.
    n.command(2, topic(), Command::Join(vec![1]));
    n.run_trips(20);
    let _ = n.events().count();
    n.command(1, topic(), Command::Broadcast(Bytes::from_static(b"owner-gone-copy-available"), Scope::Swarm));
    n.run_trips(20);
    assert_eq!(count_receivers(&mut n), 1);
}

#[test]
fn actual_simulator_broadcasts_to_everyone_for_ten_and_hundred_nodes() {
    for count in [10usize, 100] {
        let mut sim = Simulator::new(
            SimulatorConfig { rng_seed: 42, peers: count, gossip_round_timeout: std::time::Duration::from_secs(10) },
            NetworkConfig { proto: Config::default(), latency: LatencyConfig::default_static() },
        );
        let report = sim.bootstrap(BootstrapMode::Set { count: 3 });
        assert!(!report.has_peers_with_no_neighbors(), "bootstrap failed with {count} peers");
        for sequence in 0..3 {
            let msg = Bytes::from(format!("fed-gossip-02/{count}/{sequence}").into_bytes());
            assert_eq!(sim.gossip_round(vec![(0, msg)]), 0, "messages missing at {count} peers");
        }
        println!("FED_GOSSIP_02 peers={count} avg={:?}", sim.round_stats_average().mean);
    }
}


#[test]
fn thousand_join_leave_cycles_do_not_accumulate_live_peers() {
    let mut n = network(2, 91);
    n.command(0, topic(), Command::Join(vec![]));
    n.command(1, topic(), Command::Join(vec![0]));
    n.run_trips(10);
    for cycle in 0..1000u64 {
        let temporary = cycle + 2;
        n.insert(temporary);
        n.command(temporary, topic(), Command::Join(vec![0]));
        n.run_trips(8);
        n.remove(&temporary);
        n.run_trips(8);
        let live = n.peer_ids().count();
        assert_eq!(live, 2, "unexpected live peer leak at cycle {cycle}");
        assert!(n.check_synchronicity(), "neighbor disagreement at cycle {cycle}");
    }
    n.command(0, topic(), Command::Broadcast(Bytes::from_static(b"after-1000-churn"), Scope::Swarm));
    n.run_trips(12);
    assert_eq!(count_receivers(&mut n), 1);
}


#[test]
fn signed_announcement_is_delivered_but_each_receiver_enforces_its_own_grants() {
    use std::collections::BTreeMap;
    use announcement::{Announcement, AnnouncementGate, Reject};
    use ed25519_dalek::SigningKey;

    let mut n = network(3, 19);
    n.command(0, topic(), Command::Join(vec![]));
    n.command(1, topic(), Command::Join(vec![0]));
    n.command(2, topic(), Command::Join(vec![0]));
    n.run_trips(15);
    n.events().for_each(drop);

    let sk = SigningKey::from_bytes(&[99; 32]);
    let vault = [17; 16];
    let topic_bytes = [42; 32];
    let provider = sk.verifying_key().to_bytes();
    let mut gates = BTreeMap::<u64, AnnouncementGate>::new();
    for id in [1u64, 2] {
        let mut gate = AnnouncementGate::default();
        gate.grant(vault, topic_bytes, provider);
        gates.insert(id, gate);
    }

    let first = Announcement::signed(
        &sk, vault, topic_bytes, [33; 32], 1, 1, 1_000, 2_000,
    );
    let first_bytes = first.to_wire();
    n.command(0, topic(), Command::Broadcast(Bytes::from(first_bytes.clone()), Scope::Swarm));
    n.run_trips(15);
    let mut accepted = 0;
    for (peer, _, event) in n.events() {
        if let Event::Received(message) = event {
            let gate = gates.get_mut(&peer).expect("only other two peers received");
            assert_eq!(gate.accept(&message.content, 1_010), Ok(first.clone()));
            accepted += 1;
        }
    }
    assert_eq!(accepted, 2);
    assert_eq!(
        gates.get_mut(&1).unwrap().accept(&first_bytes, 1_011),
        Err(Reject::Replay),
        "duplicate delivery must not be interpreted as a new announcement",
    );

    // Revocation only affects the receiving application, NOT the gossip
    // transport: all nodes can still hear the update, but one rejects it.
    gates.get_mut(&2).unwrap().revoke(vault, topic_bytes, provider);
    let second = Announcement::signed(
        &sk, vault, topic_bytes, [33; 32], 1, 2, 1_020, 2_020,
    );
    n.command(0, topic(), Command::Broadcast(Bytes::from(second.to_wire()), Scope::Swarm));
    n.run_trips(15);

    let mut accepted = 0;
    let mut forbidden = 0;
    for (peer, _, event) in n.events() {
        if let Event::Received(message) = event {
            let result = gates.get_mut(&peer).unwrap().accept(&message.content, 1_030);
            match peer {
                1 => { assert_eq!(result, Ok(second.clone())); accepted += 1; },
                2 => { assert_eq!(result, Err(Reject::Forbidden)); forbidden += 1; },
                other => panic!("unexpected receiving peer {other}"),
            }
        }
    }
    assert_eq!((accepted, forbidden), (1, 1));
}
