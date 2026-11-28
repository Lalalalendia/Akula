use akula_music::TrackKey;
use akula_music::playback::{Event, Player, Ticket, Track};
use akula_music::playback::pcm::{Format, PcmBuffer, PcmError, Pull, MAX_BYTES};
use std::collections::VecDeque;
use std::num::NonZeroU64;

fn load() -> (Player, Ticket) {
    let mut p = Player::new(NonZeroU64::new(1).unwrap());
    let t = Track::new(TrackKey::new("mock", "a").unwrap(), 180_000).unwrap();
    let ticket = p.replace(vec![t], 0).unwrap().ticket();
    (p, ticket)
}

#[test]
fn format_and_capacity_are_bounded() {
    let (_, t) = load();
    assert_eq!(Format::new(0, 2), Err(PcmError::InvalidFormat));
    assert_eq!(Format::new(48_000, 0), Err(PcmError::InvalidFormat));
    assert_eq!(Format::new(48_000, 9), Err(PcmError::InvalidFormat));
    let f = Format::new(44_100, 2).unwrap();
    let b = PcmBuffer::new(f, t, 250).unwrap();
    assert_eq!(b.capacity_frames(), 11_025);
    assert_eq!(PcmBuffer::new(f, t, 1).unwrap().capacity_frames(), 45);
    assert!(matches!(PcmBuffer::new(f, t, 0), Err(PcmError::InvalidCapacity)));
    assert!(matches!(PcmBuffer::new(f, t, 2_001), Err(PcmError::InvalidCapacity)));
    let large = Format::new(384_000, 8).unwrap();
    assert!(matches!(PcmBuffer::new(large, t, 2_000), Err(PcmError::InvalidCapacity)));
    assert!(b.capacity_frames() * f.channels() * 2 <= MAX_BYTES);
}

#[test]
fn full_buffer_retains_unread_frames_and_reports_partial_push() {
    let (_, t) = load();
    let mut b = PcmBuffer::new(Format::new(8_000, 1).unwrap(), t, 1).unwrap();
    assert_eq!(b.push(t, &[1,2,3,4,5,6]).unwrap(), 6);
    assert_eq!(b.push(t, &[7,8,9,10]).unwrap(), 2);
    assert_eq!(b.push(t, &[99]).unwrap(), 0);
    let mut out = [0; 8];
    assert_eq!(b.pull(t, &mut out).unwrap().audio_frames, 8);
    assert_eq!(out, [1,2,3,4,5,6,7,8]);
}

#[test]
fn wraparound_preserves_interleaved_stereo_order() {
    let (_, t) = load();
    let mut b = PcmBuffer::new(Format::new(8_000, 2).unwrap(), t, 1).unwrap();
    let first: Vec<i16> = (0..12).collect();
    b.push(t, &first).unwrap();
    let mut prefix = [0; 10];
    b.pull(t, &mut prefix).unwrap();
    assert_eq!(prefix, [0,1,2,3,4,5,6,7,8,9]);
    let second: Vec<i16> = (12..26).collect();
    assert_eq!(b.push(t, &second).unwrap(), 7);
    let mut out = [0; 16];
    b.pull(t, &mut out).unwrap();
    assert_eq!(out.as_slice(), (10..26).collect::<Vec<i16>>().as_slice());
}

#[test]
fn underrun_zero_fills_and_counts_only_available_audio() {
    let (_, t) = load();
    let mut b = PcmBuffer::new(Format::new(48_000, 2).unwrap(), t, 250).unwrap();
    b.push(t, &[1,2,3,4]).unwrap();
    let mut out = [99; 8];
    assert_eq!(b.pull(t, &mut out), Ok(Pull { audio_frames: 2, silence_frames: 2 }));
    assert_eq!(out, [1,2,3,4,0,0,0,0]);
    assert_eq!(b.queued_frames(), 0);
}

#[test]
fn unaligned_input_and_output_fail_without_consumption() {
    let (_, t) = load();
    let mut b = PcmBuffer::new(Format::new(48_000, 2).unwrap(), t, 250).unwrap();
    assert_eq!(b.push(t, &[1,2,3]), Err(PcmError::Misaligned));
    b.push(t, &[4,5]).unwrap();
    let mut bad = [9; 3];
    assert_eq!(b.pull(t, &mut bad), Err(PcmError::Misaligned));
    assert_eq!(bad, [0; 3]);
    assert_eq!(b.queued_frames(), 1);
}

#[test]
fn pause_retag_preserves_queue_seek_reset_cancels_stale_audio() {
    let (mut p, t) = load();
    let mut b = PcmBuffer::new(Format::new(48_000, 1).unwrap(), t, 250).unwrap();
    b.push(t, &[1,2,3]).unwrap();
    let run = p.receive(t, Event::Ready).unwrap().unwrap().ticket();
    b.retag(run).unwrap();
    assert_eq!(b.push(t, &[9]), Err(PcmError::StaleTicket));
    let pause = p.set_paused(true).unwrap().unwrap().ticket();
    b.retag(pause).unwrap();
    assert_eq!(b.queued_frames(), 3);
    let seek = p.seek(10_000).unwrap().ticket();
    b.reset(seek).unwrap();
    assert_eq!(b.queued_frames(), 0);
    assert_eq!(b.push(pause, &[9]), Err(PcmError::StaleTicket));
    let mut out = [99; 3];
    assert_eq!(b.pull(pause, &mut out), Err(PcmError::StaleTicket));
    assert_eq!(out, [0; 3]);
    assert_eq!(b.pull(seek, &mut out).unwrap().silence_frames, 3);
    let stop = p.stop().unwrap().ticket();
    b.reset(stop).unwrap();
    assert_eq!(b.push(stop, &[9]), Err(PcmError::InvalidTicket));
    assert_eq!(b.reset(seek), Err(PcmError::StaleTicket));
}

#[test]
fn five_thousand_ring_operations_match_reference_fifo() {
    let (_, t) = load();
    let mut b = PcmBuffer::new(Format::new(8_000, 2).unwrap(), t, 4).unwrap();
    let mut reference = VecDeque::new();
    let mut seed = 113_u64;
    let cap = b.capacity_frames() * 2;
    for _ in 0..5_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let frames = ((seed >> 32) % 70) as usize;
        if seed & (1 << 40) == 0 {
            let input: Vec<i16> = (0..frames * 2).map(|i| (seed as i16).wrapping_add(i as i16)).collect();
            let count = input.len().min(cap - reference.len());
            assert_eq!(b.push(t, &input).unwrap(), count / 2);
            reference.extend(input[..count].iter().copied());
        } else {
            let mut out = vec![99; frames * 2];
            let count = reference.len().min(out.len());
            let expected: Vec<i16> = (0..out.len()).map(|_| reference.pop_front().unwrap_or(0)).collect();
            assert_eq!(b.pull(t, &mut out).unwrap(), Pull { audio_frames: count / 2, silence_frames: frames - count / 2 });
            assert_eq!(out, expected);
        }
        assert_eq!(b.queued_frames() * 2, reference.len());
    }
}
