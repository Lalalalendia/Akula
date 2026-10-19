#!/usr/bin/env bash
# P2P-BLOB-08: disposable, physically bounded ext4 staging experiment.
# Only format a brand-new 64MiB file IMAGE in GitHub RUNNER_TEMP.
# Never run mkfs on a host disk, partition, existing user path or S3 volume.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" || "$GITHUB_ACTIONS" != "true" ]]; then
  echo "P2P_BLOB_EXT4_REFUSED: requires a disposable Linux GitHub runner" >&2
  exit 1
fi
if [[ -z "$RUNNER_TEMP" || "$RUNNER_TEMP" != /* || ! -d "$RUNNER_TEMP" ]]; then
  echo "P2P_BLOB_EXT4_REFUSED: invalid RUNNER_TEMP" >&2
  exit 1
fi

for binary in sudo mkfs.ext4 mountpoint findmnt truncate python3 cargo; do
  command -v "$binary" >/dev/null || {
    echo "P2P_BLOB_EXT4_REFUSED: missing $binary" >&2
    exit 1
  }
done

lab="$(mktemp -d "$RUNNER_TEMP/akula-p2p08-ext4.XXXXXXXX")"
image="$lab/isolated-64MiB.ext4"
volume="$lab/mounted"
mkdir -m 0700 "$volume"
cleanup() {
  if mountpoint -q "$volume"; then
    sudo umount "$volume" || {
      echo "P2P_BLOB_EXT4_CLEANUP_FAILED: mounted volume still busy" >&2
      return 1
    }
  fi
  rm -rf -- "$lab"
}
trap cleanup EXIT

export AKULA_BOUNDED_EXT4_ROOT="$volume"
readonly IMAGE_BYTES=67108864
truncate -s "$IMAGE_BYTES" "$image"
test "$(stat -c %s "$image")" -eq "$IMAGE_BYTES"

# -F formats ONLY the freshly created private regular file; never a block device.
mkfs.ext4 -F -q -m 0 "$image"

mount_image() {
  if mountpoint -q "$volume"; then
    echo "P2P_BLOB_EXT4_REFUSED: target already mounted" >&2
    exit 1
  fi
  sudo mount -t ext4 -o loop,nosuid,nodev,noexec "$image" "$volume"
  if ! mountpoint -q "$volume" || [[ "$(findmnt -n -o FSTYPE --target "$volume")" != "ext4" ]]; then
    echo "P2P_BLOB_EXT4_REFUSED: expected isolated ext4 mount" >&2
    exit 1
  fi
  if [[ "$(stat -c %d "$volume")" == "$(stat -c %d "$lab")" ]]; then
    echo "P2P_BLOB_EXT4_REFUSED: volume silently fell back to host filesystem" >&2
    exit 1
  fi
  sudo chown "$(id -u):$(id -g)" "$volume"
  chmod 0700 "$volume"
  python3 - "$volume" <<'PY'
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
st = os.statvfs(root)
physical_capacity = st.f_blocks * st.f_frsize
assert 0 < physical_capacity <= 64 * 1024 * 1024, physical_capacity
assert root.stat().st_dev != root.parent.stat().st_dev
print("P2P_BLOB_EXT4_MOUNT fstype=ext4 total_bytes="
      f"{physical_capacity} separate_device=true private_mode=0700")
PY
}

mount_image

# This is a real iroh-blobs FsStore in the same kernel-capped ext4 image,
# not only a mocked filesystem writer.
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_fsstore_seed \
  -- --exact --ignored --nocapture

# All actual FsStore data + Bao/outboards + database and synthetic additional
# staging writes share the SAME 64MiB finite Linux filesystem. Its VFS must
# return ENOSPC; a ledger's in-memory reservation cannot fake this result.
python3 - "$volume" <<'PY'
import errno
import hashlib
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
stage = root / "auxiliary-staging"
stage.mkdir(mode=0o700)
sentinel = b"akula-staging-integrity-only\n" * 200
sentinel_file = stage / "sentinel"
with sentinel_file.open("wb") as out:
    out.write(sentinel)
    out.flush()
    os.fsync(out.fileno())
initial_digest = hashlib.sha256(sentinel).hexdigest()
chunk = b"A" * (1024 * 1024)
filler = stage / "over-capacity-workload"
fd = os.open(filler, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
wrote = 0
enospc = False
try:
    for _ in range(128):
        remaining = memoryview(chunk)
        while remaining:
            try:
                count = os.write(fd, remaining)
                if count <= 0:
                    raise AssertionError("zero-progress write")
                wrote += count
                remaining = remaining[count:]
            except OSError as exc:
                if exc.errno != errno.ENOSPC:
                    raise
                enospc = True
                break
        if enospc:
            break
        try:
            os.fsync(fd)
        except OSError as exc:
            if exc.errno != errno.ENOSPC:
                raise
            enospc = True
            break
finally:
    os.close(fd)

assert enospc, f"VFS failed to enforce ENOSPC after {wrote} bytes"
assert 4 * 1024 * 1024 <= wrote < 64 * 1024 * 1024, wrote
assert hashlib.sha256(sentinel_file.read_bytes()).hexdigest() == initial_digest
os.unlink(filler)
dir_fd = os.open(stage, os.O_RDONLY | os.O_DIRECTORY)
try:
    os.fsync(dir_fd)
finally:
    os.close(dir_fd)
print("P2P_BLOB_EXT4_ENOSPC enforced=true actual_write_bytes="
      f"{wrote} existing_sentinel_sha256_ok=true "
      "real_fsstore_shares_same_mount=true")
PY

sync
sudo umount "$volume"
mount_image
python3 - "$volume" <<'PY'
import hashlib
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
sentinel = b"akula-staging-integrity-only\n" * 200
assert hashlib.sha256((root / "auxiliary-staging" / "sentinel").read_bytes()).digest() == hashlib.sha256(sentinel).digest()
print("P2P_BLOB_EXT4_REMOUNT persisted_sentinal_sha256=true")
PY

cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_fsstore_reopen \
  -- --exact --ignored --nocapture

# P2P-BLOB-09a: after the real FsStore has shut down, compare external
# paths with an independently persisted journal. Every deliberately injected
# orphan/corruption/residual-temp/database-absence must HOLD; the test restores
# only its own disposable injected data. The audit never grants File Gate.
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_inventory_audit_seed \
  -- --exact --ignored --nocapture

sync
sudo umount "$volume"
mount_image
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_inventory_audit_after_remount \
  -- --exact --ignored --nocapture

# P2P-BLOB-09b: STRICT read-only redb table audit against the real
# durable journal, including a real small blob inlined in blobs.db. All
# intentionally damaged / foreign DB examples are separate disposable COPIES.
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_redb_read_only_real_external_and_inline \
  -- --exact --ignored --nocapture

sync
sudo umount "$volume"
mount_image
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_redb_read_only_after_remount \
  -- --exact --ignored --nocapture

# P2P-BLOB-10a: join durable journal, exact pinned read-only redb
# and external BLAKE3 under one *quiescent* lab owner. Inline blobs must
# not demand an external .data, but must have complete trusted DB contents.
# Still NOT fd-relative, not an atomic OS snapshot, not a File Gate grant.
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_joint_staging_checkpoint \
  -- --exact --ignored --nocapture

sync
sudo umount "$volume"
mount_image
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_joint_staging_checkpoint_after_remount \
  -- --exact --ignored --nocapture

# P2P-BLOB-10b: actual FsStore with >8MiB complete data and external
# *.obao4 pre-order file. Verify every Bao hash parent/chunk against the
# locked journal root, reject tampered outboard on a SEPARATE copy, and
# recover the same intact proof from the same ext4 image after unmount.
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_owned_bao_crypto_proof \
  -- --exact --ignored --nocapture

sync
sudo umount "$volume"
mount_image
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_owned_bao_after_remount \
  -- --exact --ignored --nocapture

# P2P-BLOB-10c: external FsStore data with inline Bao held in read-only
# redb. Corrupt proof parents / size / omit required row only in disposable DB
# copies and verify the original byte-identical database and journal.
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_inline_bao_crypto_proof \
  -- --exact --ignored --nocapture

sync
sudo umount "$volume"
mount_image
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_inline_bao_after_remount \
  -- --exact --ignored --nocapture

# A missing mount must NOT cause staging to continue on its host parent.
sudo umount "$volume"
cargo test --locked --manifest-path staya/roha-transport/Cargo.toml \
  --test iroh_blobs_compare bounded_ext4_missing_mount_fails_closed \
  -- --exact --ignored --nocapture

echo "P2P_BLOB_EXT4_ACCEPTED finite_volume=true OS_ENOSPC=true FsStore_persisted=true missing_mount_refused=true CI_lab_only=true production_adapter=false"
