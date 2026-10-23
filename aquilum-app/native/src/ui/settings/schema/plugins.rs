use super::*;

pub(super) fn demo(config: &AppConfig) -> Vec<Block> {
    let state = &config.plugin_alpha;
    let (status, tone) = if !state.installed {
        (t("settings.plugins.off"), Tone::Muted)
    } else if !state.enabled {
        (t("settings.plugins.disabled"), Tone::Muted)
    } else {
        (t("settings.plugins.ready"), Tone::Secondary)
    };
    let mut actions = Vec::new();
    if !state.installed {
        actions.push(Button::new(t("settings.plugins.install"), Action::PluginDemoInstall));
    } else {
        if state.enabled {
            actions.push(Button::new(t("settings.plugins.run"), Action::PluginDemoRun));
            actions.push(Button::new(t("settings.plugins.disable"), Action::PluginDemoDisable));
        } else {
            actions.push(Button::new(t("settings.plugins.enable"), Action::PluginDemoEnable));
        }
        actions.push(Button::new(t("settings.plugins.remove"), Action::PluginDemoRemove));
    }
    let mut rows = vec![
        info(
            t("settings.plugins.demo"),
            Some(t("settings.plugins.description")),
            Control::Status { text: status, tone },
        ),
        info(
            t("settings.plugins.status"),
            Some(t("settings.plugins.notice")),
            Control::Buttons(actions),
        ),
    ];
    if let Some(count) = state.last_words {
        rows.push(info(
            t("settings.plugins.status"),
            None,
            Control::Status {
                text: t_with("settings.plugins.last", &[("count", &count.to_string())]),
                tone: Tone::Secondary,
            },
        ));
    }
    if let Some(error) = &state.last_error {
        rows.push(info(
            t("settings.plugins.status"),
            None,
            Control::Status { text: error.clone(), tone: Tone::Danger },
        ));
    }
    vec![Block { title: t("settings.plugins.section"), rows }]
}
