//! The Aquilum core: notes on disk, documents, search, history, settings and UI state, with no
//! knowledge of the UI that hosts it.

pub mod app_core;
pub mod documents;
pub mod files;
pub mod history;
pub mod link_title;
pub mod markdown;
#[cfg(feature = "messaging-core-lab")]
pub mod messaging;
pub mod mcp;
pub mod plugins;
pub mod migration;
pub mod search;
pub mod settings;
pub mod sync;
pub mod text;
pub mod ui_state;
pub mod web_agent;
pub mod wikixiv;
pub use app_core::{Core, CoreEvent, EventSink, TaskFailed};
include!("lib_server_e2e07.rs");
#[cfg(feature = "server-bootstrap-b0-lab")]
pub mod server_bootstrap_lab;
#[cfg(all(target_os = "linux", feature = "server-bootstrap-b0-openssh-lab"))]
pub mod server_bootstrap_ssh_lab;
#[cfg(all(target_os = "windows", feature = "server-bootstrap-b0-openssh-lab"))]
pub mod server_bootstrap_ssh_windows_lab;

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
#[cfg(all(target_os = "linux", feature = "node-replica-lab"))]
pub mod node_replica_lab;
#[cfg(all(target_os = "linux", feature = "node-replica-auth-lab"))]
pub mod node_replica_auth_lab;
#[cfg(all(target_os = "linux", feature = "node-v1"))]
pub mod node_v1;
// Optional, inert P2P offer/consent laboratory; not file authorization.
#[cfg(feature = "p2p-offer-lab")]
pub mod p2p_offer_lab; #[cfg(feature = "canvas-p2p-wire-lab")] pub mod canvas_p2p_wire_lab;
#[cfg(feature = "p2p-inbox-lab")]
pub mod p2p_inbox_lab;
#[cfg(feature = "p2p-device-identity")]
pub mod p2p_device_identity;
