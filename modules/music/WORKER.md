# MUSIC-03b: bounded reference worker

No real provider, sound or threads. A deterministic mock executes controller
effects and models one in-flight load/seek. Its admission fence validates
owner/session/revision before accepting any new backend command. A successful
completion cannot release audio by itself. A separate bounded UI-intent
mailbox has capacity 16; the caller gets rejected commands back. Stop is an
explicit cancellation of queued intents, not a silently dropped command.
The single owner dispatches controller effects synchronously, avoiding a
second queue of already-committed dependent effects. Divergent backend errors
quiesce output and poison the worker; account switch starts a new lifetime.

This is a deterministic contract, not the actual concurrent transport. The
host must use globally fresh owner IDs, stop the old real worker and fence
its callbacks; owner switches require strictly increasing IDs. Stop runs before
completion/notification processing or the next explicit step. No real-time scheduling
guarantee. Replace inputs are bounded on admission; caller allocation and
future cross-thread payload budgets are separate. No real rendered frame
counter, durable statistics integration, or provider catalog is claimed.
Future backend must hold output during cancel/seek and confirm actual output
state; Phase::Playing alone is not sound. Test compilation remains pending
MUSIC-01 admission. No new dependency or workflow.
