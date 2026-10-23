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
mod server;
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
        Section::Sync => {
            let mut blocks = vec![yandex_setup(config)];
            blocks.extend(sync_recovery(ctx));
            #[cfg(feature = "dropbox-readonly-preview")]
            blocks.push(dropbox_setup(config, ctx));
            blocks
        },
        Section::Server => vec![server_setup(config)],
        Section::History => history(config),
        Section::Trash => trash(config, ctx),
        Section::System => system(config, ctx),
        Section::Shortcuts => shortcuts(),
    }
}
