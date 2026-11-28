use super::*;

#[test]
#[ignore = "downloads pinned upstream tools and requires AKULA_ASR_SAMPLE"]
fn fresh_windows_profile_installs_speech_and_transcribes_from_system_settings() {
    let source = PathBuf::from(std::env::var_os("AKULA_ASR_SAMPLE").expect("spoken WAV"));
    let mut h = Harness::new();
    let profile = h.app.workspace.data_dir.clone();
    let local_source = profile.join("Проверка речи.wav");
    std::fs::copy(source, &local_source).unwrap();
    let original = std::fs::read(&local_source).unwrap();

    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    wait(&mut h, |view| {
        view.loaded && view.tools.iter().all(Option::is_none)
    });
    press(&mut h, Speech::InstallTools);
    let end = Instant::now() + Duration::from_secs(300);
    loop {
        h.app.tick(&mut h.root, Instant::now());
        let view = h.app.workspace.speech().view();
        if view.ready() {
            assert_eq!(
                view.notice,
                Some(crate::workspace::speech_jobs::Notice::ToolkitInstalled)
            );
            break;
        }
        assert!(
            !matches!(
                view.notice,
                Some(crate::workspace::speech_jobs::Notice::Error(_))
            ) && Instant::now() < end,
            "speech setup: {view:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    h.app
        .workspace
        .speech()
        .send(Command::Inspect(local_source.clone()));
    let ready = wait(&mut h, |view| view.selection.is_some());
    let track = ready.selection.unwrap().tracks[0].index;
    press(&mut h, Speech::UseTrack(track));
    let id = wait(&mut h, |view| view.has_queued).jobs[0].id;
    press(&mut h, Speech::Run(id));
    let result = wait(&mut h, |view| {
        view.jobs
            .iter()
            .any(|row| row.id == id && matches!(row.state, State::Completed | State::Failed))
    });
    assert_eq!(result.jobs[0].state, State::Completed, "{result:?}");

    let captions = profile.join("captions.vtt");
    h.app
        .workspace
        .speech()
        .send(Command::Export(id, captions.clone()));
    wait(&mut h, |view| {
        view.notice == Some(crate::workspace::speech_jobs::Notice::Exported)
    });
    let text = std::fs::read_to_string(captions).unwrap().to_lowercase();
    assert!(
        text.contains("fellow americans") && text.contains(" --> "),
        "{text}"
    );
    assert_eq!(std::fs::read(local_source).unwrap(), original);

    assert!(profile.join("speech-tools.json").is_file());
}
