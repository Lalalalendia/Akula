use super::*;
use aquilum_core::settings::models::ServerBootstrapDraft;

/// Native first-stage owner-server setup: *public* draft only.
/// No SSH private key, password, sudo credential or token is accepted.
/// Matching public host identity is NOT permission to connect or install.
pub(super) fn server_setup(config: &AppConfig, ctx: &Context) -> Block {
    let draft = &config.server_setup;
    let pin_matches = draft.valid_offline_pin();
    let probe = ctx.server_probe.as_ref();
    let probing = matches!(probe, Some(ServerProbeView::Checking));
    let state_key = match probe {
        Some(ServerProbeView::Checking) => "settings.server.probing",
        Some(ServerProbeView::Observed { .. }) => "settings.server.verifiedReadOnly",
        Some(ServerProbeView::Unsupported { .. }) => "settings.server.unsupported",
        Some(ServerProbeView::Failed) => "settings.server.verificationFailed",
        None => "settings.server.notConnected",
    };
    let mut rows = vec![
            info(t("settings.server.state"), Some(t("settings.server.stateHint")),
                Control::Status {
                    text: t(state_key),
                    tone: if matches!(probe, Some(ServerProbeView::Observed { .. })) {
                        Tone::Secondary
                    } else { Tone::Danger },
                }),
            row(t("settings.server.host"), Some(t("settings.server.hostHint")),
                Control::Text { value: draft.host.clone(), commit: true },
                |c, v| {
                    let raw = text(v);
                    if ServerBootstrapDraft::valid_host(&raw) {
                        c.server_setup.host = raw;
                    }
                }),
            row(t("settings.server.port"), Some(t("settings.server.portHint")),
                Control::Number(Number::new(draft.ssh_port as f64,
                    1.0, 65535.0, 1.0)),
                |c, v| {
                    let num = number(v);
                    if num.is_finite() && (1.0..=65535.0).contains(&num)
                        && num.fract() == 0.0 {
                        c.server_setup.ssh_port = num as u16;
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
            row(t("settings.server.publicKey"),
                Some(t("settings.server.publicKeyHint")),
                Control::Text { value: draft.host_public_key.clone(), commit: true },
                |c, v| {
                    let raw = text(v);
                    if ServerBootstrapDraft::valid_public_key(&raw) {
                        c.server_setup.host_public_key = raw;
                    }
                }),
            row(t("settings.server.fingerprint"),
                Some(t("settings.server.fingerprintHint")),
                Control::Text {
                    value: draft.host_key_fingerprint.clone(), commit: true,
                },
                |c, v| {
                    let raw = text(v);
                    if ServerBootstrapDraft::valid_fingerprint(&raw) {
                        c.server_setup.host_key_fingerprint = raw;
                    }
                }),
            info(t("settings.server.offlineCheck"),
                Some(t("settings.server.offlineCheckHint")),
                Control::Status {
                    text: t(if pin_matches {
                        "settings.server.pinConsistent"
                    } else {
                        "settings.server.pinIncomplete"
                    }),
                    tone: if pin_matches { Tone::Secondary } else { Tone::Danger },
                }),
            info(t("settings.server.noSecrets"),
                Some(t("settings.server.noSecretsHint")),
                Control::Status {
                    text: t("settings.server.noCredentials"), tone: Tone::Danger,
                }),
            info(t("settings.server.verify"), Some(t("settings.server.verifyHint")),
                Control::Buttons(vec![
                    Button::new(t("settings.server.verifyButton"),
                        Action::VerifyServer)
                        .enabled(cfg!(all(target_os = "windows",
                            feature = "server-readonly-preflight"))
                            && pin_matches && !probing),
                    Button::new(t("settings.server.installButton"),
                        Action::OpenUrl("https://github.com/Lalalalendia/Akula"))
                        .enabled(false),
                ])),
    ];
    if let Some(ServerProbeView::Observed { facts }
        | ServerProbeView::Unsupported { facts }) = probe
    {
        // Only normalized, bounded typed host facts; never SSH stderr,
        // shell output, credentials or an installation-success claim.
        rows.push(info(
            t("settings.server.observedFacts"),
            Some(t("settings.server.observedFactsHint")),
            Control::Status {
                text: facts.clone(),
                tone: Tone::Secondary,
            },
        ));
    }
    Block { title: t("settings.server.title"), rows }
}
