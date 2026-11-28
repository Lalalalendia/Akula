use super::*;

#[test]
fn automatic_drain_runs_each_queued_job_and_keeps_paused_jobs_paused() {
    let temp = tempfile::tempdir().unwrap();
    let queue = open(temp.path());
    let first = seed(&queue, 1);
    let paused = seed(&queue, 2);
    let last = seed(&queue, 3);
    queue.pause(paused).unwrap();

    let mut config = config::Config::default();
    let tool = temp.path().join("tool");
    std::fs::write(&tool, "placeholder").unwrap();
    for role in Tool::ALL {
        config.choose(temp.path(), role, tool.clone()).unwrap();
    }
    let dispatcher = Dispatcher::new(temp.path().into(), Arc::new(|| {}));
    until(&dispatcher, View::ready);
    dispatcher.send(Command::Action(Action::RunQueue));
    until(&dispatcher, |view| {
        !view.processing_queue && !view.has_queued && view.active.is_none()
    });
    for id in [first, last] {
        // The deliberately invalid recipe fails after a claim. The following
        // job still has to run instead of leaving the automatic drain stalled.
        assert_eq!(queue.get(id).unwrap().unwrap().state, State::Failed);
    }
    assert_eq!(queue.get(paused).unwrap().unwrap().state, State::Paused);
}

#[test]
fn queue_rejects_oversized_batches_before_opening_tools() {
    let temp = tempfile::tempdir().unwrap();
    let dispatcher = Dispatcher::new(temp.path().into(), Arc::new(|| {}));
    until(&dispatcher, |view| view.loaded);
    dispatcher.send(Command::Batch(vec![
        PathBuf::from("unused");
        speech::MAX_BATCH_FILES + 1
    ]));
    until(&dispatcher, |view| {
        view.notice == Some(Notice::BatchTooLarge)
    });
    assert!(
        open(temp.path())
            .list(speech::KIND, None, 1)
            .unwrap()
            .is_empty()
    );
}
