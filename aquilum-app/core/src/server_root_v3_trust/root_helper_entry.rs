//! One-shot, non-shipping entry point for the future root-owned helper ELF.
//! The caller cannot choose a release key, path, UID, signer, clock, policy,
//! Unix listener, or command. Only an already connected AF_UNIX fd 0 may be
//! inspected, after the *running* executable has matched independently
//! enrolled root-owned signed bytes. This function NEVER calls B1d0 CAS.
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;

pub fn authenticated_owner_preview_once_fd0() -> bool {
    if rustix::process::geteuid().as_raw() != 0 {
        return false;
    }
    if super::bootstrap_helper::inspect_provisioned_running_helper_hold().is_err() {
        return false;
    }

    // SAFETY: a one-shot executable owns its stdin for the rest of its
    // lifetime. FromRawFd transfers exactly that existing fd; it does NOT
    // open a path or create a socket. A pipe, file or detached descriptor
    // fails the kernel SO_PEERCRED Unix stream check inside preview_once.
    let mut connected = unsafe { UnixStream::from_raw_fd(0) };
    match super::owner_ipc::preview_provisioned_once(&mut connected) {
        Ok(checked) => {
            // A validated B1b1b owner frame is still only preview data:
            // independent B0/B1a, witness CAS and explicit install dispatch
            // are NOT performed here and cannot be requested over this fd.
            checked.requires_independent_b0_b1a_cas
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonroot_process_cannot_gain_root_helper_preflight() {
        if rustix::process::geteuid().as_raw() != 0 {
            assert!(!authenticated_owner_preview_once_fd0());
        }
    }
}
