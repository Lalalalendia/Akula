use std::path::{Path, PathBuf};
use aquilum_core::files::deletions::{TrashedDeletion, deletion_page_impl};
use aquilum_core::files::trash::trash_state_impl;
use aquilum_core::mcp::McpStatus;
use aquilum_core::settings::models::AppConfig;
use crate::i18n::{self, plural, t, t_with};
use crate::interface::{MAX_SCALE, MIN_SCALE, SCALE_STEP};
mod appearance;
mod data;
mod mcp;
mod section;
mod support;
mod system;
mod sync;
pub(crate) mod server;
#[cfg(test)]
mod tests;
mod trash;
mod types;

pub use appearance::reader_quick;
pub use data::templates_path;
pub use mcp::new_token;
pub use section::Section;
pub use types::*;

use appearance::*;
use data::*;
use mcp::*;
use support::*;
use system::*;
use sync::*;
use server::*;
use trash::*;

pub fn blocks(section: Section, config: &AppConfig, ctx: &Context) -> Vec<Block> {
    match section {
        Section::Ui => ui(config, ctx),
        Section::Editor => editor(config),
        Section::Reader => reader(config),
        Section::Data => [templates(config, ctx), files(config)].into_iter().flatten().collect(),
        Section::Analysis => analysis(config),
        Section::Mcp => mcp(config, ctx),
        #[cfg(feature = "dropbox-readonly-preview")]
        Section::Sync => std::iter::once(yandex_setup(config, ctx)).chain(sync_recovery(ctx)).chain(std::iter::once(dropbox_setup(config, ctx))).collect(),
        #[cfg(not(feature = "dropbox-readonly-preview"))]
        Section::Sync => std::iter::once(yandex_setup(config, ctx)).chain(sync_recovery(ctx)).collect(),
        Section::Server => vec![server_setup(config, ctx)],
        Section::History => history(config),
        Section::Trash => trash(config, ctx),
        Section::System => system(config, ctx),
        Section::Shortcuts => shortcuts(),
    }
}
