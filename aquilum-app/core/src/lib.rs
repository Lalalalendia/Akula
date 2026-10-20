//! The Aquilum core: notes on disk, documents, search, history, settings and UI state, with no
//! knowledge of the UI that hosts it.

pub mod app_core;
pub mod documents;
pub mod files;
pub mod history;
pub mod link_title;
pub mod markdown;
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

#[cfg(feature = "server-bootstrap-b0-lab")]
pub mod server_bootstrap_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b0-openssh-lab"))]
pub mod server_bootstrap_ssh_lab;

#[cfg(feature = "server-bootstrap-b1a-release-lab")]
pub mod server_release_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b-custody-lab"))]
pub mod server_staging_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1-floor-lab"))]
pub mod server_release_floor_lab;

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1b-approval-lab"))]
pub mod server_owner_approval_lab;

#[cfg(all(target_os = "linux", feature = "node-catalog-lab"))]
pub mod node_catalog_lab;
