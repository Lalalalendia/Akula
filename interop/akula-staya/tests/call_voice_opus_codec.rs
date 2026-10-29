#![cfg(feature = "call-voice-codec-lab")]
// Actual bundled libopus on native Linux/Windows; synthetic PCM only.
// No OS microphone, speaker, network, real mouth-to-ear timing, or recording.

use akula_staya_integration::call_voice_codec_lab::{
    CodecLabError, DecodedAudio, OpusVoiceReceiver, OpusVoiceSender,
};
use aquilum_core::p2p_offer_lab::PeerBinding;
use aquilum_core::sync::DeviceId;
use aquilum_core::voice_jitter_lab::{
    HEADER_BYTES, PeerPolicy, SAMPLE_RATE_HZ, SAMPLES_PER_FRAME, VoiceError, VoicePacket,
};
use opus::{Application, Channels, Encoder};
use uuid::Uuid;

const CALL: [u8; 16] = [37; 16];

fn peer() -> PeerBinding {
    PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_u128(42)),
        endpoint_id: [91; 32],
    }
}

fn pcm_frame(frame: usize) -> [i16; SAMPLES_PER_FRAME] {
    let mut samples = [0i16; SAMPLES_PER_FRAME];
    for (i, sample) in samples.iter_mut().enumerate() {
        let time = (frame * SAMPLES_PER_FRAME + i) as f64 / f64::from(SAMPLE_RATE_HZ);
        let tone = 0.65 * (2.0 * std::f64::consts::PI * 310.0 * time).sin()
            + 0.2 * (2.0 * std::f64::consts::PI * 770.0 * time).sin();
        *sample = (tone * 13_000.0) as i16;
    }
    samples
}

fn admit(receiver: &mut OpusVoiceReceiver, wire: &[u8], time: u64) {
    receiver
        .receive(wire, peer(), PeerPolicy::Allowed, time)
        .unwrap();
}

#[test]
fn real_opus_pcm_roundtrip_through_bounded_akula_voice_ticks() {
    let mut sender = OpusVoiceSender::new(CALL, 0).unwrap();
    let mut receiver = OpusVoiceReceiver::new(CALL, peer()).unwrap();
    assert!(sender.is_muted());
    sender.set_muted(false).unwrap();
    let mut signal_power: u64 = 0;
    let mut transmitted_bytes = 0usize;
    for tick in 0..16usize {
        let wire = sender.encode_frame(&pcm_frame(tick)).unwrap();
        transmitted_bytes += wire.len();
        assert!(wire.len() > HEADER_BYTES);
        admit(&mut receiver, &wire, 100 + tick as u64 * 20);
        assert_eq!(receiver.queued_ticks(), 1);
        match receiver.poll(160 + tick as u64 * 20).unwrap() {
            DecodedAudio::Pcm {
                tick: received,
                samples,
                concealed: false,
            } => {
                assert_eq!(received, tick as u64);
                signal_power += samples
                    .iter()
                    .map(|s| u64::from(s.unsigned_abs()))
                    .sum::<u64>();
            }
            other => panic!("expected decoded Opus PCM: {other:?}"),
        }
    }
    assert!(signal_power > 100_000);
    assert_eq!(receiver.metrics().played_opus_ticks, 16);
    assert_eq!(receiver.metrics().conceal_ticks, 0);
    eprintln!("AKULA_CALL_VOICE_OPUS_REAL_ROUNDTRIP frames=16 actual_opus_wire_bytes={transmitted_bytes}");
}

#[test]
fn real_libopus_plc_after_one_dropped_media_frame() {
    let mut sender = OpusVoiceSender::new(CALL, 0).unwrap();
    sender.set_muted(false).unwrap();
    let mut receiver = OpusVoiceReceiver::new(CALL, peer()).unwrap();
    let mut concealed = 0;
    for tick in 0..9usize {
        let wire = sender.encode_frame(&pcm_frame(tick)).unwrap();
        if tick != 4 {
            admit(&mut receiver, &wire, 100 + tick as u64 * 20);
        }
        match receiver.poll(160 + tick as u64 * 20).unwrap() {
            DecodedAudio::Pcm {
                tick: received,
                samples,
                concealed: flag,
            } => {
                assert_eq!(received, tick as u64);
                assert_eq!(samples.len(), SAMPLES_PER_FRAME);
                if flag {
                    concealed += 1;
                }
            }
            other => panic!("expected PCM or actual PLC: {other:?}"),
        }
    }
    assert_eq!(concealed, 1);
    assert_eq!(receiver.metrics().conceal_ticks, 1);
    assert_eq!(receiver.metrics().played_opus_ticks, 8);
}

#[test]
fn actual_opus_rejects_wrong_duration_or_stereo_before_jitter_queue() {
    let mut mono = Encoder::new(SAMPLE_RATE_HZ, Channels::Mono, Application::Voip).unwrap();
    let mut stereo = Encoder::new(SAMPLE_RATE_HZ, Channels::Stereo, Application::Voip).unwrap();
    let mut output = [0u8; 1275];
    let long_pcm = vec![55i16; SAMPLES_PER_FRAME * 2];
    let count = mono.encode(&long_pcm, &mut output).unwrap();
    let long = VoicePacket::opus(CALL, 0, output[..count].to_vec())
        .unwrap()
        .encode()
        .unwrap();
    let mut receiver = OpusVoiceReceiver::new(CALL, peer()).unwrap();
    assert!(matches!(
        receiver.receive(&long, peer(), PeerPolicy::Allowed, 100),
        Err(CodecLabError::InvalidOpusFrame)
    ));
    assert_eq!(receiver.queued_ticks(), 0);

    let stereo_pcm = vec![24i16; SAMPLES_PER_FRAME * 2];
    let count = stereo.encode(&stereo_pcm, &mut output).unwrap();
    let stereo_wire = VoicePacket::opus(CALL, 0, output[..count].to_vec())
        .unwrap()
        .encode()
        .unwrap();
    assert!(matches!(
        receiver.receive(&stereo_wire, peer(), PeerPolicy::Allowed, 101),
        Err(CodecLabError::InvalidOpusFrame)
    ));
    assert_eq!(receiver.queued_ticks(), 0);
    let mut sender = OpusVoiceSender::new(CALL, 0).unwrap();
    sender.set_muted(false).unwrap();
    assert!(matches!(
        sender.encode_frame(&long_pcm),
        Err(CodecLabError::InvalidPcm)
    ));
}

#[test]
fn muted_sender_sends_only_explicit_silence_and_revoke_erases_buffer() {
    let mut sender = OpusVoiceSender::new(CALL, 0).unwrap();
    let mut receiver = OpusVoiceReceiver::new(CALL, peer()).unwrap();
    let muted = sender.encode_frame(&pcm_frame(0)).unwrap();
    assert_eq!(muted.len(), HEADER_BYTES);
    let dtx = sender.send_explicit_silence(5).unwrap();
    assert_eq!(dtx.len(), HEADER_BYTES);
    admit(&mut receiver, &muted, 100);
    admit(&mut receiver, &dtx, 101);
    for tick in 0..6u64 {
        assert_eq!(
            receiver.poll(160 + tick * 20).unwrap(),
            DecodedAudio::Silence { tick }
        );
    }
    sender.set_muted(false).unwrap();
    let active = sender.encode_frame(&pcm_frame(6)).unwrap();
    admit(&mut receiver, &active, 230);
    assert_eq!(receiver.queued_ticks(), 1);
    assert!(matches!(
        receiver.receive(&active, peer(), PeerPolicy::Revoked, 231),
        Err(CodecLabError::Voice(VoiceError::Revoked))
    ));
    assert_eq!(receiver.queued_ticks(), 0);
    assert!(matches!(
        receiver.receive(&active, peer(), PeerPolicy::Allowed, 232),
        Err(CodecLabError::Voice(VoiceError::Closed))
    ));
    sender.close();
    assert!(matches!(
        sender.encode_frame(&pcm_frame(7)),
        Err(CodecLabError::Voice(VoiceError::Closed))
    ));
}

#[test]
fn deterministic_zero_one_three_five_percent_loss_with_real_plc() {
    for target_loss in [0usize, 1, 3, 5] {
        let mut sender = OpusVoiceSender::new(CALL, 0).unwrap();
        let mut receiver = OpusVoiceReceiver::new(CALL, peer()).unwrap();
        sender.set_muted(false).unwrap();
        let mut actual_lost = 0u64;
        for tick in 0..100usize {
            let wire = sender.encode_frame(&pcm_frame(tick)).unwrap();
            // Exactly target_loss losses per 100 frames, never dropping
            // the first packet: jitter startup remains deterministic.
            let drop_packet = (tick + 1) * target_loss / 100 > tick * target_loss / 100;
            if drop_packet {
                actual_lost += 1;
            } else {
                admit(&mut receiver, &wire, 100 + tick as u64 * 20);
            }
            match receiver.poll(160 + tick as u64 * 20).unwrap() {
                DecodedAudio::Pcm {
                    tick: received,
                    samples,
                    concealed,
                } => {
                    assert_eq!(received, tick as u64);
                    assert_eq!(samples.len(), SAMPLES_PER_FRAME);
                    assert_eq!(concealed, drop_packet);
                }
                other => panic!("loss profile yielded unexpected event: {other:?}"),
            }
        }
        assert_eq!(actual_lost, target_loss as u64);
        assert_eq!(receiver.metrics().conceal_ticks, actual_lost);
        assert_eq!(receiver.metrics().played_opus_ticks, 100 - actual_lost);
        eprintln!(
            "AKULA_CALL_VOICE_OPUS_SYNTHETIC_LOSS percent={target_loss} \
             plc_ticks={actual_lost} frames=100"
        );
    }
}
