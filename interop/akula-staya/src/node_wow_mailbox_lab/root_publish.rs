//! B1d0-style first-file publication for the separately approved WOW role.
//! ROOT-ONLY CI LAB: not a release installer, root IPC opcode or mail daemon.
//! An independently signed owner manifest is checked against the fixed
//! RootTrust before any mutation. There is NO auto-enrolment, update or retry.
use super::{approval, policy};
use rustix::fs::{FlockOperation, Mode, OFlags, RenameFlags, flock, openat, renameat_with};
use std::{
    fs::{File, Metadata, Permissions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

const ROOT_PARENT: &str = "/etc/akula-node";
const DIR_NAME: &str = "wow-mail";
const FILES: [(&str, &str); 3] = [
    ("owner-mail-policy.pending", "owner-mail-policy.bin"),
    (
        "owner-mail-epoch-floor.pending",
        "owner-mail-epoch-floor.bin",
    ),
    ("owner-mail-commit.pending", "owner-mail-commit.bin"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishStop {
    None,
    AfterPendingSync,
    AfterPolicyRename,
    AfterFloorRename,
    AfterReceiptRename,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishHold {
    NotCiRoot,
    RootAuthority,
    ExistingOrInterrupted,
    InvalidOwnerSignature,
    InvalidOwnerApproval,
    Io,
    InjectedStop,
}
fn err(_: impl std::fmt::Debug) -> PublishHold {
    PublishHold::Io
}
fn root_dir_ok(m: &Metadata, mode: u32) -> bool {
    m.is_dir() && m.uid() == 0 && m.permissions().mode() & 0o7777 == mode
}
fn open_enrolled_mail() -> Result<(File, u32), PublishHold> {
    if rustix::process::geteuid().as_raw() != 0
        || std::env::var("GITHUB_ACTIONS").ok().as_deref() != Some("true")
        || std::env::var("AKULA_WOW_ROOT_CAS_CI").ok().as_deref() != Some("1")
    {
        return Err(PublishHold::NotCiRoot);
    }
    let parent = File::from(
        rustix::fs::open(
            Path::new(ROOT_PARENT),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?,
    );
    if !root_dir_ok(&parent.metadata().map_err(err)?, 0o755) {
        return Err(PublishHold::RootAuthority);
    }
    let dir = File::from(
        openat(
            &parent,
            DIR_NAME,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?,
    );
    let m = dir.metadata().map_err(err)?;
    if !root_dir_ok(&m, 0o750) || m.gid() == 0 || m.gid() == u32::MAX {
        return Err(PublishHold::RootAuthority);
    }
    Ok((dir, m.gid()))
}
fn require_absent(dir: &File, name: &str) -> Result<(), PublishHold> {
    match openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(PublishHold::ExistingOrInterrupted),
    }
}
fn staged(dir: &File, gid: u32, name: &str, data: &[u8]) -> Result<(), PublishHold> {
    let mut f = File::from(
        openat(
            dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(err)?,
    );
    if f.metadata().map_err(err)?.nlink() != 1 {
        return Err(PublishHold::Io);
    }
    // FD-pinned chmod/chown; never alter a pathname supplied by a client.
    rustix::fs::fchown(
        &f,
        Some(rustix::process::Uid::ROOT),
        // SAFETY: gid came from verified kernel stat metadata and was
        // explicitly checked against the invalid -1 sentinel above.
        Some(unsafe { rustix::process::Gid::from_raw(gid) }),
    )
    .map_err(err)?;
    f.set_permissions(Permissions::from_mode(0o640))
        .map_err(err)?;
    f.write_all(data).map_err(err)?;
    f.sync_all().map_err(err)?;
    let meta = f.metadata().map_err(err)?;
    if meta.uid() != 0
        || meta.gid() != gid
        || meta.nlink() != 1
        || meta.permissions().mode() & 0o7777 != 0o640
        || meta.len() != data.len() as u64
    {
        return Err(PublishHold::Io);
    }
    Ok(())
}
fn publish(dir: &File, source: &str, target: &str) -> Result<(), PublishHold> {
    renameat_with(dir, source, dir, target, RenameFlags::NOREPLACE).map_err(err)?;
    dir.sync_all().map_err(err)?;
    Ok(())
}
fn verify_published(dir: &File, expected: &[(&str, &[u8]); 3]) -> Result<(), PublishHold> {
    for (name, bytes) in expected {
        let mut f = File::from(
            openat(
                dir,
                *name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(err)?,
        );
        let mut actual = Vec::new();
        f.read_to_end(&mut actual).map_err(err)?;
        if actual != *bytes {
            return Err(PublishHold::Io);
        }
    }
    Ok(())
}

/// Exactly one initial epoch-1 policy for a separately pre-provisioned root
/// Node. An owner signature alone is NOT a reusable install approval; the
/// production B1d0 witness/one-shot nonce must be composed before deployment.
/// This function is callable only from explicit root CI test code.
pub fn first_mail_publish_ci(
    signed: &[u8],
    separate_approval: &[u8],
    now_ms: i64,
    stop: PublishStop,
) -> Result<(), PublishHold> {
    let (dir, gid) = open_enrolled_mail()?;
    flock(&dir, FlockOperation::LockExclusive).map_err(err)?;
    for (pending, final_name) in FILES {
        require_absent(&dir, pending)?;
        require_absent(&dir, final_name)?;
    }
    if signed.is_empty() || signed.len() > 4096 {
        return Err(PublishHold::InvalidOwnerSignature);
    }
    let claims = policy::decode_root_verified_claims(signed, now_ms)
        .map_err(|_| PublishHold::InvalidOwnerSignature)?;
    // First provision only; no update/rollback, no unaudited auto-enrolment.
    if claims.policy_epoch != 1 {
        return Err(PublishHold::InvalidOwnerSignature);
    }
    // This detached, domain-separated owner action must be independently
    // signed; simply signing the mailbox policy does NOT approve installation.
    // The empty-directory NOREPLACE gate is only a CI first-install CAS.
    // B1d0's external trusted monotonic witness must be composed before ship.
    let proposal = approval::verify_first_mail_approval(
        separate_approval,
        *blake3::hash(signed).as_bytes(),
        claims,
        claims.operator_public, // verified against fixed root-owned RootTrust
        0, // disposable CI genesis; NOT a production witness observation
        now_ms,
    ).map_err(|_| PublishHold::InvalidOwnerApproval)?;
    if proposal.required_next_revision != 1 || !proposal.requires_independent_cas_commit {
        return Err(PublishHold::InvalidOwnerApproval);
    }
    let floor = policy::mail_epoch_floor_for_lab(signed, 1);
    let receipt = policy::mail_commit_receipt_for_lab(signed, &floor, 1);
    let records = [
        (FILES[0].0, signed),
        (FILES[1].0, floor.as_slice()),
        (FILES[2].0, receipt.as_slice()),
    ];
    for (name, bytes) in records {
        staged(&dir, gid, name, bytes)?;
    }
    dir.sync_all().map_err(err)?;
    if stop == PublishStop::AfterPendingSync {
        return Err(PublishHold::InjectedStop);
    }
    publish(&dir, FILES[0].0, FILES[0].1)?;
    if stop == PublishStop::AfterPolicyRename {
        return Err(PublishHold::InjectedStop);
    }
    publish(&dir, FILES[1].0, FILES[1].1)?;
    if stop == PublishStop::AfterFloorRename {
        return Err(PublishHold::InjectedStop);
    }
    publish(&dir, FILES[2].0, FILES[2].1)?;
    if stop == PublishStop::AfterReceiptRename {
        return Err(PublishHold::InjectedStop);
    }
    verify_published(
        &dir,
        &[
            (FILES[0].1, signed),
            (FILES[1].1, &floor),
            (FILES[2].1, &receipt),
        ],
    )?;
    Ok(())
}
