use super::*;
    use super::super::trust::{admitted_fixture, policy_fixture};

    #[test]
    fn durable_once_and_wrong_peer_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let trust = policy_fixture(&temp.path().join("root"), [5; 32]);
        let admission = admitted_fixture(&trust);
        let mut ledger = DurableChallengeFence::open(&state, trust.clone()).unwrap();
        let nonce = ledger.issue(&admission).unwrap();
        assert!(!ledger.consume_once([8; 32], [31; 32], nonce).unwrap());
        drop(ledger);
        let mut reopened = DurableChallengeFence::open(&state, trust).unwrap();
        assert!(reopened.consume_once([7; 32], [31; 32], nonce).unwrap());
        assert!(!reopened.consume_once([7; 32], [31; 32], nonce).unwrap());
        assert!(!reopened.consume_once([7; 32], [32; 32], nonce).unwrap());
        assert!(!reopened.consume_once([7; 32], [31; 32], [0; 32]).unwrap());
    }

    #[test]
    fn root_policy_revoke_and_bad_state_permissions_halt() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let owner_dir = temp.path().join("root");
        let trust = policy_fixture(&owner_dir, [6; 32]);
        let admission = admitted_fixture(&trust);
        let mut ledger = DurableChallengeFence::open(&state, trust).unwrap();
        let nonce = ledger.issue(&admission).unwrap();
        fs::write(owner_dir.join("owner-policy.json"), b"revoked").unwrap();
        assert!(ledger.consume_once([7; 32], [31; 32], nonce).is_err());
        assert!(ledger.issue(&admission).is_err());
        fs::set_permissions(&state, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(DurableChallengeFence::open(
            &state, policy_fixture(&temp.path().join("root2"), [6; 32]),
        ).is_err());
    }
