//! AK-CALL-02, non-shipping, actual libopus + bounded Akula voice lab.
//! No device capture, speaker, recording, Iroh transport, or OS permission.
//! Code executes only on a non-realtime worker: it allocates and may block.
//! The host must separately authenticate QUIC peer and accepted call, check
//! current contact trust, stop OS capture on mute/revoke, and close on loss.
//! 48 kHz mono 20 ms is this laboratory's *only* negotiated profile.

use aquilum_core::p2p_offer_lab::PeerBinding;
use aquilum_core::voice_jitter_lab::{
    MAX_OPUS_BYTES, Payload, PeerPolicy, Playout, SAMPLE_RATE_HZ, SAMPLES_PER_FRAME, VoiceError,
    VoiceMetrics, VoicePacket, VoiceReceiver, VoiceSender,
};
use opus::{Application, Channels, Decoder, Encoder};

#[derive(Debug)]
pub enum CodecLabError {
    Voice(VoiceError),
    Opus(opus::Error),
    InvalidPcm,
    InvalidOpusFrame,
}

impl From<VoiceError> for CodecLabError {
    fn from(value: VoiceError) -> Self {
        Self::Voice(value)
    }
}

impl From<opus::Error> for CodecLabError {
    fn from(value: opus::Error) -> Self {
        Self::Opus(value)
    }
}

fn validate_opus_frame(data: &[u8]) -> Result<(), CodecLabError> {
    // The Akula wire advertises one 20 ms mono tick. An arbitrary opaque Opus
    // packet may describe 40/60/120 ms or stereo; reject before buffering and
    // decoding to prevent overflowing the host's one-tick playout contract.
    let samples = opus::packet::get_nb_samples(data, SAMPLE_RATE_HZ)?;
    let channels = opus::packet::get_nb_channels(data)?;
    if samples != SAMPLES_PER_FRAME || channels != Channels::Mono {
        return Err(CodecLabError::InvalidOpusFrame);
    }
    Ok(())
}

pub struct OpusVoiceSender {
    voice: VoiceSender,
    encoder: Encoder,
    scratch: [u8; MAX_OPUS_BYTES],
}

impl OpusVoiceSender {
    pub fn new(call_id: [u8; 16], start_tick: u64) -> Result<Self, CodecLabError> {
        Ok(Self {
            voice: VoiceSender::new(call_id, start_tick)?,
            encoder: Encoder::new(SAMPLE_RATE_HZ, Channels::Mono, Application::Voip)?,
            scratch: [0; MAX_OPUS_BYTES],
        })
    }

    /// Local mute starts TRUE; unmute is not an OS permission grant. The host
    /// must own consent, device lifecycle and independent capture shutdown.
    pub fn set_muted(&mut self, muted: bool) -> Result<(), CodecLabError> {
        self.voice.set_muted(muted)?;
        Ok(())
    }

    pub fn is_muted(&self) -> bool {
        self.voice.is_muted()
    }

    /// Never run inside a sound-device callback. A muted sender never calls
    /// the encoder and never publishes the supplied audio samples.
    pub fn encode_frame(&mut self, mono_pcm: &[i16]) -> Result<Vec<u8>, CodecLabError> {
        if self.voice.is_muted() {
            return Ok(self.voice.send_silence(1)?);
        }
        if mono_pcm.len() != SAMPLES_PER_FRAME {
            return Err(CodecLabError::InvalidPcm);
        }
        let encoded_len = self.encoder.encode(mono_pcm, &mut self.scratch)?;
        if encoded_len == 0 || encoded_len > MAX_OPUS_BYTES {
            return Err(CodecLabError::InvalidOpusFrame);
        }
        let payload = &self.scratch[..encoded_len];
        validate_opus_frame(payload)?;
        Ok(self.voice.send_opus(payload)?)
    }

    /// Only after an independent, measured VAD decision. No speech detector
    /// exists here; do not enable automatic DTX based on synthetic samples.
    pub fn send_explicit_silence(&mut self, ticks: u8) -> Result<Vec<u8>, CodecLabError> {
        Ok(self.voice.send_silence(ticks)?)
    }

    pub fn close(&mut self) {
        self.voice.close();
        self.scratch.fill(0);
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum DecodedAudio {
    Waiting,
    Pcm {
        tick: u64,
        samples: [i16; SAMPLES_PER_FRAME],
        concealed: bool,
    },
    Silence {
        tick: u64,
    },
}

pub struct OpusVoiceReceiver {
    voice: VoiceReceiver,
    decoder: Decoder,
    call_id: [u8; 16],
    authenticated_remote: PeerBinding,
}

impl OpusVoiceReceiver {
    pub fn new(
        call_id: [u8; 16],
        authenticated_remote: PeerBinding,
    ) -> Result<Self, CodecLabError> {
        Ok(Self {
            voice: VoiceReceiver::new(call_id, authenticated_remote)?,
            decoder: Decoder::new(SAMPLE_RATE_HZ, Channels::Mono)?,
            call_id,
            authenticated_remote,
        })
    }

    pub fn metrics(&self) -> VoiceMetrics {
        self.voice.metrics()
    }

    pub fn queued_ticks(&self) -> usize {
        self.voice.queued_ticks()
    }

    /// PeerBinding MUST come from the verified live QUIC connection and
    /// currently pinned OS trust, never from bytes inside the media frame.
    pub fn receive(
        &mut self,
        wire: &[u8],
        independently_authenticated_sender: PeerBinding,
        policy: PeerPolicy,
        arrival_ms: u64,
    ) -> Result<(), CodecLabError> {
        if self.voice.is_closed() {
            return Err(CodecLabError::Voice(VoiceError::Closed));
        }
        if policy != PeerPolicy::Allowed {
            // Invoke the core state machine to close/erase the queue even if
            // the incoming bytes are malformed or the peer is now revoked.
            self.voice.close();
            return Err(CodecLabError::Voice(if policy == PeerPolicy::Revoked {
                VoiceError::Revoked
            } else {
                VoiceError::Denied
            }));
        }
        if independently_authenticated_sender != self.authenticated_remote {
            return Err(CodecLabError::Voice(VoiceError::WrongPeer));
        }
        let packet = VoicePacket::decode(wire)?;
        if packet.call_id() != self.call_id {
            return Err(CodecLabError::Voice(VoiceError::WrongCall));
        }
        if let Payload::Opus(data) = packet.payload() {
            validate_opus_frame(data)?;
        }
        self.voice
            .receive(wire, independently_authenticated_sender, policy, arrival_ms)?;
        Ok(())
    }

    /// Returns a bounded mono PCM buffer (or a silence marker) for a host-owned
    /// playback queue. It does not open, access or write an OS sound device.
    pub fn poll(&mut self, host_now_ms: u64) -> Result<DecodedAudio, CodecLabError> {
        match self.voice.poll(host_now_ms)? {
            Playout::Waiting => Ok(DecodedAudio::Waiting),
            Playout::Silence { tick } => Ok(DecodedAudio::Silence { tick }),
            Playout::Opus { tick, bytes } => {
                let mut pcm = [0i16; SAMPLES_PER_FRAME];
                let count = self.decoder.decode(&bytes, &mut pcm, false)?;
                if count != SAMPLES_PER_FRAME {
                    return Err(CodecLabError::InvalidOpusFrame);
                }
                Ok(DecodedAudio::Pcm {
                    tick,
                    samples: pcm,
                    concealed: false,
                })
            }
            Playout::Conceal { tick } => {
                let mut pcm = [0i16; SAMPLES_PER_FRAME];
                let count = self.decoder.decode(&[], &mut pcm, false)?;
                if count != SAMPLES_PER_FRAME {
                    return Err(CodecLabError::InvalidOpusFrame);
                }
                Ok(DecodedAudio::Pcm {
                    tick,
                    samples: pcm,
                    concealed: true,
                })
            }
        }
    }

    /// Host calls this on end, revoke, loss of consent, or QUIC disconnect.
    /// No incoming packet may reopen this instance.
    pub fn close(&mut self) {
        self.voice.close();
    }
}
