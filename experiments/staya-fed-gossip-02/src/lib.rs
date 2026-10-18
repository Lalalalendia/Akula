//! Small, isolated FED-GOSSIP-02 laboratory using the actual iroh-gossip
//! 0.101.0 IO-less HyParView and PlumTree protocol state machine.
//! Simulated delivery is NOT signed federation authority or real UDP/QUIC.

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
