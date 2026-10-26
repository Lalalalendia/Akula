# Portable S3 invitation v1 — Android/Core dependency

The new `aquilum-core` feature `sync-s3-invite` exposes the **existing**
`S3PairInvite`, `S3PairProfile` and shared `S3ConnectionProfile` without
AWS S3/STS SDKs, Tokio, `sync-engine` or host custody. Full `sync-s3` **implies**
`sync-s3-invite`: both desktop transport and offline mobile reader use the
same validator, envelope serializer, KDF, authenticated decryption and errors.

Wire V1 is unchanged: `AKS3V1\0\0`, PBKDF2-HMAC-SHA256 (600,000 iterations),
16-byte salt, 12-byte AES-256-GCM nonce, full header AAD and 16 KiB upper
bound. No separate Android crypto implementation or Argon2id substitution.

**Security boundary:** Decrypting an invitation yields only a candidate
configuration and a purported genesis fingerprint. Never persist plaintext
credentials into JSON or vault notes. Trusted owner fingerprint, S3 genesis
ancestry, provider account identity, Android Keystore residency, and mobile
File Gate admission remain separate gates. No network permission, S3 client,
sync worker or writer is activated by `sync-s3-invite` alone.

Acceptance:
- `cargo test -p aquilum-core --no-default-features --features sync-s3-invite`
  must pass the existing encrypted invite/authentication/tamper/version/limit
  and profile JSON tests with no AWS SDK feature.
- `sync-s3` still resolves the same transport error and connection profile,
  and existing desktop S3 tests compile.
- Android ARM64 and x86_64 must compile with only `sync-s3-invite`, while
  local File Gate and no-INTERNET app behavior remain unchanged.
