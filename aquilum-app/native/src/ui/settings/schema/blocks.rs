use super::*;

pub fn blocks(section: Section, config: &AppConfig, ctx: &Context) -> Vec<Block> {
    match section {
        Section::Ui => ui(config, ctx),
        Section::Editor => editor(config),
        Section::Reader => reader(config),
        Section::Data => [templates(config, ctx), files(config)].into_iter().flatten().collect(),
        Section::Analysis => analysis(config),
        Section::Mcp => mcp(config, ctx),
        Section::Sync => std::iter::once(yandex_setup(config)).chain(sync_recovery(ctx)).collect(),
        Section::Server => vec![server_setup(config, ctx)],
        Section::History => history(config),
        Section::Trash => trash(config, ctx),
        Section::System => system(config, ctx),
        Section::Shortcuts => shortcuts(),
    }
}
