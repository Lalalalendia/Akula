# MUSIC-03c: thread transport and rendered-frame clock candidate

A real standard-library owner thread processes a bounded request queue (16).
No network, filesystem or real audio runs in this thread. The reference
Runtime and the queue share a mutex; each mock step is short and serialized.
Stop removes pending requests and runs first at its next turn. Shutdown
closes admission, cancels queued work, stops the mock and joins. This is not
a lock-free audio callback or a latency guarantee. A real network/device
backend must perform I/O outside the mutex and use a separate callback-safe
handoff. The caller handles explicit backpressure; discarded requests after
Stop are intentionally canceled. A new owner requires old worker shutdown.

RenderedClock is a separate accounting contract for cumulative output
frames, fixed sample rate per session, owner/session/revision fencing.
Stereo frames are not twice as much listening as mono. Position/seek,
buffered samples and elapsed wall time are not evidence of output. Before
changing the authorization the backend must settle the old counter;
a late report from an old revision is ignored. Rate changes, UTC mapping,
actual callback/device drain confirmation and SQLite integration remain
unimplemented. The clock uses synthetic reports in tests, not sound.

Six frame tests and three threaded transport tests are staged. No new
dependencies or workflows. Require actual Rust CI before acceptance.
