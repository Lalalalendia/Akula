//! Research-only composition: actual iroh-gossip simulated transport,
//! independently signed announcements and read-only inventory reconciliation.
//! Does not grant file access or mutate Akula's journal/File Gate.
use bytes::Bytes;
use ed25519_dalek::SigningKey;
use iroh_gossip::proto::{sim::{LatencyConfig,Network,NetworkConfig},Command,Config,Event,Scope,TopicId};
use rand::{rngs::ChaCha12Rng,SeedableRng};
use staya_fed_gossip_lab::announcement::{Announcement,AnnouncementGate,Reject};
use staya_fed_inventory_lab::{compare,full_compare,sample_revision,Inventory,Revision,TEST_VAULT};

pub fn reconcile_if_authorized(
    gate: &mut AnnouncementGate, payload: &[u8], now: u64,
    local: &Inventory, remote: &Inventory,
) -> Result<Vec<Revision>, String> {
    // A valid announcement must refer to the very Vault being compared.
    // Never let a provider's scope-verified but unrelated hint trigger a scan.
    let claim = gate.accept(payload,now).map_err(|e|format!("announcement: {e:?}"))?;
    if claim.vault != local.vault_scope() || claim.vault != remote.vault_scope() {
        return Err("announcement/inventory Vault mismatch".into());
    }
    // These are synthetic fixtures: the separate inventory code checks vault
    // scope and a simulated permission bit, not real Akula signed grants.
    let diff = compare(local,remote,100_000).map_err(|e|format!("inventory: {e:?}"))?;
    if diff.collisions.len()>0 {return Err("same ID with different content digest".into())}
    let full = full_compare(local,remote).map_err(|e|format!("baseline: {e:?}"))?;
    if diff.missing_from_a!=full.missing_from_a || diff.missing_from_b!=full.missing_from_b {
        return Err("range discrepancy".into())
    }
    // The announced content hash is a hint, not proof of file presence.
    let _hint_only = claim.content_digest;
    Ok(diff.missing_from_a)
}

#[cfg(test)]
mod tests {
    use super::*;
    const TOPIC:[u8;32]=[42;32];
    fn setup() -> (SigningKey, AnnouncementGate) {
        let key=SigningKey::from_bytes(&[19;32]);
        let mut gate=AnnouncementGate::default();
        gate.grant(TEST_VAULT,TOPIC,key.verifying_key().to_bytes());
        (key,gate)
    }
    fn inventories()->(Inventory,Inventory) {
        let a=Inventory::fixture(TEST_VAULT,true,(0..1000).map(sample_revision).collect()).unwrap();
        let b=Inventory::fixture(TEST_VAULT,true,(0..1001).map(sample_revision).collect()).unwrap();
        (a,b)
    }
    #[test]
    fn network_announcement_causes_read_only_reconciliation_on_two_peers() {
        let mut n:Network<u64,ChaCha12Rng>=Network::new(
            NetworkConfig{proto:Config::default(),latency:LatencyConfig::default_static()},
            ChaCha12Rng::seed_from_u64(18));
        let t=TopicId::from_bytes(TOPIC);
        for id in 0..3{n.insert(id)}
        n.command(0,t,Command::Join(vec![]));
        n.command(1,t,Command::Join(vec![0]));
        n.command(2,t,Command::Join(vec![0]));
        n.run_trips(15);
        n.events().for_each(drop);
        let (key, mut g1)=setup();
        let (_, mut g2)=setup();
        let (a,b)=inventories();
        let msg=Announcement::signed(&key,TEST_VAULT,TOPIC,[6;32],1,1,1000,2000);
        let raw=msg.to_wire();
        n.command(0,t,Command::Broadcast(Bytes::from(raw.clone()),Scope::Swarm));
        n.run_trips(15);
        let mut accepted=0;
        for (peer,_,ev) in n.events(){
            if let Event::Received(m)=ev{
                let gate=if peer==1{&mut g1}else{&mut g2};
                let missing=reconcile_if_authorized(gate,&m.content,1001,&a,&b).unwrap();
                assert_eq!(missing,vec![sample_revision(1000)]);
                accepted+=1;
            }
        }
        assert_eq!(accepted,2);
        assert!(reconcile_if_authorized(&mut g1,&raw,1002,&a,&b).is_err(),"replay");
    }
    #[test]
    fn signed_message_for_other_vault_never_triggers_inventory_comparison() {
        let (key, mut gate) = setup();
        let other_vault = [22u8;16];
        gate.grant(other_vault, TOPIC, key.verifying_key().to_bytes());
        let foreign_notice = Announcement::signed(&key, other_vault, TOPIC, [6;32], 2, 1, 1000, 2000);
        let (a,b) = inventories();
        let result = reconcile_if_authorized(&mut gate, &foreign_notice.to_wire(), 1001, &a, &b);
        assert_eq!(result, Err("announcement/inventory Vault mismatch".into()));
    }
    #[test]
    fn untrusted_inventories_cannot_be_compared_after_valid_announcement() {
        let (key, mut gate) = setup();
        let notice = Announcement::signed(&key, TEST_VAULT, TOPIC, [6;32], 2, 1, 1000, 2000);
        let left = Inventory::fixture(TEST_VAULT, false, vec![]).unwrap();
        let right = Inventory::fixture(TEST_VAULT, true, vec![sample_revision(2)]).unwrap();
        let result = reconcile_if_authorized(&mut gate, &notice.to_wire(), 1001, &left, &right);
        assert_eq!(result, Err("inventory: NoGrant".into()));
    }
    #[test]
    fn revoked_or_wrong_vault_fails_before_candidate_discovery() {
        let (key,mut gate)=setup();
        let msg=Announcement::signed(&key,TEST_VAULT,TOPIC,[6;32],1,1,1000,2000);
        let (a,b)=inventories();
        gate.revoke(TEST_VAULT,TOPIC,key.verifying_key().to_bytes());
        assert!(reconcile_if_authorized(&mut gate,&msg.to_wire(),1001,&a,&b).is_err());
        gate.grant(TEST_VAULT,TOPIC,key.verifying_key().to_bytes());
        let foreign=Inventory::fixture([5;16],true,b.entries().to_vec()).unwrap();
        assert!(reconcile_if_authorized(&mut gate,&msg.to_wire(),1001,&a,&foreign).is_err());
        // The failed attempt may have consumed the sequence, which is fail-closed.
    }
}
