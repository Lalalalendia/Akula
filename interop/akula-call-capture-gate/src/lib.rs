//! Passive call-to-device authorization. No automatic OS capture.
use aquilum_core::{
    call_authorized_media_lab::permit_media,
    call_signaling_lab::CallSession,
    p2p_offer_lab::PeerBinding,
    voice_jitter_lab::{PeerPolicy, VoiceError},
};

/// The host's current OS permission; never learned from a network frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OsPermission {
    Granted,
    Denied,
}

pub trait CaptureDevice {
    fn stop(&mut self);
}

pub struct CaptureGate<D: CaptureDevice> {
    device: Option<D>,
    call_id: [u8; 16],
    remote: PeerBinding,
}

impl<D: CaptureDevice> CaptureGate<D> {
    /// Open only when all independent authorities agree. A host must still
    /// recheck actual Windows permissions at the point of device creation.
    pub fn open(
        call: &CallSession,
        consent: bool,
        os_permission: OsPermission,
        remote: PeerBinding,
        policy: PeerPolicy,
        call_id: [u8; 16],
        device_factory: impl FnOnce() -> Result<D, VoiceError>,
    ) -> Result<Self, VoiceError> {
        if os_permission != OsPermission::Granted {
            return Err(VoiceError::Denied);
        }
        permit_media(call, consent, remote, policy, call_id)?;
        let device = device_factory()?;
        Ok(Self {
            device: Some(device),
            call_id,
            remote,
        })
    }

    /// Call on every host media event, before any audio frame may be consumed.
    /// Revocation, changed identity, lost transport and mute destroy the device.
    pub fn revalidate(
        &mut self,
        call: &CallSession,
        consent: bool,
        os_permission: OsPermission,
        remote: PeerBinding,
        policy: PeerPolicy,
        muted: bool,
    ) -> Result<(), VoiceError> {
        if self.device.is_none() {
            return Err(VoiceError::Closed);
        }
        let verdict = if muted || os_permission != OsPermission::Granted {
            Err(VoiceError::Denied)
        } else {
            permit_media(call, consent, remote, policy, self.call_id)
        };
        if verdict.is_err() || remote != self.remote {
            self.close();
            return verdict.and(Err(VoiceError::WrongPeer));
        }
        Ok(())
    }

    pub fn active(&self) -> bool {
        self.device.is_some()
    }

    pub fn close(&mut self) {
        if let Some(mut device) = self.device.take() {
            device.stop();
        }
    }
}

impl<D: CaptureDevice> Drop for CaptureGate<D> {
    fn drop(&mut self) {
        self.close();
    }
}
