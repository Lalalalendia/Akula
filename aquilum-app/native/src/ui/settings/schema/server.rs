use super::*;
use aquilum_core::settings::models::ServerBootstrapDraft;

pub(crate) mod identity;
mod server_status;

/// One ordinary-user server form. Independent host identity stays public;
/// neither a successful SSH probe nor a UI click authorizes installation.
pub(super) fn server_setup(config: &AppConfig, ctx: &Context) -> Block {
    let draft = &config.server_setup;
    let probe = ctx.server_probe.as_ref();
    let probing = matches!(probe, Some(ServerProbeView::Checking));
    let mut rows = vec![
        row(t("settings.server.host"), Some(t("settings.server.hostHint")),
            Control::Text { value: draft.host.clone(), commit: true },
            |c, v| {
                let raw = text(v);
                if ServerBootstrapDraft::valid_host(&raw) {
                    c.server_setup.host = raw;
                }
            }),
        row(t("settings.server.user"), Some(t("settings.server.userHint")),
            Control::Text { value: draft.ssh_user.clone(), commit: true },
            |c, v| {
                let raw = text(v);
                if ServerBootstrapDraft::valid_user(&raw) {
                    c.server_setup.ssh_user = raw;
                }
            }),
        row(t("settings.server.identity"), Some(t("settings.server.identityHint")),
            Control::Text { value: identity::display(draft), commit: true },
            |c, v| identity::update(&mut c.server_setup, &text(v))),
        info(t("settings.server.safety"), None,
            Control::Status {
                text: t("settings.server.shortHint"), tone: Tone::Muted,
            }),
        info(t("settings.server.advanced"), None,
            Control::Buttons(vec![Button::new(t(if ctx.server_advanced {
                "settings.server.advancedHide"
            } else {
                "settings.server.advancedShow"
            }), Action::ToggleServerAdvanced)])),
    ];
    if ctx.server_advanced {
        rows.push(row(t("settings.server.port"), Some(t("settings.server.portHint")),
            Control::Number(Number::new(draft.ssh_port as f64,
                1.0, 65535.0, 1.0)),
            |c, v| {
                let num = number(v);
                if num.is_finite() && (1.0..=65535.0).contains(&num)
                    && num.fract() == 0.0
                {
                    c.server_setup.ssh_port = num as u16;
                }
            }));
        server_status::append_facts(&mut rows, probe);
    }
    rows.push(server_status::status_row(probe));
    rows.push(info(t("settings.server.verify"), None,
        Control::Buttons(vec![
            Button::new(t("settings.server.verifyButton"), Action::VerifyServer)
                .enabled(cfg!(all(windows, feature = "server-readonly-preflight"))
                    && identity::can_probe(draft) && !probing),
            // B1 installation is enabled only when signed installer, explicit
            // owner approval, real systemd health and recovery are connected.
            Button::new(t("settings.server.installButton"), Action::InstallServer)
                .enabled(false),
        ])));
    Block { title: t("settings.server.title"), rows }
}
