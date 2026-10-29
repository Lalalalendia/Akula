//! Non-shipping AK-CALL-01/02 authorization seam. No OS devices or socket.
/// Host-owned media permission is independent of network-call acceptance.
use crate::{
    call_signaling_lab::CallSession,
    p2p_offer_lab::PeerBinding,
    voice_jitter_lab::{PeerPolicy, VoiceError, VoiceReceiver, VoiceSender},
};

/// This function does NOT open devices. The host must separately verify
/// current OS peer pin, capture permission, and hardware lifetime at use time.
pub fn permit_media(
    call: &CallSession,
    host_consented: bool,
    authenticated_remote: PeerBinding,
    peer_policy: PeerPolicy,
    call_id: [u8; 16],
) -> Result<(), VoiceError> {
    if !host_consented || peer_policy != PeerPolicy::Allowed {
        return Err(VoiceError::Denied);
    }
    let Some((active_id, remote)) = call.active_media_identity() else {
        return Err(VoiceError::Denied);
    };
    if active_id != call_id {
        return Err(VoiceError::WrongCall);
    }
    if authenticated_remote != remote {
        return Err(VoiceError::WrongPeer);
    }
    Ok(())
}

/// An authorized, still-active call can allocate isolated packetizers.
/// They start muted; host OS microphone capture must remain OFF.
pub fn start_muted_media(
    call: &CallSession,
    host_consented: bool,
    remote: PeerBinding,
    policy: PeerPolicy,
    call_id: [u8; 16],
) -> Result<(VoiceSender, VoiceReceiver), VoiceError> {
    permit_media(call, host_consented, remote, policy, call_id)?;
    let sender = VoiceSender::new(call_id, 0)?;
    let receiver = VoiceReceiver::new(call_id, remote)?;
    Ok((sender, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        call_signaling_lab::{CallSession, LocalPeerPolicy, RemoteEvent},
        sync::DeviceId,
    };
    use uuid::Uuid;

    fn peer(n: u128, endpoint: u8) -> PeerBinding {
        PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(n)),
            endpoint_id: [endpoint; 32],
        }
    }

    #[test]
    fn no_capture_authority_before_accept_or_after_end() {
        let (a, b) = (peer(1, 1), peer(2, 2));
        let (mut caller, offer) =
            CallSession::outbound(a, b, [9; 16], 100, 60, LocalPeerPolicy::Allowed).unwrap();
        assert!(matches!(
            start_muted_media(&caller, true, b, PeerPolicy::Allowed, [9; 16]),
            Err(VoiceError::Denied)
        ));
        let mut callee =
            CallSession::incoming(&offer, a, b, LocalPeerPolicy::Allowed, 101).unwrap();
        let ring = callee.ring_local(101).unwrap();
        assert_eq!(
            caller.receive(&ring, b, LocalPeerPolicy::Allowed, 101),
            Ok(RemoteEvent::Ringing)
        );
        let accept = callee.accept_local(102).unwrap();
        assert_eq!(
            caller.receive(&accept, b, LocalPeerPolicy::Allowed, 102),
            Ok(RemoteEvent::Accepted)
        );
        assert!(matches!(
            start_muted_media(&caller, false, b, PeerPolicy::Allowed, [9; 16]),
            Err(VoiceError::Denied)
        ));
        assert!(matches!(
            start_muted_media(&caller, true, peer(3, 3), PeerPolicy::Allowed, [9; 16]),
            Err(VoiceError::WrongPeer)
        ));
        assert!(matches!(
            start_muted_media(&caller, true, b, PeerPolicy::Allowed, [7; 16]),
            Err(VoiceError::WrongCall)
        ));
        let (sender, receiver) =
            start_muted_media(&caller, true, b, PeerPolicy::Allowed, [9; 16]).unwrap();
        assert!(sender.is_muted());
        assert_eq!(receiver.queued_ticks(), 0);
        caller.transport_lost();
        assert!(matches!(
            start_muted_media(&caller, true, b, PeerPolicy::Allowed, [9; 16]),
            Err(VoiceError::Denied)
        ));
    }

    #[test]
    fn revoked_host_policy_never_allocates_media() {
        let (a, b) = (peer(1, 1), peer(2, 2));
        let (mut caller, offer) =
            CallSession::outbound(a, b, [8; 16], 100, 60, LocalPeerPolicy::Allowed).unwrap();
        let mut callee =
            CallSession::incoming(&offer, a, b, LocalPeerPolicy::Allowed, 101).unwrap();
        let ring = callee.ring_local(101).unwrap();
        caller.receive(&ring, b, LocalPeerPolicy::Allowed, 101).unwrap();
        let accept = callee.accept_local(102).unwrap();
        caller.receive(&accept, b, LocalPeerPolicy::Allowed, 102).unwrap();
        assert!(matches!(
            start_muted_media(&caller, true, b, PeerPolicy::Revoked, [8; 16]),
            Err(VoiceError::Denied)
        ));
    }
}
