//! VIDEO-TOOLS-01h NON-RELEASE: harmless Windows child process fixture.
//! This executable is source-built on the runner, copied under test-only
//! component filenames, and NEVER includes the real video tools or updater.
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fail(reason: &str) -> ! {
    eprintln!("VIDEO_TOOLS_01H_CHILD_HOLD: {reason}");
    std::process::exit(3);
}
fn main() {
    if std::env::args().collect::<Vec<_>>().get(1).map(String::as_str)
        != Some("--akula-01h-child")
        || std::env::args().len() != 2
        || std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("RUNNER_OS").as_deref() != Ok("Windows")
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
        || std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request")
    {
        fail("incorrect benign-child invocation or non-PR environment");
    }
    let work = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01H_WORK")
        .unwrap_or_else(|| fail("ephemeral work directory missing")));
    if !work.is_dir() {
        fail("ephemeral work directory unavailable");
    }
    fs::write(work.join("01h-child-started"), b"benign-image-started")
        .unwrap_or_else(|_| fail("cannot prove child image start"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !work.join("01h-finish").is_file() {
        if Instant::now() >= deadline {
            fail("test parent did not complete second mutation attempt");
        }
        std::thread::sleep(Duration::from_millis(70));
    }
    fs::write(work.join("01h-child-exited"), b"benign-image-exited")
        .unwrap_or_else(|_| fail("cannot prove clean benign child exit"));
}
