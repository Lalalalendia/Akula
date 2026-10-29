use akula_call_capture_gate::{CaptureDevice, CaptureGate, OsPermission};
use aquilum_core::{
    call_signaling_lab::{CallSession, LocalPeerPolicy},
    p2p_offer_lab::PeerBinding,
    sync::DeviceId,
    voice_jitter_lab::{PeerPolicy, VoiceError},
};
use std::{cell::Cell, rc::Rc};
use uuid::Uuid;

struct Fake(Rc<Cell<u32>>);
impl CaptureDevice for Fake {
    fn stop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
fn peer(n: u128, key: u8) -> PeerBinding {
    PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_u128(n)),
        endpoint_id: [key; 32],
    }
}
fn active_call() -> (CallSession, PeerBinding) {
    let (a, b) = (peer(1, 1), peer(2, 2));
    let (mut caller, offer) =
        CallSession::outbound(a, b, [7; 16], 100, 60, LocalPeerPolicy::Allowed).unwrap();
    let mut callee = CallSession::incoming(&offer, a, b, LocalPeerPolicy::Allowed, 101).unwrap();
    let ring = callee.ring_local(101).unwrap();
    caller
        .receive(&ring, b, LocalPeerPolicy::Allowed, 101)
        .unwrap();
    let accept = callee.accept_local(102).unwrap();
    caller
        .receive(&accept, b, LocalPeerPolicy::Allowed, 102)
        .unwrap();
    (caller, b)
}
#[test]
fn rejects_os_denial_before_opening_device() {
    let (call, remote) = active_call();
    let result = CaptureGate::<Fake>::open(
        &call,
        true,
        OsPermission::Denied,
        remote,
        PeerPolicy::Allowed,
        [7; 16],
        || panic!("device opened"),
    );
    assert!(matches!(result, Err(VoiceError::Denied)));
}
#[test]
fn revoked_consent_and_disconnected_call_stop_exactly_once() {
    let (mut call, remote) = active_call();
    let stopped = Rc::new(Cell::new(0));
    let mut gate = CaptureGate::open(
        &call,
        true,
        OsPermission::Granted,
        remote,
        PeerPolicy::Allowed,
        [7; 16],
        || Ok(Fake(Rc::clone(&stopped))),
    )
    .unwrap();
    assert!(gate.active());
    assert!(gate
        .revalidate(
            &call,
            true,
            OsPermission::Granted,
            remote,
            PeerPolicy::Allowed,
            false,
        )
        .is_ok());
    call.transport_lost();
    assert!(gate
        .revalidate(
            &call,
            true,
            OsPermission::Granted,
            remote,
            PeerPolicy::Allowed,
            false,
        )
        .is_err());
    assert!(!gate.active());
    gate.close();
    assert_eq!(stopped.get(), 1);
}
#[test]
fn mute_or_changed_remote_closes_capture() {
    for altered in [false, true] {
        let (call, remote) = active_call();
        let count = Rc::new(Cell::new(0));
        let mut gate = CaptureGate::open(
            &call,
            true,
            OsPermission::Granted,
            remote,
            PeerPolicy::Allowed,
            [7; 16],
            || Ok(Fake(Rc::clone(&count))),
        )
        .unwrap();
        let other = peer(3, 3);
        let result = gate.revalidate(
            &call,
            true,
            OsPermission::Granted,
            if altered { other } else { remote },
            PeerPolicy::Allowed,
            !altered,
        );
        assert!(result.is_err());
        assert_eq!(count.get(), 1);
    }
}

#[test]
fn consent_os_permission_or_policy_revocation_stops_capture_once() {
    for (consent, os_permission, policy) in [
        (false, OsPermission::Granted, PeerPolicy::Allowed),
        (true, OsPermission::Denied, PeerPolicy::Allowed),
        (true, OsPermission::Granted, PeerPolicy::Revoked),
    ] {
        let (call, remote) = active_call();
        let stopped = Rc::new(Cell::new(0));
        let mut gate = CaptureGate::open(
            &call,
            true,
            OsPermission::Granted,
            remote,
            PeerPolicy::Allowed,
            [7; 16],
            || Ok(Fake(Rc::clone(&stopped))),
        )
        .unwrap();
        assert!(matches!(
            gate.revalidate(&call, consent, os_permission, remote, policy, false),
            Err(VoiceError::Denied)
        ));
        assert!(!gate.active());
        gate.close();
        drop(gate);
        assert_eq!(stopped.get(), 1, "every revoke path stops exactly once");
    }
}

#[test]
fn invalid_peer_or_call_id_never_invokes_capture_factory() {
    let (call, remote) = active_call();
    let wrong_peer = CaptureGate::<Fake>::open(
        &call,
        true,
        OsPermission::Granted,
        peer(3, 3),
        PeerPolicy::Allowed,
        [7; 16],
        || panic!("wrong peer must never open capture"),
    );
    assert!(matches!(wrong_peer, Err(VoiceError::WrongPeer)));

    let wrong_call = CaptureGate::<Fake>::open(
        &call,
        true,
        OsPermission::Granted,
        remote,
        PeerPolicy::Allowed,
        [9; 16],
        || panic!("wrong call must never open capture"),
    );
    assert!(matches!(wrong_call, Err(VoiceError::WrongCall)));
}

#[test]
fn dropping_authorized_gate_closes_the_device_once() {
    let (call, remote) = active_call();
    let stopped = Rc::new(Cell::new(0));
    {
        let gate = CaptureGate::open(
            &call,
            true,
            OsPermission::Granted,
            remote,
            PeerPolicy::Allowed,
            [7; 16],
            || Ok(Fake(Rc::clone(&stopped))),
        )
        .unwrap();
        assert!(gate.active());
    }
    assert_eq!(stopped.get(), 1, "Drop must not retain the capture device");
}
