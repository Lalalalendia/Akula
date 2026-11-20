use super::*;
#[test]
fn installed_quickjs_arguments_are_explicit_and_disable_path_runtimes() {
    let url = "https://youtu.be/dQw4w9WgXcQ";
    let rt = OsStr::new("C:\\LocalAppData\\Akula\\tools\\quickjs\\qjs.exe");
    let args = command_args_with_js(url, Path::new("C:\\staging"), SINGLE_VIDEO_MAX, Some(rt))
        .into_iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
    assert_eq!(args[0], "--no-js-runtimes");
    assert_eq!(args[1], "--js-runtimes");
    assert_eq!(args[2], "quickjs:C:\\LocalAppData\\Akula\\tools\\quickjs\\qjs.exe");
    assert!(args.contains(&"--ignore-config".to_owned()));
    assert!(args.contains(&"--no-plugin-dirs".to_owned()));
    assert!(args.contains(&"--no-remote-components".to_owned()));
    assert_eq!(args[args.len() - 2], "--");
    assert_eq!(args.last().unwrap(), url);
    assert_eq!(args.iter().filter(|a| a.starts_with("quickjs:")).count(), 1);
}
#[test]
fn local_runtime_must_be_absolute_before_download_staging() {
    let temp = tempfile::tempdir().unwrap();
    let library = Library::at(temp.path().join("videos"), &[]).unwrap();
    let result = download_one_with_js(
        &library, OsStr::new("missing-yt-dlp"), OsStr::new("relative/qjs.exe"),
        "https://youtu.be/dQw4w9WgXcQ", |_| {},
    );
    assert!(result.unwrap_err().contains("absolute"));
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0);
}
#[test]
fn nonshipping_legacy_adapter_does_not_inherit_quickjs() {
    let args = command_args_with_js("https://youtu.be/dQw4w9WgXcQ",
        Path::new("/tmp/isolated"), SINGLE_VIDEO_MAX, None);
    assert!(!args.iter().any(|a| a == "--js-runtimes"));
    assert!(!args.iter().any(|a| a.to_string_lossy().starts_with("quickjs:")));
}
