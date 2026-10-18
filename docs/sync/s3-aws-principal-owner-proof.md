# SYNC-05d2k — AWS STS principal and S3 bucket-owner proof

**Scope:** opt-in, AWS-only *read-side* trust evidence for Akula S3 profiles. It does not turn on user-facing sync, V2 root publication, mutation/baseline authority, cloud deletion, GC, key storage or account key rotation. Custom endpoints (including MinIO, LocalStack and self-hosted S3) remain unsupported by this proof.

## Existing gap

The V1 `S3RemoteAnchorScope::new` and V2 `S3V2TrustedAnchorJournal` accept a trusted-by-caller *string* as the account binding. The resulting BLAKE3 scope fingerprint checks consistency after restart, but cannot establish who owns the AWS credentials or the S3 bucket. Passing a string copied from untrusted S3 data cannot be called authentication.

## New AWS-only probe

`AwsS3ProbeClient::verify_aws_account_scope(profile_id, independently_approved_account_id)`:

1. Requires a pre-approved exact **12-digit AWS account ID** (it is not inferred from today's cloud root or a user-controlled endpoint), and a normal regional AWS S3 configuration with `endpoint=None`. Rejects any explicit S3-compatible endpoint **before STS/S3 network calls**.
2. Uses the **same resolved `aws-config::SdkConfig` credential provider** for a separate official `aws-sdk-sts` client. Explicitly clears service endpoint overrides, including a custom `AWS_ENDPOINT_URL_STS`. Requires successful signed `GetCallerIdentity`, exact account ID, well-formed IAM/STS ARN matching that account and nonempty caller ID. No caller ARN, access key or secret is saved/logged.
3. Uses the existing S3 SDK client to `HeadBucket` with `x-amz-expected-bucket-owner` set to the independently approved account ID. A foreign-owned bucket, missing permission, unavailable service or other ambiguous result is HOLD. For now cross-account bucket sharing is intentionally rejected.
4. Only after **both** probes succeed, creates a typed `S3AwsVerifiedAccountScope`. Its internal `S3RemoteAnchorScope` fingerprints the exact selected profile, region, endpoint, bucket, namespace/vault and the `aws-account:<ID>` claim.

This avoids equating *access to an S3 object* or a `HEAD` ETag with actual AWS principal identity. The typed result carries **observation evidence only**, not remote-writer permission.

## Security limitations and release gates

- Existing legacy `S3RemoteAnchorScope::new` and `enroll_verified_genesis` are unchanged for MinIO tests / separate trust workflows; they do not magically gain AWS principal authentication. **Before exposing production AWS sync**, wire the typed proof into host admission and V2 root re-verification; prohibit fallback to a raw host-provided string.
- The STS call and bucket HEAD are separate events. Provider-chain credential rotation or hostile external writers can race them; this is not a remote signed manifest, writer identity, tamper-proof monotonic hardware counter or durable root consent receipt. A privileged operation must obtain fresh identity evidence with a stable caller-owned credential/authority policy.
- The AWS account ID must come from an independently approved channel (e.g. trusted local enrollment). Auto-accepting whatever account STS returns merely authenticates an arbitrary account chosen by whoever supplied credentials.
- This is deliberately **AWS-only**. MinIO and other S3-compatible deployments need an independently authenticated provider identity mechanism. Never emulate AWS STS by trusting a fake response from the custom S3 endpoint.
- `HeadBucket` may require additional bucket permissions; lack of permission is refusal, not silent bypass. An IAM role within the account is not automatically entitled to publish or overwrite.
- This change does not integrate with automatic S3 read/write, File Gate, confirmed document baselines, remote CAS, revocation, encryption, anti-rollback against malicious app-private database replacement, or two-device production release.

## Verification

- Offline unit tests reject malformed expected accounts, custom endpoints, foreign-account and inconsistent ARN/user ID combinations.
- Cargo uses the **existing** `aws-sdk-sts = 1.119.0` already in the workspace `Cargo.lock`. The SDK is optional under `sync-s3`; ordinary builds remain independent of STS.
- Explicit S3/MinIO CI invokes `s3_transport::principal::tests`. The existing MinIO tests use custom endpoints and do **not** contact real AWS STS.
- **Still needed:** an isolated real AWS account with approved bucket owner to exercise successful and wrong-owner `GetCallerIdentity + HeadBucket` end-to-end; until then do not claim successful live AWS identity acceptance. Exact PR head compilation/tests and OS gates must pass before merge.
