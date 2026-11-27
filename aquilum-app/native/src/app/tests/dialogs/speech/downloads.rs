#![cfg(target_os = "linux")]

use super::*;
use crate::ui::editor::resolve::Request;
use crate::video::queue::{self, Status};
use crate::workspace::speech_jobs::Notice;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn system(h: &mut Harness) {
    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
}

fn download(h: &mut Harness, id: &str, succeeded: bool) {
    let url = format!("https://youtu.be/{id}");
    h.app
        .on_editor_request(&mut h.root, Request::DownloadVideo { url: url.clone() });
    wait(h, |_| match queue::status(&url) {
        Some(Status::Ready(path)) if succeeded => path.is_file(),
        Some(Status::Failed(_)) if !succeeded => true,
        _ => false,
    });
}

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
#[ignore = "requires a deterministic downloader fixture and real FFprobe"]
fn speech_download_intake_tracks_publication_and_respects_local_opt_in() {
    let mut h = Harness::new();
    system(&mut h);
    let data = h.app.workspace.data_dir.clone();
    let probe = data.join("test-ffprobe");
    // Hold one local probe open while the user disables admission.
    executable(
        &probe,
        r#"#!/bin/sh
for file do :; done
case "$file" in
    *AutoOptOut1.mp4)
        : > "${0%/*}/probe-started"
        while [ ! -e "${0%/*}/release-probe" ]; do sleep 0.03; done
    ;;
esac
exec "$AKULA_ASR_PROBE" "$@"
"#,
    );
    let placeholder = data.join("placeholder");
    std::fs::write(&placeholder, "unused").unwrap();
    for (role, path) in Tool::ALL.into_iter().zip([
        probe,
        placeholder.clone(),
        placeholder.clone(),
        placeholder.clone(),
        placeholder,
    ]) {
        h.app.workspace.speech().send(Command::SetTool(role, path));
    }
    wait(&mut h, View::ready);

    assert!(!h.app.workspace.speech().view().auto_downloads);
    download(&mut h, "AutoOff0001", true);
    press(&mut h, Speech::AutoDownloads(true));
    wait(&mut h, |view| view.auto_downloads);

    let mut h = h.restarted(1.0);
    system(&mut h);
    wait(&mut h, |view| view.loaded && view.auto_downloads);
    download(&mut h, "AutoOff0001", true);
    download(&mut h, "AutoFail001", false);

    download(&mut h, "AutoOn00001", true);
    wait(&mut h, |view| {
        view.jobs.len() == 1
            && matches!(&view.notice, Some(Notice::DownloadQueued(path)) if path.ends_with("AutoOn00001.mp4"))
    });

    download(&mut h, "AutoOptOut1", true);
    wait(&mut h, |view| {
        view.admitting_download.is_some() && data.join("probe-started").is_file()
    });
    press(&mut h, Speech::AutoDownloads(false));
    wait(&mut h, |view| {
        !view.auto_downloads && view.admitting_download.is_none()
    });
    press(&mut h, Speech::AutoDownloads(true));
    wait(&mut h, |view| view.auto_downloads);
    std::fs::write(data.join("release-probe"), "").unwrap();

    download(&mut h, "AutoOn00002", true);
    wait(&mut h, |view| {
        view.admitting_download.is_none()
            && matches!(&view.notice, Some(Notice::DownloadQueued(path)) if path.ends_with("AutoOn00002.mp4"))
    });
    let queue = Queue::open(&data.join("media-jobs.sqlite")).unwrap();
    let jobs = queue.list(speech::KIND, None, 10).unwrap();
    let library = crate::video::library::default_root(&data)
        .unwrap()
        .canonicalize()
        .unwrap();
    for (job, name) in jobs.iter().zip(["AutoOn00002.mp4", "AutoOn00001.mp4"]) {
        assert_eq!(Path::new(&job.spec.source), library.join(name));
        assert_eq!(job.state, State::Queued);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&job.spec.recipe).unwrap()["track"],
            1
        );
    }
    assert_eq!(jobs.len(), 2);
}
