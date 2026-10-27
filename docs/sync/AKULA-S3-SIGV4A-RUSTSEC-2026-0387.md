# S3 SigV4a security HOLD — RUSTSEC-2026-0387

Date: 2026-10-24. Scope: the optional AWS Rust SDK dependency behind `aquilum-core/sync-s3`.

## Problem

RustSec RUSTSEC-2026-0387 (issued 2026-10-23) identifies secret-dependent timing in `crypto-bigint`, affecting version 0.5.5 locked by this workspace. RustSec lists the first fixed version as 0.7.6.

The AWS SDK's `aws-sigv4 1.6.0` enables `crypto-bigint 0.5.x` and `p256 0.13.x` when `sigv4a` is selected. These major-zero version lines cannot be repaired by a compatible lockfile-only update.

## Temporary capability boundary

Remove only the explicit `sigv4a` Cargo feature from `aws-sdk-s3` and `aws-sdk-sts`. Keep region-scoped AWS Signature Version 4, HTTPS, Tokio, AWS SSO and process credentials. This does not disable the S3/MinIO or Yandex S3 transports.

**Do not claim SigV4a support** in installed builds while this HOLD applies. AWS Multi-Region Access Point and other requests requiring SigV4a or region-set signatures are unsupported; they must fail rather than being described as successfully authenticated or silently reclassified as supported SigV4.

Do not ignore `RUSTSEC-2026-0387`, bypass `cargo audit`, hand-edit Cargo.lock checksums, or claim a FIPS-validated signer.

## Evidence and acceptance

An isolated Actions run, [#40086898002](https://github.com/Lalalalendia/Akula/actions/runs/40086898002), used `cargo update --workspace` on the feature-reduced manifest. Its resulting `Cargo.lock` had Git blob SHA `97aa703c7c6b074724aeacff560b2ee276bd481e`, independently checked against the actual generated file. It removed 17 previously locked package records, including `crypto-bigint`, and preserved the other locked versions where possible.

Required before accepting this hotfix:

1. `cargo audit --no-yanked --file aquilum-app/Cargo.lock` passes on the new lockfile.
2. Normal native and Core builds succeed with `--locked` and S3's existing opt-in settings.
3. Localhost MinIO, Yandex's read-only preview, and S3 safety/failure gates pass without new permission to write.
4. Any regression involving unsupported AWS SigV4a is explicitly documented rather than bypassed.

## Restore condition

Re-enable SigV4a only after the AWS SDK migrates its `aws-sigv4` and P-256 dependency stack to a version patched for this advisory, and the signing, negative, and region-set cases are reverified. Restoration needs a separate reviewed change.
