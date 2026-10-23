use super::super::*;

#[test]
fn b1d0_native_server_settings_have_three_fields_and_never_grant_install() {
    use base64::Engine as _;
    let mut config = AppConfig::default();
    let ctx = Context::new(1.0);
    i18n::set_language("ru");
    let page = &blocks(Section::Server, &config, &ctx)[0];
    let writable: Vec<_> = page.rows.iter().filter(|r| r.setter.is_some()).collect();
    assert_eq!(writable.len(), 3, "three fields and no SSH port on the main form");
    assert!(writable.iter().all(|r| matches!(r.control, Control::Text { commit: true, .. })));
    let buttons: Vec<_> = page.rows.iter().filter_map(|r| match &r.control {
        Control::Buttons(list) => Some(list.as_slice()),
        _ => None,
    }).flatten().collect();
    assert!(buttons.iter().any(|b| b.action == Action::ToggleServerAdvanced));
    assert!(buttons.iter().any(|b| b.action == Action::VerifyServer && !b.enabled));
    assert!(buttons.iter().any(|b| b.action == Action::InstallServer && !b.enabled));
    assert!(!page.rows.iter().any(|r| r.label == t("settings.server.offlineCheck")));

    let pin = writable.iter().find(|r| r.label == t("settings.server.identity")).unwrap();
    let Some(Setter::Config(set)) = pin.setter else { panic!("identity setter missing") };
    let mut wire = b"\0\0\0\x0bssh-ed25519\0\0\0\x20".to_vec();
    wire.extend_from_slice(&[0x45; 32]);
    let public = format!(
        "ssh-ed25519 {}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(&wire),
    );
    set(&mut config, &Value::Text(public.clone()));
    assert_eq!(config.server_setup.host_public_key, public);
    let fingerprint = config.server_setup.host_key_fingerprint.clone();
    assert!(fingerprint.starts_with("SHA256:"));
    config.server_setup.host = "203.0.113.10".into();
    config.server_setup.ssh_user = "ubuntu".into();
    assert!(config.server_setup.valid_offline_pin());

    set(&mut config, &Value::Text(fingerprint.clone()));
    assert_eq!(config.server_setup.host_public_key, "");
    assert_eq!(config.server_setup.host_key_fingerprint, fingerprint);
    assert!(server::identity::can_probe(&config.server_setup));
    set(&mut config, &Value::Text("-----BEGIN OPENSSH PRIVATE KEY-----".into()));
    assert!(config.server_setup.host_public_key.is_empty());
    assert!(config.server_setup.host_key_fingerprint.is_empty());
    assert!(!server::identity::can_probe(&config.server_setup));

    let mut advanced = Context::new(1.0);
    advanced.server_advanced = true;
    let more = blocks(Section::Server, &config, &advanced);
    assert_eq!(more[0].rows.iter().filter(|r| r.setter.is_some()).count(), 4);
    assert!(more[0].rows.iter().any(|r| r.label == t("settings.server.port")));
    i18n::set_language("en");
}

#[test]
fn b1_e2e_01_native_verify_button_never_grants_install() {
    use base64::Engine;
    use sha2::Digest;
    let mut wire = b"\0\0\0\x0bssh-ed25519\0\0\0\x20".to_vec();
    wire.extend_from_slice(&[7u8;32]);
    assert_eq!(wire.len(), 51);
    let public = format!(
        "ssh-ed25519 {}",
        base64::engine::general_purpose::STANDARD.encode(&wire),
    );
    let fingerprint = format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD
            .encode(sha2::Sha256::digest(&wire)),
    );
    let mut config = AppConfig::default();
    config.server_setup.host = "vps.example.org".into();
    config.server_setup.ssh_user = "ubuntu".into();
    config.server_setup.host_public_key = public;
    config.server_setup.host_key_fingerprint = fingerprint;
    assert!(config.server_setup.valid_offline_pin());
    let mut ctx = Context::new(1.0);
    let buttons = |config: &AppConfig, ctx: &Context| {
        let page = &blocks(Section::Server, &config, ctx)[0];
        let buttons = page.rows.iter().find_map(|row| {
            if row.label == t("settings.server.verify")
                && let Control::Buttons(ref list) = row.control {
                    Some(list.clone())
                } else { None }
        }).expect("server actions");
        assert_eq!(buttons.len(), 2);
        assert!(matches!(buttons[0].action, Action::VerifyServer));
        assert!(!buttons[1].enabled, "a read-only KEX is never an install grant");
        buttons
    };
    let prepared = buttons(&config, &ctx);
    assert_eq!(prepared[0].enabled, cfg!(all(
        target_os = "windows", feature = "server-readonly-preflight",
    )));
    ctx.server_probe = Some(ServerProbeView::Checking);
    assert!(!buttons(&config, &ctx)[0].enabled, "duplicate SSH probes must be disabled");
    ctx.server_probe = Some(ServerProbeView::Observed {
        facts: "Ubuntu / x86_64; 4 CPU; 8192 MiB RAM".into(),
    });
    let observed = blocks(Section::Server, &config, &ctx);
    assert!(!observed[0].rows.iter().any(|row|
        row.label == t("settings.server.observedFacts")
    ), "details are hidden in the default three-field view");
    ctx.server_advanced = true;
    let details = blocks(Section::Server, &config, &ctx);
    assert!(details[0].rows.iter().any(|row|
        row.label == t("settings.server.observedFacts")
    ));
    assert!(!buttons(&config, &ctx)[1].enabled);
    let mut bad = config.clone();
    bad.server_setup.host_public_key = "-----BEGIN OPENSSH PRIVATE KEY-----".into();
    assert!(!bad.server_setup.valid_offline_pin());
    assert!(!buttons(&bad, &ctx)[0].enabled);
}
