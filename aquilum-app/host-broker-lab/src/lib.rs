//! RN-4E1b-03b1a: standalone, deliberately non-authorizing IPC boundary.
//!
//! A successful frame parse MUST NOT imply a peer grant, local registration,
//! File Gate permission, durable receipt or APPLIED. Those operations do not
//! exist in this crate. No remote paths or payloads are ever accepted.

pub const REQUEST_BYTES: usize = 32;
pub const RESPONSE_BYTES: usize = 32;
const REQUEST_MAGIC: [u8; 4] = *b"AKHB";
const RESPONSE_MAGIC: [u8; 4] = *b"AKHR";
const VERSION: u16 = 1;
const HELLO_ONLY: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Denial {
    /// No installed authority, registered replica or device grant is present.
    Hold = 1,
    /// Strict framing refused; caller supplied bytes are not authoritative.
    Malformed = 2,
    /// Future/unknown operation has no compatibility fallback.
    Unsupported = 3,
}

/// Returns exactly 32 bytes containing one of THREE NEGATIVE statuses.
/// A future version must not introduce "allow" in this function. A privileged
/// service must authenticate its OS caller and independently check policy
/// *before* ever handing a narrow Core-owned capability to a separate API.
pub fn deny_frame(frame: &[u8]) -> [u8; RESPONSE_BYTES] {
    let mut response = [0u8; RESPONSE_BYTES];
    response[0..4].copy_from_slice(&RESPONSE_MAGIC);
    response[4..6].copy_from_slice(&VERSION.to_le_bytes());
    let status = if frame.len() != REQUEST_BYTES
        || frame.get(0..4) != Some(REQUEST_MAGIC.as_slice())
        || frame.get(24..32) != Some([0u8; 8].as_slice())
        || frame.get(8..24) == Some([0u8; 16].as_slice())
    {
        Denial::Malformed
    } else {
        let version = u16::from_le_bytes([frame[4], frame[5]]);
        let operation = u16::from_le_bytes([frame[6], frame[7]]);
        if version != VERSION || operation != HELLO_ONLY {
            Denial::Unsupported
        } else {
            response[8..24].copy_from_slice(&frame[8..24]);
            Denial::Hold
        }
    };
    response[6..8].copy_from_slice(&(status as u16).to_le_bytes());
    response
}

#[cfg(windows)]
pub mod windows_server;

// No installed Akula caller and no write API; the client only observes
// Windows SCM + the pipe's pinned SYSTEM/service token before sending bytes.
#[cfg(windows)]
pub mod windows_client_attestation;

// SCM lifecycle is only a standalone experimental service, not native Akula.
#[cfg(windows)]
pub mod windows_scm_service;

// Read-only, cancelable SCM pipe transport. No application activates this.
#[cfg(windows)]
mod windows_cancellable_pipe;

// Read-only installer-owned namespace diagnostics. Never grants a peer.
#[cfg(windows)]
pub mod windows_store_custody;

// OS-handle-backed physical directory identity probe. Observation only:
// neither a caller's path nor FILE_ID_INFO can mint host authority.
#[cfg(windows)]
mod windows_root_observation;

// This P2P policy kernel is UNIT-TEST ONLY: no production enrollment or
// channel-binding constructor exists, and no Allow variant can escape.
#[cfg(test)]
mod broker_policy_kernel;

// A Windows OS-process crash/CAS experiment only. This is NEVER compiled
// into the normal Broker executable or the user's installed Akula.
#[cfg(all(test, windows))]
mod windows_revocation_journal;

// Anti-rollback WITNESS CONTRACT ONLY. Still all(test, windows): no
// installer-backed witness, runtime service grant or File Gate consumer.
#[cfg(all(test, windows))]
mod windows_revocation_witness;

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> [u8; REQUEST_BYTES] {
        let mut request = [0u8; REQUEST_BYTES];
        request[0..4].copy_from_slice(&REQUEST_MAGIC);
        request[4..6].copy_from_slice(&VERSION.to_le_bytes());
        request[6..8].copy_from_slice(&HELLO_ONLY.to_le_bytes());
        request[8..24].copy_from_slice(&[9u8; 16]);
        request
    }

    fn outcome(response: [u8; RESPONSE_BYTES]) -> u16 {
        assert_eq!(&response[0..4], &RESPONSE_MAGIC);
        assert_eq!(u16::from_le_bytes([response[4], response[5]]), VERSION);
        assert!(response[24..32].iter().all(|b| *b == 0));
        u16::from_le_bytes([response[6], response[7]])
    }

    #[test]
    fn valid_probe_always_returns_hold_and_never_a_capability() {
        let request = hello();
        let answer = deny_frame(&request);
        assert_eq!(outcome(answer), Denial::Hold as u16);
        assert_eq!(&answer[8..24], &request[8..24]);
        assert!(!answer.iter().any(|b| *b == b'/'));
    }

    #[test]
    fn unknown_future_operations_and_versions_never_default_to_hello() {
        for (offset, value) in [(4, 2), (6, 2), (6, 255)] {
            let mut request = hello();
            request[offset] = value;
            assert_eq!(outcome(deny_frame(&request)), Denial::Unsupported as u16);
        }
    }

    #[test]
    fn reject_truncated_extended_reserved_or_zero_nonce_frames() {
        let full = hello();
        for length in 0..REQUEST_BYTES {
            assert_eq!(
                outcome(deny_frame(&full[..length])),
                Denial::Malformed as u16
            );
        }
        let mut extended = full.to_vec();
        extended.extend_from_slice(&[0u8; 4096]);
        assert_eq!(outcome(deny_frame(&extended)), Denial::Malformed as u16);
        let mut reserved = full;
        reserved[30] = 1;
        assert_eq!(outcome(deny_frame(&reserved)), Denial::Malformed as u16);
        let mut zero_nonce = full;
        zero_nonce[8..24].fill(0);
        assert_eq!(outcome(deny_frame(&zero_nonce)), Denial::Malformed as u16);
    }

    #[test]
    fn fixed_seed_mutations_are_always_one_of_three_denials() {
        let mut value = 0x72ed_u64;
        let mut frame = hello();
        for _ in 0..6000 {
            value = value.wrapping_mul(6364136223846793005).wrapping_add(1);
            let index = (value as usize) % REQUEST_BYTES;
            frame[index] ^= (value >> 16) as u8;
            let status = outcome(deny_frame(&frame));
            assert!((1..=3).contains(&status), "a write permit must never exist");
        }
    }
}
