//! Exercises the complete native notify -> WorkspaceWatcher -> SearchService
//! path, including both event batching stages. Run on Windows/macOS/Linux.
use aquilum_core::files::watcher::{WatchBatch, WatchScope, WatchSink, WorkspaceWatcher};
use aquilum_core::search::models::SearchIndexState;
use aquilum_core::search::paths::is_markdown;
use aquilum_core::search::query::Request;
use aquilum_core::search::SearchService;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn wait_for(
    stage: &str,
    timeout: Duration,
    mut condition: impl FnMut() -> bool,
) -> Duration {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return start.elapsed();
        }
        thread::sleep(Duration::from_millis(15));
    }
    panic!("{stage} did not complete within {timeout:?}");
}

fn search_contains(search: &SearchService, workspace: &str, term: &str, path: &Path) -> bool {
    let response = search
        .search(workspace, &Request::new(term))
        .expect("prepared search index must remain queryable");
    response.results.iter().any(|result| {
        Path::new(&result.path).file_name() == path.file_name()
    })
}

#[test]
fn native_workspace_watcher_to_search_roundtrip() {
    const TIMEOUT: Duration = Duration::from_secs(30);

    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("Vault");
    fs::create_dir_all(&workspace).unwrap();
    let user_dot_dir = workspace.join(".заметки");
    fs::create_dir(&user_dot_dir).unwrap();
    let internal_dir = workspace.join(".git");
    fs::create_dir(&internal_dir).unwrap();
    let private = internal_dir.join("private.md");
    fs::write(&private, "watcherservicesecretunique").unwrap();
    // Force the initial scan to build a real index before monitoring edits.
    fs::write(workspace.join("Baseline.md"), "# Baseline\nseedcontent\n").unwrap();

    let workspace_name = workspace.to_string_lossy().into_owned();
    let search = Arc::new(SearchService::new(
        directory.path().join("search"),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    search.prepare(&workspace_name).unwrap();
    wait_for("initial scan", TIMEOUT, || {
        let status = search.status(Some(&workspace_name));
        status.state == SearchIndexState::Ready && !status.updating && status.indexed_documents == 1
    });

    let delivered = Arc::new(AtomicUsize::new(0));
    let sink_search = Arc::clone(&search);
    let sink_delivered = Arc::clone(&delivered);
    let sink: WatchSink = Arc::new(move |batch: WatchBatch| {
        sink_delivered.fetch_add(1, Ordering::Relaxed);
        let rescan = batch.scope == WatchScope::Rescan;
        let notes: Vec<PathBuf> = if rescan {
            batch.paths
        } else {
            batch.paths.into_iter().filter(|p| is_markdown(p)).collect()
        };
        if !notes.is_empty() || rescan {
            sink_search.ingest_watch(notes, rescan);
        }
    });

    let watcher = WorkspaceWatcher::new(sink);
    watcher.watch(&workspace).unwrap();

    let note = workspace.join("Заметка.md");
    let create_term = "firstwatchcreateduniquealpha";
    let modify_term = "secondwatchmodifieduniquebeta";
    let started = Instant::now();
    fs::write(&note, format!("# Test\n{create_term}\n")).unwrap();
    let created = wait_for("native create to search", TIMEOUT, || {
        search_contains(&search, &workspace_name, create_term, &note)
    });
    println!("REAL_WATCHER os={} stage=create elapsed_ms={:.3}", std::env::consts::OS, created.as_secs_f64() * 1_000.0);

    fs::write(&note, format!("# Test\n{modify_term}\n")).unwrap();
    let modified = wait_for("native modify to search", TIMEOUT, || {
        search_contains(&search, &workspace_name, modify_term, &note)
            && !search_contains(&search, &workspace_name, create_term, &note)
    });
    println!("REAL_WATCHER os={} stage=modify elapsed_ms={:.3}", std::env::consts::OS, modified.as_secs_f64() * 1_000.0);

    fs::remove_file(&note).unwrap();
    let deleted = wait_for("native delete to search", TIMEOUT, || {
        !search_contains(&search, &workspace_name, modify_term, &note)
    });
    println!("REAL_WATCHER os={} stage=delete elapsed_ms={:.3}", std::env::consts::OS, deleted.as_secs_f64() * 1_000.0);
    // Real OS-level notification from a user-owned dot folder must reach Search.
    // A service folder already contained a Markdown file during the initial scan.
    assert!(!search_contains(&search, &workspace_name, "watcherservicesecretunique", &private));
    let dot_note = user_dot_dir.join("Идея.md");
    let dot_created_term = "userdotwatchcreateduniquegamma";
    let dot_modified_term = "userdotwatchmodifieduniquedelta";
    fs::write(&dot_note, format!("# User dot folder\n{dot_created_term}\n")).unwrap();
    let dot_created = wait_for("user dot folder native create", TIMEOUT, || {
        search_contains(&search, &workspace_name, dot_created_term, &dot_note)
    });
    println!("REAL_WATCHER os={} stage=user_dot_create elapsed_ms={:.3}", std::env::consts::OS, dot_created.as_secs_f64() * 1_000.0);
    fs::write(&dot_note, format!("# User dot folder\n{dot_modified_term}\n")).unwrap();
    let dot_modified = wait_for("user dot folder native modify", TIMEOUT, || {
        search_contains(&search, &workspace_name, dot_modified_term, &dot_note)
            && !search_contains(&search, &workspace_name, dot_created_term, &dot_note)
    });
    println!("REAL_WATCHER os={} stage=user_dot_modify elapsed_ms={:.3}", std::env::consts::OS, dot_modified.as_secs_f64() * 1_000.0);
    fs::remove_file(&dot_note).unwrap();
    let dot_deleted = wait_for("user dot folder native delete", TIMEOUT, || {
        !search_contains(&search, &workspace_name, dot_modified_term, &dot_note)
    });
    println!("REAL_WATCHER os={} stage=user_dot_delete elapsed_ms={:.3}", std::env::consts::OS, dot_deleted.as_secs_f64() * 1_000.0);
    assert!(delivered.load(Ordering::Relaxed) >= 1, "native watcher must deliver at least one event");
    println!(
        "REAL_WATCHER os={} stage=summary callbacks={} initial_to_cleanup_ms={:.3} native_notifications=true search_updates_verified=true",
        std::env::consts::OS,
        delivered.load(Ordering::Relaxed),
        started.elapsed().as_secs_f64() * 1_000.0,
    );

    watcher.stop();
    // Release the write guard before stopping the worker; otherwise its
    // temporary borrow can outlive the search service at function exit.
    let opened = search.active.write().unwrap().take();
    if let Some(index) = opened {
        index.stop();
    }
}
