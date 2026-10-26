use super::*;
    use super::*;
    use std::io::Write;
    use std::net::Shutdown;

    fn wire(plan: &[u8], owner: &[u8]) -> Vec<u8> {
        let mut data = INTENT_DOMAIN.to_vec();
        data.extend_from_slice(&(plan.len() as u16).to_be_bytes());
        data.extend_from_slice(&(owner.len() as u16).to_be_bytes());
        data.extend_from_slice(plan);
        data.extend_from_slice(owner);
        data
    }

    fn receive(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), OwnerIpcError> {
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        sender.write_all(data).unwrap();
        sender.shutdown(Shutdown::Write).unwrap();
        read_bounded_intent(&mut receiver)
    }

    #[test]
    fn one_bounded_request_is_only_data_not_an_install_receipt() {
        let plan = br#"{"schema_version":3}"#;
        let owner = [0x22u8; 128];
        let (got_plan, got_owner) = receive(&wire(plan, &owner)).unwrap();
        assert_eq!(got_plan, plan);
        assert_eq!(got_owner, owner);
        // The legacy preview domain is not a commit-intent domain.
        let mut preview = wire(plan, &owner);
        preview[0] ^= 1;
        assert!(receive(&preview).is_err());
    }

    #[test]
    fn truncated_extra_and_unbounded_messages_hold() {
        let good = wire(br#"{}"#, &[0x33; 65]);
        assert!(receive(&good).is_ok());
        assert!(receive(&good[..good.len() - 1]).is_err());
        let mut extra = good.clone();
        extra.push(0);
        assert!(receive(&extra).is_err());
        let mut huge = good.clone();
        huge[INTENT_DOMAIN.len()] = 0xff;
        huge[INTENT_DOMAIN.len() + 1] = 0xff;
        assert!(receive(&huge).is_err());
        let mut short_owner = good.clone();
        short_owner[INTENT_DOMAIN.len() + 2] = 0;
        short_owner[INTENT_DOMAIN.len() + 3] = 64;
        assert!(receive(&short_owner).is_err());
        assert!(receive(&wire(b"{}", &[])).is_err());
        assert!(receive(&wire(b"", &[0x33; 65])).is_err());
    }

    #[test]
    fn root_commit_intent_is_not_available_to_unprivileged_caller() {
        if rustix::process::geteuid().as_raw() != 0 {
            let (mut caller, _other) = UnixStream::pair().unwrap();
            assert!(matches!(
                prepare_provisioned_commit_intent_once(&mut caller),
                Err(OwnerIpcError::Trust(RootEnrollmentError::NotRoot))
            ));
        }
    }
