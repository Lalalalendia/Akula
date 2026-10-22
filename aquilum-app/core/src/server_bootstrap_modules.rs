//! Platform-gated bootstrap exports kept at the crate root.
#[cfg(feature = "server-bootstrap-b0-lab")]
pub mod server_bootstrap_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b0-openssh-lab"))]
pub mod server_bootstrap_ssh_lab;
#[cfg(all(target_os = "windows", feature = "server-bootstrap-b0-openssh-lab"))]
pub mod server_bootstrap_ssh_windows_lab;

#[cfg(feature = "server-bootstrap-b1a-release-lab")]
pub mod server_release_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b-custody-lab"))]
pub mod server_staging_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1-floor-lab"))]
pub mod server_release_floor_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1b-approval-lab"))]
pub mod server_owner_approval_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1c-witness-lab"))]
pub mod server_witness_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b3a-helper-ipc-lab"))]
pub mod server_installer_ipc_lab;
#[cfg(all(target_os = "linux",feature = "server-bootstrap-b1b3c-pinned-fd-lab"))]
pub mod server_installer_pinned_release_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b4a-cas-fence-lab"))]
pub mod server_installer_cas_fence_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b4b-os-witness-lab"))]
pub mod server_installer_os_witness_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1c0-fixed-node-lab"))]
pub mod server_installer_fixed_node_lab;
#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1d0-durable-node"))]
pub mod server_installer_durable_node;

#[cfg(all(target_os = "linux", feature = "node-catalog-lab"))]
pub mod node_catalog_lab;

