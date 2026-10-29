//! AK-CALL-02: default-OFF, in-memory Opus-payload and jitter-buffer contract.
//!
//! This is NOT an Opus encoder/decoder, AEC, PLC implementation, microphone
//! callback, OS device permission, QUIC transport, encryption or live call.
//! The host MUST authenticate the remote QUIC PeerBinding independently,
//! establish a locally accepted call (AK-CALL-01), and continually enforce
//! contact trust and microphone consent. Receiving packets cannot start capture.
//! Only a 20 ms/48 kHz mono packet-duration profile is modelled here; actual
//! codecs, device sample-rate conversion, negotiated modes and AEC are future
//! independently tested integrations. All clocks are supplied by the host.

#![deny(clippy::all)]

use crate::p2p_offer_lab::PeerBinding;
use std::collections::BTreeMap;

pub const FRAME_MS: u64 = 20;
pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const SAMPLES_PER_FRAME: usize = 960;
pub const MAX_OPUS_BYTES: usize = 1275;
pub const HEADER_BYTES: usize = 40;
pub const MAX_WIRE_BYTES: usize = HEADER_BYTES + MAX_OPUS_BYTES;
pub const MAX_SILENCE_TICKS: u8 = 10;
pub const MAX_PENDING_TICKS: usize = 24;
pub const MAX_AHEAD_TICKS: u64 = 32;
pub const START_DELAY_MS: u64 = 60;
pub const MAX_PLAYOUT_LAG_MS: u64 = 60;
const MAGIC: &[u8; 8] = b"AKVOICE1";
const VERSION: u8 = 1;
const OPUS: u8 = 1;
const SILENCE: u8 = 2;
const OFF_CALL: usize = 12;
const OFF_TICK: usize = 28;
const OFF_SPAN: usize = 36;
const OFF_LENGTH: usize = 38;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoiceError {
    InvalidWire,
    UnsupportedVersion,
    InvalidSession,
    InvalidPacket,
    WrongCall,
    WrongPeer,
    Denied,
    Revoked,
    Closed,
    Replay,
    Late,
    TooFarAhead,
    BufferFull,
    InvalidClock,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerPolicy {
    /// Advisory host input, never a permission created from media packets.
    Allowed,
    Denied,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Payload {
    /// Opaque 20 ms Opus packet. No codec parsing or sound production occurs.
    Opus(Vec<u8>),
    /// A previously determined silent/DTX span; NOT a voice activity detector.
    Silence { ticks: u8 },
}

impl Payload {
    fn ticks(&self) -> u8 {
        match self {
            Self::Opus(_) => 1,
            Self::Silence { ticks } => *ticks,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoicePacket {
    call_id: [u8; 16],
    tick: u64,
    payload: Payload,
}

impl VoicePacket {
    pub fn opus(call_id: [u8; 16], tick: u64, bytes: Vec<u8>) -> Result<Self, VoiceError> {
        let frame = Self { call_id, tick, payload: Payload::Opus(bytes) };
        frame.validate()?;
        Ok(frame)
    }

    pub fn silence(call_id: [u8; 16], tick: u64, ticks: u8) -> Result<Self, VoiceError> {
        let frame = Self { call_id, tick, payload: Payload::Silence { ticks } };
        frame.validate()?;
        Ok(frame)
    }

    pub fn tick(&self) -> u64 { self.tick }
    pub fn call_id(&self) -> [u8; 16] { self.call_id }
    pub fn payload(&self) -> &Payload { &self.payload }

    fn validate(&self) -> Result<(), VoiceError> {
        if self.call_id == [0; 16] {
            return Err(VoiceError::InvalidSession);
        }
        let span = self.payload.ticks();
        if self.tick.checked_add(u64::from(span)).is_none() {
            return Err(VoiceError::SequenceExhausted);
        }
        match &self.payload {
            Payload::Opus(bytes) if !bytes.is_empty() && bytes.len() <= MAX_OPUS_BYTES => Ok(()),
            Payload::Silence { ticks } if (1..=MAX_SILENCE_TICKS).contains(ticks) => Ok(()),
            _ => Err(VoiceError::InvalidPacket),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, VoiceError> {
        self.validate()?;
        let (kind, opus): (u8, &[u8]) = match &self.payload {
            Payload::Opus(bytes) => (OPUS, bytes.as_slice()),
            Payload::Silence { .. } => (SILENCE, &[]),
        };
        let mut wire = vec![0u8; HEADER_BYTES + opus.len()];
        wire[..8].copy_from_slice(MAGIC);
        wire[8] = VERSION;
        wire[9] = kind;
        wire[OFF_CALL..OFF_TICK].copy_from_slice(&self.call_id);
        wire[OFF_TICK..OFF_SPAN].copy_from_slice(&self.tick.to_be_bytes());
        wire[OFF_SPAN] = self.payload.ticks();
        wire[OFF_LENGTH..HEADER_BYTES].copy_from_slice(&(opus.len() as u16).to_be_bytes());
        wire[HEADER_BYTES..].copy_from_slice(opus);
        Ok(wire)
    }

    pub fn decode(wire: &[u8]) -> Result<Self, VoiceError> {
        if wire.len() < HEADER_BYTES || wire.len() > MAX_WIRE_BYTES {
            return Err(VoiceError::InvalidWire);
        }
        if &wire[..8] != MAGIC || wire[8] != VERSION ||
            wire[10] != 0 || wire[11] != 0 || wire[37] != 0
        {
            return Err(VoiceError::UnsupportedVersion);
        }
        let kind = wire[9];
        let len = u16::from_be_bytes([wire[OFF_LENGTH], wire[OFF_LENGTH + 1]]) as usize;
        if len != wire.len() - HEADER_BYTES {
            return Err(VoiceError::InvalidWire);
        }
        let mut call_id = [0u8; 16];
        call_id.copy_from_slice(&wire[OFF_CALL..OFF_TICK]);
        let tick = u64::from_be_bytes(wire[OFF_TICK..OFF_SPAN].try_into()
            .map_err(|_| VoiceError::InvalidWire)?);
        let span = wire[OFF_SPAN];
        match kind {
            OPUS if span == 1 => Self::opus(call_id, tick, wire[HEADER_BYTES..].to_vec()),
            SILENCE if len == 0 => Self::silence(call_id, tick, span),
            _ => Err(VoiceError::InvalidPacket),
        }
    }
}

/// Packetization from already-encoded Opus bytes only. Does not read a device.
/// Local mute maps caller-provided bytes to silence; this alone does NOT stop
/// an external capture callback, which the OS-backed host must stop separately.
pub struct VoiceSender {
    call_id: [u8; 16],
    next_tick: u64,
    muted: bool,
    closed: bool,
}

impl VoiceSender {
    pub fn new(call_id: [u8; 16], start_tick: u64) -> Result<Self, VoiceError> {
        if call_id == [0; 16] || start_tick == u64::MAX {
            return Err(VoiceError::InvalidSession);
        }
        Ok(Self { call_id, next_tick: start_tick, muted: true, closed: false })
    }

    pub fn set_muted(&mut self, muted: bool) -> Result<(), VoiceError> {
        if self.closed { return Err(VoiceError::Closed); }
        self.muted = muted;
        Ok(())
    }

    pub fn is_muted(&self) -> bool { self.muted }
    pub fn next_tick(&self) -> u64 { self.next_tick }

    pub fn send_opus(&mut self, encoded_opus: &[u8]) -> Result<Vec<u8>, VoiceError> {
        if self.closed { return Err(VoiceError::Closed); }
        if self.muted { return self.send_silence(1); }
        let packet = VoicePacket::opus(self.call_id, self.next_tick, encoded_opus.to_vec())?;
        let next = self.next_tick.checked_add(1).ok_or(VoiceError::SequenceExhausted)?;
        let wire = packet.encode()?;
        self.next_tick = next;
        Ok(wire)
    }

    /// Only call after independent VAD/DTX determination; never guesses speech.
    pub fn send_silence(&mut self, ticks: u8) -> Result<Vec<u8>, VoiceError> {
        if self.closed { return Err(VoiceError::Closed); }
        let packet = VoicePacket::silence(self.call_id, self.next_tick, ticks)?;
        let next = self.next_tick.checked_add(u64::from(ticks))
            .ok_or(VoiceError::SequenceExhausted)?;
        let wire = packet.encode()?;
        self.next_tick = next;
        Ok(wire)
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.muted = true;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Slot {
    Opus(Vec<u8>),
    Silent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Playout {
    /// Wait for initial 60ms jitter allowance or next scheduled deadline.
    Waiting,
    /// Feed these opaque bytes into an independently reviewed Opus decoder.
    Opus { tick: u64, bytes: Vec<u8> },
    /// Produce digital silence; no Opus decoder or device is invoked here.
    Silence { tick: u64 },
    /// Ask the future decoder for PLC; this module does NOT synthesize audio.
    Conceal { tick: u64 },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VoiceMetrics {
    pub received_packets: u64,
    pub duplicate_packets: u64,
    pub late_packets: u64,
    pub future_packets: u64,
    pub full_packets: u64,
    pub played_opus_ticks: u64,
    pub played_silent_ticks: u64,
    pub conceal_ticks: u64,
    pub skipped_stale_ticks: u64,
    pub peak_buffered_ticks: usize,
}

/// All operations are NON-realtime: a BTreeMap and Vec allocations are used.
/// Do not invoke from an audio callback; only from a bounded async/media task.
/// A receiver is tied to ONE externally accepted call/QUIC-authenticated peer.
/// This type never opens an endpoint, negotiates trust, captures or plays sound.
pub struct VoiceReceiver {
    call_id: [u8; 16],
    authenticated_remote: PeerBinding,
    queued: BTreeMap<u64, Slot>,
    next_tick: Option<u64>,
    next_deadline_ms: Option<u64>,
    last_arrival_ms: Option<u64>,
    last_poll_ms: Option<u64>,
    metrics: VoiceMetrics,
    closed: bool,
}

impl VoiceReceiver {
    pub fn new(call_id: [u8; 16], authenticated_remote: PeerBinding)
        -> Result<Self, VoiceError>
    {
        if call_id == [0; 16] || authenticated_remote.endpoint_id == [0; 32] ||
            authenticated_remote.device_id.as_uuid().is_nil()
        {
            return Err(VoiceError::InvalidSession);
        }
        Ok(Self {
            call_id, authenticated_remote,
            queued: BTreeMap::new(), next_tick: None,
            next_deadline_ms: None, last_arrival_ms: None,
            last_poll_ms: None, metrics: VoiceMetrics::default(),
            closed: false,
        })
    }

    pub fn metrics(&self) -> VoiceMetrics { self.metrics }
    pub fn queued_ticks(&self) -> usize { self.queued.len() }
    pub fn is_closed(&self) -> bool { self.closed }

    pub fn receive(
        &mut self,
        wire: &[u8],
        independently_authenticated_sender: PeerBinding,
        local_policy: PeerPolicy,
        arrival_ms: u64,
    ) -> Result<(), VoiceError> {
        if self.closed { return Err(VoiceError::Closed); }
        if local_policy != PeerPolicy::Allowed {
            self.close();
            return Err(if local_policy == PeerPolicy::Revoked {
                VoiceError::Revoked
            } else { VoiceError::Denied });
        }
        if independently_authenticated_sender != self.authenticated_remote {
            return Err(VoiceError::WrongPeer);
        }
        if self.last_arrival_ms.is_some_and(|last| arrival_ms < last) {
            return Err(VoiceError::InvalidClock);
        }
        let packet = VoicePacket::decode(wire)?;
        if packet.call_id != self.call_id {
            return Err(VoiceError::WrongCall);
        }
        let span = u64::from(packet.payload.ticks());
        // While startup is still buffering, an earlier tick may arrive
        // after the first packet. Do not rewind after playout has begun.
        let start = match self.next_tick {
            Some(first) if packet.tick < first => {
                let still_starting = self.metrics.played_opus_ticks == 0
                    && self.metrics.played_silent_ticks == 0
                    && self.metrics.conceal_ticks == 0
                    && self.next_deadline_ms.is_some_and(|d| arrival_ms < d);
                if !still_starting || first - packet.tick > START_DELAY_MS / FRAME_MS {
                    self.metrics.late_packets = self.metrics.late_packets.saturating_add(1);
                    return Err(VoiceError::Late);
                }
                if self.queued.last_key_value().is_some_and(
                    |(&last, _)| last - packet.tick >= MAX_AHEAD_TICKS
                ) {
                    self.metrics.future_packets = self.metrics.future_packets.saturating_add(1);
                    return Err(VoiceError::TooFarAhead);
                }
                packet.tick
            }
            Some(first) => first,
            None => packet.tick,
        };
        let end = packet.tick.checked_add(span).ok_or(VoiceError::SequenceExhausted)?;
        if end - 1 - start >= MAX_AHEAD_TICKS {
            self.metrics.future_packets = self.metrics.future_packets.saturating_add(1);
            return Err(VoiceError::TooFarAhead);
        }
        if self.queued.len() + span as usize > MAX_PENDING_TICKS {
            self.metrics.full_packets = self.metrics.full_packets.saturating_add(1);
            return Err(VoiceError::BufferFull);
        }
        for tick in packet.tick..end {
            if self.queued.contains_key(&tick) {
                self.metrics.duplicate_packets = self.metrics.duplicate_packets.saturating_add(1);
                return Err(VoiceError::Replay);
            }
        }
        let deadline = if self.next_tick.is_none() {
            Some(arrival_ms.checked_add(START_DELAY_MS).ok_or(VoiceError::InvalidClock)?)
        } else { None };
        let Payload::Opus(ref bytes) = packet.payload else {
            for tick in packet.tick..end { self.queued.insert(tick, Slot::Silent); }
            self.next_tick = Some(start);
            if let Some(deadline) = deadline {
                self.next_deadline_ms = Some(deadline);
            }
            self.last_arrival_ms = Some(arrival_ms);
            self.metrics.received_packets = self.metrics.received_packets.saturating_add(1);
            self.metrics.peak_buffered_ticks = self.metrics.peak_buffered_ticks.max(self.queued.len());
            return Ok(());
        };
        self.queued.insert(packet.tick, Slot::Opus(bytes.clone()));
        self.next_tick = Some(start);
        if let Some(deadline) = deadline {
            self.next_deadline_ms = Some(deadline);
        }
        self.last_arrival_ms = Some(arrival_ms);
        self.metrics.received_packets = self.metrics.received_packets.saturating_add(1);
        self.metrics.peak_buffered_ticks = self.metrics.peak_buffered_ticks.max(self.queued.len());
        Ok(())
    }

    /// One 20ms playout quantum. If the caller wakes too late, discard stale
    /// packets rather than draining a growing backlog. host_now_ms MUST be
    /// monotonic; this function is not for OS real-time callback invocation.
    pub fn poll(&mut self, host_now_ms: u64) -> Result<Playout, VoiceError> {
        if self.closed { return Err(VoiceError::Closed); }
        if self.last_poll_ms.is_some_and(|last| host_now_ms < last) {
            return Err(VoiceError::InvalidClock);
        }
        self.last_poll_ms = Some(host_now_ms);
        let Some(mut tick) = self.next_tick else {
            return Ok(Playout::Waiting);
        };
        let mut deadline = self.next_deadline_ms.ok_or(VoiceError::InvalidClock)?;
        if host_now_ms < deadline { return Ok(Playout::Waiting); }

        if host_now_ms.saturating_sub(deadline) > MAX_PLAYOUT_LAG_MS {
            let skip = (host_now_ms - deadline) / FRAME_MS;
            let step_ms = skip.checked_mul(FRAME_MS).ok_or(VoiceError::InvalidClock)?;
            tick = tick.checked_add(skip).ok_or(VoiceError::SequenceExhausted)?;
            deadline = deadline.checked_add(step_ms).ok_or(VoiceError::InvalidClock)?;
            self.queued.retain(|key, _| *key >= tick);
            self.metrics.skipped_stale_ticks =
                self.metrics.skipped_stale_ticks.saturating_add(skip);
        }
        let next_tick = tick.checked_add(1).ok_or(VoiceError::SequenceExhausted)?;
        let next_deadline_ms = deadline.checked_add(FRAME_MS).ok_or(VoiceError::InvalidClock)?;
        let event = match self.queued.remove(&tick) {
            Some(Slot::Opus(bytes)) => {
                self.metrics.played_opus_ticks = self.metrics.played_opus_ticks.saturating_add(1);
                Playout::Opus { tick, bytes }
            }
            Some(Slot::Silent) => {
                self.metrics.played_silent_ticks = self.metrics.played_silent_ticks.saturating_add(1);
                Playout::Silence { tick }
            }
            None => {
                self.metrics.conceal_ticks = self.metrics.conceal_ticks.saturating_add(1);
                Playout::Conceal { tick }
            }
        };
        self.next_tick = Some(next_tick);
        self.next_deadline_ms = Some(next_deadline_ms);
        Ok(event)
    }

    /// Must be called by host on end, contact revocation, or consent removal.
    /// Idempotent; no remote media packet can reopen this receiver.
    pub fn close(&mut self) {
        self.closed = true;
        self.queued.clear();
        self.next_tick = None;
        self.next_deadline_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::DeviceId;
    use uuid::Uuid;

    const CALL: [u8; 16] = [7; 16];
    fn peer(n: u128, endpoint: u8) -> PeerBinding {
        PeerBinding { device_id: DeviceId::from_uuid(Uuid::from_u128(n)),
            endpoint_id: [endpoint; 32] }
    }
    fn sender() -> VoiceSender { VoiceSender::new(CALL, 0).unwrap() }
    fn receiver() -> VoiceReceiver { VoiceReceiver::new(CALL, peer(2, 42)).unwrap() }
    fn audio(tick: u64, byte: u8) -> Vec<u8> {
        VoicePacket::opus(CALL, tick, vec![byte; 5]).unwrap().encode().unwrap()
    }
    fn admit(receiver: &mut VoiceReceiver, wire: &[u8], at: u64) -> Result<(), VoiceError> {
        receiver.receive(wire, peer(2, 42), PeerPolicy::Allowed, at)
    }

    #[test]
    fn voice_packet_opus_roundtrip_and_exact_length() {
        let pkt = VoicePacket::opus(CALL, 123, vec![1, 2, 3, 4]).unwrap();
        let bytes = pkt.encode().unwrap();
        assert_eq!(bytes.len(), HEADER_BYTES + 4);
        assert_eq!(VoicePacket::decode(&bytes), Ok(pkt));
        assert_eq!(VoicePacket::decode(&bytes[..HEADER_BYTES - 1]), Err(VoiceError::InvalidWire));
        let mut excess = bytes;
        excess.push(0);
        assert_eq!(VoicePacket::decode(&excess), Err(VoiceError::InvalidWire));
    }

    #[test]
    fn empty_or_oversized_opus_and_bad_dtx_span_rejected() {
        assert!(matches!(VoicePacket::opus(CALL, 1, vec![]), Err(VoiceError::InvalidPacket)));
        assert!(matches!(VoicePacket::opus(CALL, 1, vec![0; MAX_OPUS_BYTES + 1]),
            Err(VoiceError::InvalidPacket)));
        for span in [0, MAX_SILENCE_TICKS + 1] {
            assert_eq!(VoicePacket::silence(CALL, 1, span), Err(VoiceError::InvalidPacket));
        }
    }

    #[test]
    fn corrupt_magic_version_flags_kind_and_length_fail_closed() {
        let frame = audio(1, 3);
        for index in [0usize, 8, 9, 10, 11, 37, 38, 39] {
            let mut corrupt = frame.clone();
            corrupt[index] ^= 0xff;
            assert!(VoicePacket::decode(&corrupt).is_err(), "index {index}");
        }
        let oversized = vec![0u8; MAX_WIRE_BYTES + 1];
        assert_eq!(VoicePacket::decode(&oversized), Err(VoiceError::InvalidWire));
    }

    #[test]
    fn invalid_call_id_and_sequence_overflow_refused() {
        assert_eq!(VoiceSender::new([0; 16], 0).err(), Some(VoiceError::InvalidSession));
        assert_eq!(VoicePacket::opus([0; 16], 1, vec![2]), Err(VoiceError::InvalidSession));
        assert_eq!(VoicePacket::opus(CALL, u64::MAX, vec![2]),
            Err(VoiceError::SequenceExhausted));
        assert_eq!(VoicePacket::silence(CALL, u64::MAX - 5, 10),
            Err(VoiceError::SequenceExhausted));
        assert_eq!(VoiceReceiver::new(CALL, peer(2, 0)).err(),
            Some(VoiceError::InvalidSession));
    }

    #[test]
    fn sender_is_muted_by_default_and_never_publishes_supplied_audio() {
        let mut s = sender();
        assert!(s.is_muted());
        let muted = s.send_opus(&[91, 92, 93]).unwrap();
        assert_eq!(muted.len(), HEADER_BYTES);
        assert!(matches!(VoicePacket::decode(&muted).unwrap().payload(), Payload::Silence { ticks: 1 }));
        s.set_muted(false).unwrap();
        assert_eq!(s.next_tick(), 1);
        let voice = s.send_opus(&[91, 92, 93]).unwrap();
        assert_eq!(VoicePacket::decode(&voice).unwrap().payload(),
            &Payload::Opus(vec![91, 92, 93]));
        assert_eq!(s.next_tick(), 2);
    }

    #[test]
    fn same_authenticated_peer_and_call_id_are_mandatory() {
        let mut r = receiver();
        let frame = audio(0, 3);
        assert_eq!(r.receive(&frame, peer(3, 43), PeerPolicy::Allowed, 100),
            Err(VoiceError::WrongPeer));
        let different_call = VoicePacket::opus([8; 16], 0, vec![9]).unwrap().encode().unwrap();
        assert_eq!(admit(&mut r, &different_call, 100), Err(VoiceError::WrongCall));
        assert_eq!(r.queued_ticks(), 0);
        assert_eq!(admit(&mut r, &frame, 100), Ok(()));
        assert_eq!(r.queued_ticks(), 1);
    }

    #[test]
    fn two_passive_participants_exchange_opus_without_device_access() {
        let mut alice = VoiceSender::new(CALL, 10).unwrap();
        let mut bob = VoiceSender::new(CALL, 30).unwrap();
        alice.set_muted(false).unwrap();
        bob.set_muted(false).unwrap();
        let mut rx_a = VoiceReceiver::new(CALL, peer(2, 42)).unwrap();
        let mut rx_b = VoiceReceiver::new(CALL, peer(1, 41)).unwrap();
        assert_eq!(rx_a.receive(&bob.send_opus(&[2, 2]).unwrap(), peer(2, 42),
            PeerPolicy::Allowed, 100), Ok(()));
        assert_eq!(rx_b.receive(&alice.send_opus(&[1, 1]).unwrap(), peer(1, 41),
            PeerPolicy::Allowed, 101), Ok(()));
        assert_eq!(rx_a.poll(160), Ok(Playout::Opus { tick: 30, bytes: vec![2, 2] }));
        assert_eq!(rx_b.poll(161), Ok(Playout::Opus { tick: 10, bytes: vec![1, 1] }));
    }

    #[test]
    fn waits_sixty_ms_then_plays_one_quantum_at_each_deadline() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 7), 100).unwrap();
        admit(&mut r, &audio(1, 8), 120).unwrap();
        assert_eq!(r.poll(159), Ok(Playout::Waiting));
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 0, bytes: vec![7; 5] }));
        assert_eq!(r.poll(179), Ok(Playout::Waiting));
        assert_eq!(r.poll(180), Ok(Playout::Opus { tick: 1, bytes: vec![8; 5] }));
        assert_eq!(r.metrics().played_opus_ticks, 2);
    }

    #[test]
    fn late_reordered_packet_is_accepted_before_playout() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        admit(&mut r, &audio(2, 3), 110).unwrap();
        admit(&mut r, &audio(1, 2), 115).unwrap();
        assert!(matches!(r.poll(160), Ok(Playout::Opus { tick: 0, .. })));
        assert!(matches!(r.poll(180), Ok(Playout::Opus { tick: 1, .. })));
        assert!(matches!(r.poll(200), Ok(Playout::Opus { tick: 2, .. })));
    }

    #[test]
    fn startup_reorders_first_speech_frames_before_deadline() {
        let mut r = receiver();
        admit(&mut r, &audio(2, 3), 100).unwrap();
        admit(&mut r, &audio(0, 1), 110).unwrap();
        admit(&mut r, &audio(1, 2), 120).unwrap();
        assert_eq!(r.poll(159), Ok(Playout::Waiting));
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 0, bytes: vec![1; 5] }));
        assert_eq!(r.poll(180), Ok(Playout::Opus { tick: 1, bytes: vec![2; 5] }));
        assert_eq!(r.poll(200), Ok(Playout::Opus { tick: 2, bytes: vec![3; 5] }));
    }

    #[test]
    fn startup_rewind_is_bounded_and_keeps_future_window_safe() {
        let mut r = receiver();
        admit(&mut r, &audio(10, 1), 100).unwrap();
        assert_eq!(admit(&mut r, &audio(6, 2), 110), Err(VoiceError::Late));
        admit(&mut r, &audio(9, 3), 120).unwrap();
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 9, bytes: vec![3; 5] }));
        let mut w = receiver();
        admit(&mut w, &audio(2, 1), 100).unwrap();
        admit(&mut w, &audio(33, 2), 110).unwrap();
        assert_eq!(admit(&mut w, &audio(0, 3), 120), Err(VoiceError::TooFarAhead));
        assert_eq!(w.queued_ticks(), 2);
    }

    #[test]
    fn expired_startup_and_started_playout_never_rewind() {
        let mut r = receiver();
        admit(&mut r, &audio(2, 1), 100).unwrap();
        assert_eq!(admit(&mut r, &audio(1, 2), 160), Err(VoiceError::Late));
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 2, bytes: vec![1; 5] }));
        assert_eq!(admit(&mut r, &audio(0, 3), 170), Err(VoiceError::Late));
    }

    #[test]
    fn dropped_packet_yields_plc_request_not_fake_decoded_audio() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        admit(&mut r, &audio(2, 3), 110).unwrap();
        assert!(matches!(r.poll(160), Ok(Playout::Opus { tick: 0, .. })));
        assert_eq!(r.poll(180), Ok(Playout::Conceal { tick: 1 }));
        assert!(matches!(r.poll(200), Ok(Playout::Opus { tick: 2, .. })));
        assert_eq!(r.metrics().conceal_ticks, 1);
        assert_eq!(admit(&mut r, &audio(1, 2), 210), Err(VoiceError::Late));
    }

    #[test]
    fn duplicates_are_rejected_without_replacing_original() {
        let mut r = receiver();
        let a = audio(0, 1);
        admit(&mut r, &a, 100).unwrap();
        assert_eq!(admit(&mut r, &audio(0, 8), 110), Err(VoiceError::Replay));
        assert_eq!(r.metrics().duplicate_packets, 1);
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 0, bytes: vec![1; 5] }));
    }

    #[test]
    fn dtx_silence_is_bounded_and_cannot_overlap_an_opus_tick() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        let silence = VoicePacket::silence(CALL, 1, 4).unwrap().encode().unwrap();
        admit(&mut r, &silence, 110).unwrap();
        assert_eq!(admit(&mut r, &audio(3, 9), 115), Err(VoiceError::Replay));
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 0, bytes: vec![1; 5] }));
        for t in 1..=4 {
            assert_eq!(r.poll(160 + 20 * t), Ok(Playout::Silence { tick: t }));
        }
        assert_eq!(r.metrics().played_silent_ticks, 4);
    }

    #[test]
    fn dtx_span_saves_payload_bytes_without_claiming_vad_quality() {
        let mut sender = sender();
        let dtx = sender.send_silence(10).unwrap();
        assert_eq!(dtx.len(), HEADER_BYTES);
        assert_eq!(sender.next_tick(), 10);
        assert!(dtx.len() < 10 * (HEADER_BYTES + 80));
        let mut r = receiver();
        admit(&mut r, &dtx, 100).unwrap();
        assert_eq!(r.queued_ticks(), 10);
        assert_eq!(r.poll(160), Ok(Playout::Silence { tick: 0 }));
    }

    #[test]
    fn bounded_future_window_and_maximum_pending_memory() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        assert_eq!(admit(&mut r, &audio(MAX_AHEAD_TICKS, 1), 101),
            Err(VoiceError::TooFarAhead));
        for tick in 1..MAX_PENDING_TICKS as u64 {
            admit(&mut r, &audio(tick, 2), 101 + tick).unwrap();
        }
        assert_eq!(r.queued_ticks(), MAX_PENDING_TICKS);
        assert_eq!(admit(&mut r, &audio(MAX_PENDING_TICKS as u64, 9), 140),
            Err(VoiceError::BufferFull));
        assert_eq!(r.queued_ticks(), MAX_PENDING_TICKS);
        assert_eq!(r.metrics().peak_buffered_ticks, MAX_PENDING_TICKS);
    }

    #[test]
    fn delayed_host_skips_stale_audio_instead_of_backlog_draining() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        admit(&mut r, &audio(4, 5), 105).unwrap();
        assert_eq!(r.poll(240), Ok(Playout::Opus { tick: 4, bytes: vec![5; 5] }));
        assert_eq!(r.metrics().skipped_stale_ticks, 4);
        assert_eq!(r.queued_ticks(), 0);
    }

    #[test]
    fn blocked_or_revoked_policy_clears_buffer_and_cannot_reactivate() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        assert_eq!(r.receive(&audio(1, 2), peer(2, 42), PeerPolicy::Revoked, 101),
            Err(VoiceError::Revoked));
        assert!(r.is_closed());
        assert_eq!(r.queued_ticks(), 0);
        assert_eq!(admit(&mut r, &audio(1, 2), 102), Err(VoiceError::Closed));
        assert_eq!(r.poll(160), Err(VoiceError::Closed));
        let mut s = sender();
        s.close();
        assert_eq!(s.send_silence(1), Err(VoiceError::Closed));
        let mut denied = receiver();
        assert_eq!(denied.receive(&audio(0, 1), peer(2, 42),
            PeerPolicy::Denied, 100), Err(VoiceError::Denied));
        assert!(denied.is_closed());
    }

    #[test]
    fn monotonic_host_clock_required_without_mutating_valid_queue() {
        let mut r = receiver();
        admit(&mut r, &audio(0, 1), 100).unwrap();
        assert_eq!(admit(&mut r, &audio(1, 2), 99), Err(VoiceError::InvalidClock));
        assert_eq!(r.poll(160), Ok(Playout::Opus { tick: 0, bytes: vec![1; 5] }));
        assert_eq!(r.poll(159), Err(VoiceError::InvalidClock));
        assert_eq!(r.queued_ticks(), 0);
    }
}
