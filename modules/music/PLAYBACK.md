# MUSIC-03a: deterministic playback controller candidate

This dependency-free branch is based on MUSIC-01, not the SQLite branch.
No live provider, audio device, network, UI, resource scheduler, or release
activation. Queue capped at 1,000 entries and upcoming UI snapshot at 30.
Tracks validate duration and identifier lengths. No random shuffle yet.

Single owner delivers effects in revision order; a host must allocate a
unique owner ID per controller/account lifetime and stop the previous
backend when replacing it. Load creates a new session, including repeats;
every backend-affecting transition gets a fresh nonwrapping revision.
A backend must fence stale commands as well as late responses and hold
output during load/seek until it gets the current SetPaused command.
The pure reducer cannot itself stop a real misbehaving audio worker.

Progress is media position, not rendered time. Playing is requested state,
not evidence of sound. The future backend must measure rendered frames and
route statistics using owner/session, not request revision. Account fences,
bounded command transport, output acknowledgement and backpressure remain
follow-up integration contracts. At the final track with repeat off, stop
clears the queue; explicit next ignores repeat-one. Previous at index zero
restarts the first track unless repeat-all wraps. These are initial policies.

Run `cargo test --manifest-path modules/music/Cargo.toml --offline`.
The existing MUSIC-01 required CI route covers this crate when admitted.
No new PR/workflow; wait for MUSIC-01 and reconcile with MUSIC-02 before
integrating. Tests are staged; CI or a real Rust toolchain is required.
