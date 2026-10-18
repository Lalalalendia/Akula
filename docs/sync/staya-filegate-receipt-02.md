# STAYA FILEGATE-RECEIPT-02: real Akula Core File Gate → local disk → signed LesHa receipt

**Status:** independent research; not shipping code. Parent experiment [PR #331](https://github.com/Lalalalendia/Akula/pull/331) passed 8/8 with a synthetic, in-memory disk. This stacked experiment calls a **real** public `aquilum_core::files::gate::create` in a temporary test workspace, then performs additional file/parent-directory `sync_all` and reads exact disk bytes before persisting a signed receipt in isolated appdata. No second writer or workspace mutation API is introduced.

## Underlying existing Akula code

- [Real File Gate](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/files/gate/write.rs): `create()` obtains Core's File Gate lock and invokes its existing primary `create_file_impl`; history logging is handled by Core. The API used in this experiment does **not** imply remote Sync Core write authorization.
- [Actual file creation](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/files/entries.rs): `create_new` plus bytes `write_all` and `sync_all`, no overwriting existing note names. The test then syncs the containing directory and re-reads file content.
- [Existing production-adjacent recovery experiments](https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/core/src/sync/local_create_tests.rs) already test real process exit after staging (before publish) and after no-replace publish. They deliberately **never** mark the sync baseline confirmed. This new experiment does not claim to replace those controls.
- [LesHa verified-receipt evaluator](https://github.com/Lalalalendia/med/blob/5046ff53207ce2fc89fb83277d9ceec1fee30935/crates/lesha-replication/src/lib.rs) exact SHA is used by Cargo path and GitHub CI. Its `new_prevalidated` assumes a trusted caller; this research explicitly creates such an object **only after** verifying a signed receipt and the **local** filesystem.

## Simulated crash barriers

The child executable is a separate process, not an in-process panic. In all cases, the parent starts with a fresh disposable temp workspace:

1. `before_gate`: immediate process `exit(80)`; no file or receipt is created or credited.
2. `after_gate`: real Core File Gate creates, syncs file, parent syncs, and readback verifies bytes; then process exits with `exit(81)` *before* a receipt is written. A new verifier sees the file but must return `MissingReceipt`, and LesHa still counts **zero** verified copies.
3. `after_receipt`: after the same real gate, the receipt is signed Ed25519 and persisted as a new file in appdata with its file and directory synced; child exits with `exit(82)` before a response is sent. A new process reopens the receipt and disk bytes and checks digest/signature/node/subject. Rechecking the same receipt cannot increase the number of replicas.

Other tests: corruption of payload, forged receipt signature, replay/duplicate receipt, wrong node identity, same-host failure domain (no false independent-host credit), no overwrite through `gate::create`, and actual LesHa A/B/C → lost A → planned D → physically created test file on D → signed verified receipt → counted D. Bounded repair jobs/bytes leave pending work pending.

**Important:** D's test bytes are the same known deterministic fixture, written through D's File Gate. This is not a Roha/noq C→D network transfer, nor a real three-machine placement proof.

## Trust and durability limitations

- Each `node-N` is a directory on a **single GitHub-hosted machine**. Test labels Host 1/2/3 represent **declared** domains, not physical independence. A test explicitly proves three replicas with the same host label cannot satisfy a three-host policy.
- Process exit with `std::process::exit` proves restart behavior, NOT sudden power loss. GitHub runners cannot guarantee storage device power-failure semantics, so no power-loss claims should be made.
- Receipts are signed with **deterministic test seeds**; the signer has access to the local file. This is not remote-disk attestation, real Akula grant enrollment, secure identity custody, host OS service authorization or adversarial path safety. All filesystem paths are supplied by the local test fixture, never a gossip message.
- `aquilum_core::files::gate::create` creates a new local note under a legacy `Core::open`; it is not a production remote Sync Core commit or a complete installed-host custody epoch. Existing `VaultPort` and `SqliteSyncJournal` retain their separate legal authority. No sync baseline is marked by this crate.
- LesHa's `ChunkId` / `subject_hash` use **SHA-256**, whereas Akula's primary file hash is **BLAKE3**. Both are checked independently and domain separated. Equal digest sizes do not imply protocol type equivalence.
- The receipt file is test appdata, not a new canonical Akula record. It needs a genuine durable, signed, revocable, host-scoped receipt schema and replay/fencing semantics before production adoption. A local file/receipt can still be compromised by a malicious same-user process; pinned-handle OS trust is separate work.

## Separate gates after CI

**R1:** wire file receipts through a trusted registered-host File Gate lifetime and real signed Akula Vault/Provider identity, with fail-closed version generation/revocation. **R2:** inject crashes *during* staging and after filesystem publication while preserving Akula's `NeedsReview` state without automatically publishing an ACK. **R3:** transfer actual bytes C→D over pinned Roha/noq and verify them on independent machines with disk quotas and no false `Replicated`. **R4:** power-loss/reboot and disk corruption on real independent hosts.

The experiment lives only in a draft branch and is not permission to merge, deploy, or bypass the current registered-host HOLD policy.
