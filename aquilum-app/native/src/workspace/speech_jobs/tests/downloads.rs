use super::*;

#[test]
fn speech_tools_from_older_profiles_leave_download_admission_off() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path();
    std::fs::write(
        profile.join("speech-tools.json"),
        r#"{"files":[null,null,null,null,null]}"#,
    )
    .unwrap();
    let dispatcher = Dispatcher::new(profile.into(), Arc::new(|| {}));
    assert!(!until(&dispatcher, |view| view.loaded).auto_downloads);
    dispatcher.send(Command::Action(Action::AutoDownloads(true)));
    until(&dispatcher, |view| {
        matches!(&view.notice, Some(Notice::Error(_)))
    });
    assert!(!config::Config::load(profile).unwrap().auto_downloads);
}
