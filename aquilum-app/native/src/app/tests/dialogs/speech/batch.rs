#![cfg(target_os = "linux")]

use super::*;
use crate::workspace::speech_jobs::Notice;
use std::os::unix::fs::PermissionsExt;

#[test]
#[ignore = "requires real FFprobe and FFmpeg; uses a deterministic local Whisper fixture"]
fn real_speech_batch_from_settings_keeps_queue_order_and_respects_pause() {
    let tool = |key| PathBuf::from(std::env::var_os(key).unwrap());
    let probe = tool("AKULA_ASR_PROBE");
    let ffmpeg = tool("AKULA_ASR_FFMPEG");
    let mut h = Harness::new();
    let tools = h.app.workspace.data_dir.join("test-tools");
    std::fs::create_dir(&tools).unwrap();
    let whisper = tools.join("test-whisper");
    let model = tools.join("model");
    let vad = tools.join("vad");
    std::fs::write(&whisper, r#"#!/bin/sh
[ "$OMP_THREAD_LIMIT" = 1 ] && [ "$OPENBLAS_NUM_THREADS" = 1 ] || exit 3
if [ ! -f "${0%/*}/permit" ]; then
    : > "${0%/*}/recognizing"
    exec /bin/sleep 30
fi
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-t" ]; then
        shift
        [ "$1" = 1 ] || exit 4
    fi
    if [ "$1" = "-of" ]; then
        shift
        printf '%s' '{"transcription":[{"offsets":{"from":100,"to":1000},"text":"Verified batch cue"}]}' > "$1.json"
        exit
    fi
    shift
done
exit 2
"#).unwrap();
    std::fs::set_permissions(&whisper, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&model, "fixture").unwrap();
    std::fs::write(&vad, "fixture").unwrap();

    let first = h.vault.path().join("first.mkv");
    let second = h.vault.path().join("second.wav");
    let third = h.vault.path().join("third.wav");
    let invalid = h.vault.path().join("invalid.mp4");
    assert!(
        std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:a",
                "pcm_s16le"
            ])
            .arg(&second)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=128x72:d=2",
                "-i"
            ])
            .arg(&second)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le"
            ])
            .arg(&first)
            .status()
            .unwrap()
            .success()
    );
    std::fs::copy(&second, &third).unwrap();
    std::fs::write(&invalid, "not a video").unwrap();
    let originals = [&first, &second, &third].map(|path| std::fs::read(path).unwrap());

    h.app.open_settings(&mut h.root);
    h.settle();
    h.open_section(Section::System);
    for (role, path) in Tool::ALL
        .into_iter()
        .zip([probe, ffmpeg, whisper, model, vad])
    {
        h.app.workspace.speech().send(Command::SetTool(role, path));
    }
    wait(&mut h, View::ready);
    press(&mut h, Speech::CpuMode(speech::CpuMode::Quiet));
    wait(&mut h, |view| view.cpu_mode == speech::CpuMode::Quiet);
    let batch = vec![first.clone(), invalid, second.clone(), third.clone()];
    for _ in 0..2 {
        h.app
            .workspace
            .speech()
            .send(Command::Action(Speech::Refresh));
        wait(&mut h, |view| view.notice.is_none());
        h.app.workspace.speech().send(Command::Batch(batch.clone()));
        let view = wait(&mut h, |view| {
            !view.preparing
                && matches!(&view.notice, Some(Notice::Batch(report)) if report.accepted == 3 && report.rejected == 1)
        });
        assert_eq!(view.jobs.len(), 3);
    }
    let database = h.app.workspace.data_dir.join("media-jobs.sqlite");
    let queue = Queue::open(&database).unwrap();
    let jobs = queue.list(speech::KIND, None, 10).unwrap();
    let (first_id, second_id, third_id) = (jobs[2].id, jobs[1].id, jobs[0].id);
    for (job, path) in jobs.iter().rev().zip([&first, &second, &third]) {
        assert_eq!(job.spec.source, path.to_str().unwrap());
    }
    let recipe: serde_json::Value = serde_json::from_str(&jobs[2].spec.recipe).unwrap();
    assert_eq!(recipe["track"], 1);
    assert!(jobs.iter().all(|job| job.state == State::Queued));

    press(&mut h, Speech::RunQueue);
    wait(&mut h, |view| {
        view.processing_queue
            && view.active == Some(first_id)
            && tools.join("recognizing").is_file()
    });
    press(&mut h, Speech::StopQueue);
    wait(&mut h, |view| {
        !view.processing_queue && view.active.is_none()
    });
    assert_eq!(queue.get(first_id).unwrap().unwrap().state, State::Paused);
    assert_eq!(queue.get(second_id).unwrap().unwrap().state, State::Queued);

    std::fs::write(tools.join("permit"), "").unwrap();
    press(&mut h, Speech::RunQueue);
    wait(&mut h, |view| {
        !view.processing_queue
            && [second_id, third_id].into_iter().all(|id| {
                view.jobs
                    .iter()
                    .any(|job| job.id == id && job.state == State::Completed)
            })
    });
    assert_eq!(queue.get(first_id).unwrap().unwrap().state, State::Paused);
    press(&mut h, Speech::Run(first_id));
    wait(&mut h, |view| {
        view.jobs
            .iter()
            .any(|job| job.id == first_id && job.state == State::Completed)
    });
    for ((id, path), original) in [(first_id, first), (second_id, second), (third_id, third)]
        .into_iter()
        .zip(originals)
    {
        let mut vtt = Vec::new();
        speech::write_vtt(&queue, id, &mut vtt).unwrap();
        assert!(
            String::from_utf8(vtt)
                .unwrap()
                .contains("Verified batch cue")
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}
