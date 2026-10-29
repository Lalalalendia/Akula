use super::*;

    const NOW: u64 = 1_800_000_000;

    fn peer(n: u128, b: u8) -> PeerBinding {
        PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(n)),
            endpoint_id: [b; 32],
        }
    }

    fn peers() -> (PeerBinding, PeerBinding) {
        (peer(1, 41), peer(2, 42))
    }

    fn started() -> (CallSession, CallSession, Wire) {
        let (a, b) = peers();
        let (caller, wire) = CallSession::outbound(
            a, b, [7; 16], NOW, 60, LocalPeerPolicy::Allowed
        ).unwrap();
        let receiver = CallSession::incoming(
            &wire, a, b, LocalPeerPolicy::Allowed, NOW + 1
        ).unwrap();
        (caller, receiver, wire)
    }

    fn connected() -> (CallSession, CallSession) {
        let (a, b) = peers();
        let (mut caller, mut receiver, _) = started();
        let ringing = receiver.ring_local(NOW + 1).unwrap();
        assert_eq!(caller.receive(&ringing, b, LocalPeerPolicy::Allowed, NOW + 1).unwrap(),
            RemoteEvent::Ringing);
        let acceptance = receiver.accept_local(NOW + 2).unwrap();
        assert_eq!(caller.receive(&acceptance, b, LocalPeerPolicy::Allowed, NOW + 2).unwrap(),
            RemoteEvent::Accepted);
        assert_eq!(caller.phase(), CallPhase::Active);
        assert_eq!(receiver.phase(), CallPhase::Active);
        assert!(!caller.remote_muted().eq(&false));
        let _ = a;
        (caller, receiver)
    }

    #[test]
    fn two_peer_offer_ring_accept_end_and_idempotent_close() {
        let (_, b) = peers();
        let (mut caller, mut receiver) = connected();
        let end = caller.end_local().unwrap().unwrap();
        assert_eq!(receiver.receive(&end, peers().0, LocalPeerPolicy::Allowed, NOW + 80).unwrap(),
            RemoteEvent::Ended);
        assert_eq!(receiver.phase(), CallPhase::Closed);
        assert_eq!(caller.phase(), CallPhase::Closed);
        assert!(caller.end_local().unwrap().is_none());
        assert!(receiver.end_local().unwrap().is_none());
        assert_eq!(caller.receive(&end, b, LocalPeerPolicy::Allowed, NOW + 81),
            Err(CallError::Closed));
    }

    #[test]
    fn ring_does_not_accept_and_call_accept_is_not_device_permission() {
        let (mut caller, mut receiver, _) = started();
        let (_, b) = peers();
        assert_eq!(receiver.phase(), CallPhase::IncomingRinging);
        assert!(receiver.accept_local(NOW + 1).is_err());
        let ring = receiver.ring_local(NOW + 1).unwrap();
        assert_eq!(caller.receive(&ring, b, LocalPeerPolicy::Allowed, NOW + 1).unwrap(),
            RemoteEvent::Ringing);
        assert_eq!(caller.phase(), CallPhase::RemoteRinging);
        assert!(receiver.ring_local(NOW + 1).is_err());
        // There is NO method to start, capture, grant, or record any media.
        assert!(caller.notify_local_muted(false).is_err());
    }

    #[test]
    fn unknown_blocked_and_revoked_contact_denied() {
        let (a,b) = peers();
        for status in [LocalPeerPolicy::Unknown, LocalPeerPolicy::Blocked, LocalPeerPolicy::Revoked] {
            assert!(CallSession::outbound(a,b,[7;16],NOW,20,status).is_err());
            let (_,_,wire) = started();
            assert!(CallSession::incoming(&wire,a,b,status,NOW+1).is_err());
        }
    }

    #[test]
    fn forged_quic_endpoint_or_receiver_id_never_rings() {
        let (a,b) = peers();
        let (_,_,wire) = started();
        assert!(matches!(CallSession::incoming(
            &wire, peer(1,55), b, LocalPeerPolicy::Allowed, NOW+1
        ), Err(CallError::WrongPeer)));
        assert!(matches!(CallSession::incoming(
            &wire, a, peer(3,42), LocalPeerPolicy::Allowed, NOW+1
        ), Err(CallError::WrongPeer)));
        let (mut caller,mut receiver) = connected();
        let end = receiver.end_local().unwrap().unwrap();
        assert_eq!(caller.receive(&end, peer(2,77), LocalPeerPolicy::Allowed, NOW+5),
            Err(CallError::WrongPeer));
        assert_eq!(caller.phase(), CallPhase::Active);
    }

    #[test]
    fn replay_and_out_of_order_do_not_mutate_state() {
        let (mut caller, mut receiver, _) = started();
        let (_, b) = peers();
        let ring = receiver.ring_local(NOW+1).unwrap();
        let mut gap = ring;
        gap[OFF_SEQ..OFF_CREATED].copy_from_slice(&3u64.to_be_bytes());
        assert_eq!(caller.receive(&gap,b,LocalPeerPolicy::Allowed,NOW+1),
            Err(CallError::OutOfOrder));
        assert_eq!(caller.phase(), CallPhase::Dialing);
        assert_eq!(caller.receive(&ring,b,LocalPeerPolicy::Allowed,NOW+1).unwrap(),
            RemoteEvent::Ringing);
        assert_eq!(caller.receive(&ring,b,LocalPeerPolicy::Allowed,NOW+1),
            Err(CallError::Replay));
        assert_eq!(caller.phase(), CallPhase::RemoteRinging);
    }

    #[test]
    fn malformed_wire_version_flags_and_unknown_kind_are_fail_closed() {
        let (a,b)=peers();
        let (_,_,wire)=started();
        for index in [0,8,9,10] {
            let mut corrupt=wire;
            corrupt[index] ^= 0xff;
            assert!(CallSession::incoming(&corrupt,a,b,LocalPeerPolicy::Allowed,NOW+1).is_err());
        }
        assert_eq!(CallSession::incoming(&wire[..WIRE_BYTES-1],a,b,
                LocalPeerPolicy::Allowed,NOW+1).err(), Some(CallError::InvalidWire));
        let mut padded=wire.to_vec();
        padded.push(0);
        assert_eq!(CallSession::incoming(&padded,a,b,
                LocalPeerPolicy::Allowed,NOW+1).err(), Some(CallError::Oversize));
    }

    #[test]
    fn fresh_call_id_and_short_lifetime_are_required() {
        let (a,b)=peers();
        for (id,ttl) in [([0;16],30),([7;16],0),([7;16],121)] {
            assert!(CallSession::outbound(a,b,id,NOW,ttl,LocalPeerPolicy::Allowed).is_err());
        }
        assert!(CallSession::outbound(a,b,[7;16],u64::MAX,30,
            LocalPeerPolicy::Allowed).is_err());
        let (_,_,wire)=started();
        assert_eq!(CallSession::incoming(&wire,a,b,LocalPeerPolicy::Allowed,NOW-1).err(),
            Some(CallError::NotYetValid));
        assert_eq!(CallSession::incoming(&wire,a,b,LocalPeerPolicy::Allowed,NOW+60).err(),
            Some(CallError::Expired));
    }

    #[test]
    fn remote_reject_and_busy_and_caller_cancel() {
        let (a,b)=peers();
        for choice in [Refusal::Reject,Refusal::Busy] {
            let (mut caller,mut receiver,_) = started();
            let ring=receiver.ring_local(NOW+1).unwrap();
            caller.receive(&ring,b,LocalPeerPolicy::Allowed,NOW+1).unwrap();
            let refusal=receiver.refuse_local(choice,NOW+2).unwrap();
            let received=caller.receive(&refusal,b,LocalPeerPolicy::Allowed,NOW+2).unwrap();
            assert_eq!(received,if choice==Refusal::Reject {
                RemoteEvent::Rejected
            } else { RemoteEvent::Busy });
            assert_eq!(caller.phase(),CallPhase::Closed);
        }
        // A busy receiver may decline without sending an extra Ringing packet.
        let (mut early_caller, mut early_receiver, _) = started();
        let early_busy = early_receiver.refuse_local(Refusal::Busy, NOW+1).unwrap();
        assert_eq!(early_caller.receive(&early_busy,b,LocalPeerPolicy::Allowed,NOW+1).unwrap(),
            RemoteEvent::Busy);
        assert_eq!(early_receiver.phase(), CallPhase::Closed);
        let (mut early_caller, mut early_receiver, _) = started();
        let early_reject = early_receiver.end_local().unwrap().unwrap();
        assert_eq!(early_caller.receive(&early_reject,b,LocalPeerPolicy::Allowed,NOW+1).unwrap(),
            RemoteEvent::Rejected);
        let (mut caller,mut receiver,_)=started();
        let cancel=caller.end_local().unwrap().unwrap();
        assert_eq!(receiver.receive(&cancel,a,LocalPeerPolicy::Allowed,NOW+2).unwrap(),
            RemoteEvent::Cancelled);
        assert_eq!(receiver.phase(),CallPhase::Closed);
    }

    #[test]
    fn premature_accept_and_wrong_direction_are_rejected() {
        let (mut caller,mut receiver,_)=started();
        let (_,b)=peers();
        assert!(receiver.accept_local(NOW+1).is_err());
        let ring=receiver.ring_local(NOW+1).unwrap();
        let mut fake_accept=ring;
        fake_accept[9]=Kind::Accept.byte();
        assert_eq!(caller.receive(&fake_accept,b,LocalPeerPolicy::Allowed,NOW+1),
            Err(CallError::InvalidTransition));
        assert_eq!(caller.phase(),CallPhase::Dialing);
        let mut fake_cancel=ring;
        fake_cancel[9]=Kind::Cancel.byte();
        assert_eq!(caller.receive(&fake_cancel,b,LocalPeerPolicy::Allowed,NOW+1),
            Err(CallError::InvalidTransition));
    }

    #[test]
    fn wrong_call_id_and_different_peer_binding_fail_closed() {
        let (mut caller,mut receiver,_)=started();
        let (_,b)=peers();
        let ring=receiver.ring_local(NOW+1).unwrap();
        let mut wrong_id=ring;
        wrong_id[OFF_CALL]=19;
        assert_eq!(caller.receive(&wrong_id,b,LocalPeerPolicy::Allowed,NOW+1),
            Err(CallError::WrongCall));
        let mut wrong_endpoint=ring;
        wrong_endpoint[OFF_CALLEE_ENDPOINT]=3;
        assert_eq!(caller.receive(&wrong_endpoint,b,LocalPeerPolicy::Allowed,NOW+1),
            Err(CallError::WrongCall));
        assert_eq!(caller.phase(),CallPhase::Dialing);
    }

    #[test]
    fn remote_mute_is_display_only_and_never_restarts_capture() {
        let (mut caller,mut receiver)=connected();
        let (a,b)=peers();
        let status=receiver.notify_local_muted(false).unwrap();
        assert_eq!(caller.receive(&status,b,LocalPeerPolicy::Allowed,NOW+3).unwrap(),
            RemoteEvent::RemoteMuted(false));
        assert!(!caller.remote_muted());
        let status=caller.notify_local_muted(true).unwrap();
        assert_eq!(receiver.receive(&status,a,LocalPeerPolicy::Allowed,NOW+3).unwrap(),
            RemoteEvent::RemoteMuted(true));
        assert!(receiver.remote_muted());
        // No audio device, activation, storage or media encoder lives here.
    }

    #[test]
    fn revoke_during_active_closes_call_and_forbids_reanimation() {
        let (mut caller,mut receiver)=connected();
        let (_,b)=peers();
        let mute=receiver.notify_local_muted(false).unwrap();
        assert_eq!(caller.receive(&mute,b,LocalPeerPolicy::Revoked,NOW+3),
            Err(CallError::Revoked));
        assert_eq!(caller.phase(),CallPhase::Closed);
        assert_eq!(caller.receive(&mute,b,LocalPeerPolicy::Allowed,NOW+4),
            Err(CallError::Closed));
        assert!(caller.notify_local_muted(true).is_err());
        receiver.revoke();
        assert_eq!(receiver.phase(),CallPhase::Closed);
    }

    #[test]
    fn expired_ringing_never_accepts_but_active_can_end_later() {
        let (mut caller,mut receiver,_)=started();
        let (_,b)=peers();
        let ring=receiver.ring_local(NOW+1).unwrap();
        assert_eq!(caller.receive(&ring,b,LocalPeerPolicy::Allowed,NOW+60),
            Err(CallError::Expired));
        assert_eq!(caller.phase(),CallPhase::Closed);
        assert!(!caller.expire(NOW+61));
        let (mut caller,mut receiver)=connected();
        assert!(!caller.expire(NOW+90));
        let end=receiver.end_local().unwrap().unwrap();
        assert_eq!(caller.receive(&end,b,LocalPeerPolicy::Allowed,NOW+90).unwrap(),
            RemoteEvent::Ended);
    }

    #[test]
    fn no_implicit_reconnect_or_reuse_of_closed_session() {
        let (mut caller, mut receiver)=connected();
        let (_,b)=peers();
        assert!(caller.end_local().unwrap().is_some());
        assert!(caller.end_local().unwrap().is_none());
        assert_eq!(caller.receive(&receiver.notify_local_muted(true).unwrap(),
            b,LocalPeerPolicy::Allowed,NOW+2),Err(CallError::Closed));
        assert!(caller.accept_local(NOW+2).is_err());
    }

    #[test]
    fn late_ringing_and_accept_from_previous_call_do_not_activate_new_one() {
        let (a, b) = peers();
        let (mut old_caller, mut old_receiver, _) = started();
        let old_ring = old_receiver.ring_local(NOW + 1).unwrap();
        old_caller.receive(&old_ring, b, LocalPeerPolicy::Allowed, NOW + 1).unwrap();
        let old_accept = old_receiver.accept_local(NOW + 2).unwrap();

        // A new authenticated attempt needs a fresh call ID; stale packets
        // from the previous connection are not valid just because peers match.
        let (mut caller, offer) = CallSession::outbound(
            a, b, [8; 16], NOW + 10, 60, LocalPeerPolicy::Allowed
        ).unwrap();
        let mut receiver = CallSession::incoming(
            &offer, a, b, LocalPeerPolicy::Allowed, NOW + 11
        ).unwrap();
        for old in [old_ring, old_accept] {
            assert_eq!(
                caller.receive(&old, b, LocalPeerPolicy::Allowed, NOW + 11),
                Err(CallError::WrongCall)
            );
        }
        assert_eq!(caller.phase(), CallPhase::Dialing);
        let ringing = receiver.ring_local(NOW + 11).unwrap();
        assert_eq!(
            caller.receive(&ringing, b, LocalPeerPolicy::Allowed, NOW + 11),
            Ok(RemoteEvent::Ringing)
        );
        let accept = receiver.accept_local(NOW + 12).unwrap();
        assert_eq!(
            caller.receive(&accept, b, LocalPeerPolicy::Allowed, NOW + 12),
            Ok(RemoteEvent::Accepted)
        );
    }

    #[test]
    fn stale_end_cannot_close_successor_even_with_correct_sequence_number() {
        let (_, mut old_receiver) = connected();
        let old_end = old_receiver.end_local().unwrap().unwrap();
        let (a, b) = peers();
        let (mut caller, offer) = CallSession::outbound(
            a, b, [9; 16], NOW + 10, 60, LocalPeerPolicy::Allowed
        ).unwrap();
        let mut receiver = CallSession::incoming(
            &offer, a, b, LocalPeerPolicy::Allowed, NOW + 11
        ).unwrap();
        let ringing = receiver.ring_local(NOW + 11).unwrap();
        caller.receive(&ringing, b, LocalPeerPolicy::Allowed, NOW + 11).unwrap();
        let accept = receiver.accept_local(NOW + 12).unwrap();
        caller.receive(&accept, b, LocalPeerPolicy::Allowed, NOW + 12).unwrap();
        // Both old End and new End have receiver sequence=3.
        assert_eq!(
            caller.receive(&old_end, b, LocalPeerPolicy::Allowed, NOW + 13),
            Err(CallError::WrongCall)
        );
        assert_eq!(caller.phase(), CallPhase::Active);
        let real_end = receiver.end_local().unwrap().unwrap();
        assert_eq!(
            caller.receive(&real_end, b, LocalPeerPolicy::Allowed, NOW + 13),
            Ok(RemoteEvent::Ended)
        );
        assert_eq!(caller.phase(), CallPhase::Closed);
    }

    #[test]
    fn transport_loss_while_ringing_needs_fresh_offer_and_consent() {
        let (a, b) = peers();
        let (mut caller, mut receiver, _) = started();
        let old_ring = receiver.ring_local(NOW + 1).unwrap();
        caller.transport_lost();
        assert_eq!(caller.phase(), CallPhase::Closed);
        assert_eq!(
            caller.receive(&old_ring, b, LocalPeerPolicy::Allowed, NOW + 2),
            Err(CallError::Closed)
        );
        assert!(caller.end_local().unwrap().is_none());

        // Merely creating a new transport path does not revive the old call.
        let (mut next_caller, new_offer) = CallSession::outbound(
            a, b, [10; 16], NOW + 10, 60, LocalPeerPolicy::Allowed
        ).unwrap();
        let mut next_receiver = CallSession::incoming(
            &new_offer, a, b, LocalPeerPolicy::Allowed, NOW + 11
        ).unwrap();
        assert_eq!(
            next_caller.receive(&old_ring, b, LocalPeerPolicy::Allowed, NOW + 11),
            Err(CallError::WrongCall)
        );
        assert_eq!(next_caller.phase(), CallPhase::Dialing);
        let new_ring = next_receiver.ring_local(NOW + 11).unwrap();
        assert_eq!(
            next_caller.receive(&new_ring, b, LocalPeerPolicy::Allowed, NOW + 11),
            Ok(RemoteEvent::Ringing)
        );
    }

    #[test]
    fn transport_loss_after_accept_is_terminal_for_late_control_packets() {
        let (mut caller, mut receiver) = connected();
        let (_, b) = peers();
        let late_unmute = receiver.notify_local_muted(false).unwrap();
        let late_end = receiver.end_local().unwrap().unwrap();
        caller.transport_lost();
        assert_eq!(caller.phase(), CallPhase::Closed);
        assert!(caller.remote_muted());
        for late in [late_unmute, late_end] {
            assert_eq!(
                caller.receive(&late, b, LocalPeerPolicy::Allowed, NOW + 4),
                Err(CallError::Closed)
            );
        }
        assert_eq!(caller.notify_local_muted(false), Err(CallError::InvalidTransition));
        assert!(caller.end_local().unwrap().is_none());
        // The host, not this passive module, must stop microphone/camera access.
    }

    #[test]
    fn reordered_and_replayed_control_after_path_change_preserves_state() {
        let (mut caller, mut receiver) = connected();
        let (_, b) = peers();
        let unmute = receiver.notify_local_muted(false).unwrap();
        let mute = receiver.notify_local_muted(true).unwrap();

        // A delayed packet arriving on the new path cannot skip a sequence.
        assert_eq!(
            caller.receive(&mute, b, LocalPeerPolicy::Allowed, NOW + 4),
            Err(CallError::OutOfOrder)
        );
        assert!(caller.remote_muted());
        assert_eq!(
            caller.receive(&unmute, b, LocalPeerPolicy::Allowed, NOW + 4),
            Ok(RemoteEvent::RemoteMuted(false))
        );
        assert!(!caller.remote_muted());
        assert_eq!(
            caller.receive(&unmute, b, LocalPeerPolicy::Allowed, NOW + 4),
            Err(CallError::Replay)
        );
        assert_eq!(
            caller.receive(&mute, b, LocalPeerPolicy::Allowed, NOW + 4),
            Ok(RemoteEvent::RemoteMuted(true))
        );
        assert!(caller.remote_muted());
        assert_eq!(caller.phase(), CallPhase::Active);
    }

    #[test]
    fn expired_offer_stays_invalid_after_new_process_or_network_path() {
        let (a, b) = peers();
        let (_, _, old_offer) = started();
        // Freshly allocated session state is not permission to accept
        // an already expired OFFER replayed from an old network path.
        assert!(matches!(
            CallSession::incoming(
                &old_offer, a, b, LocalPeerPolicy::Allowed, NOW + 60
            ),
            Err(CallError::Expired)
        ));
        let (fresh, offer) = CallSession::outbound(
            a, b, [11; 16], NOW + 61, 60, LocalPeerPolicy::Allowed
        ).unwrap();
        let receiver = CallSession::incoming(
            &offer, a, b, LocalPeerPolicy::Allowed, NOW + 62
        ).unwrap();
        assert_eq!(fresh.phase(), CallPhase::Dialing);
        assert_eq!(receiver.phase(), CallPhase::IncomingRinging);
        // Durable anti-replay across restarts *within* TTL is still a host
        // requirement; this isolated laboratory does not implement it.
    }

    #[test]
    fn local_accept_preflight_denies_revoked_policy_after_ringing() {
        let (a, _) = peers();
        let (_, mut receiver, _) = started();
        receiver.ring_local(NOW + 1).unwrap();
        assert_eq!(
            receiver.verify_local_accept_peer(a, LocalPeerPolicy::Revoked),
            Err(CallError::Revoked)
        );
        assert_eq!(receiver.phase(), CallPhase::Closed);
        assert_eq!(receiver.accept_local(NOW + 2), Err(CallError::InvalidTransition));
    }

    #[test]
    fn local_accept_preflight_rejects_wrong_authenticated_peer() {
        let (_, mut receiver, _) = started();
        receiver.ring_local(NOW + 1).unwrap();
        assert_eq!(
            receiver.verify_local_accept_peer(peer(99, 99), LocalPeerPolicy::Allowed),
            Err(CallError::WrongPeer)
        );
        assert_eq!(receiver.phase(), CallPhase::Closed);
        assert_eq!(receiver.accept_local(NOW + 2), Err(CallError::InvalidTransition));
    }

    #[test]
    fn local_accept_preflight_keeps_allowed_session_ringing_until_consent() {
        let (a, _) = peers();
        let (_, mut receiver, _) = started();
        receiver.ring_local(NOW + 1).unwrap();
        assert_eq!(
            receiver.verify_local_accept_peer(a, LocalPeerPolicy::Allowed),
            Ok(())
        );
        assert_eq!(receiver.phase(), CallPhase::IncomingRinging);
        assert!(receiver.accept_local(NOW + 2).is_ok());
        assert_eq!(receiver.phase(), CallPhase::Active);
    }
