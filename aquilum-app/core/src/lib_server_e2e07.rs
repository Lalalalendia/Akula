// Feature-gated root-level server modules; no default install authority.
#[cfg(feature = "server-bootstrap-b1a-release-lab")]
pub mod server_release_lab;
#[cfg(feature = "server-install-review")]
pub mod server_install_root_gate;
#[cfg(feature = "server-install-review")]
pub mod server_first_root_stage;
// Shared Windows/Linux canonical B1 owner signing bytes; not install authority.
#[cfg(any(feature = "server-install-review", all(target_os = "linux", feature = "server-bootstrap-b1b1b-approval-lab")))]
pub mod server_owner_approval_wire;
#[cfg(all(target_os = "windows", feature = "server-owner-windows-signer"))]
pub mod server_owner_windows;
#[cfg(all(target_os = "linux", feature = "server-root-b0v3-gate-lab"))]
pub mod server_root_v3_guard;
#[cfg(all(target_os = "linux", feature = "server-root-b0v3-gate-lab"))]
pub mod server_root_v3_trust;
