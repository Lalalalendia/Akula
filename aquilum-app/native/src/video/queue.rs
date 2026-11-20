use std::collections::HashMap; use std::ffi::OsString; use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use super::{download, library::Library, youtube};
#[derive(Clone, Debug)]
pub enum Status {
    Queued,
    Downloading(download::Progress),
    Ready(PathBuf),
    Failed(String),
}
static JOBS: OnceLock<Mutex<HashMap<String, Status>>> = OnceLock::new(); static SERIAL: Mutex<()> = Mutex::new(());
fn jobs() -> &'static Mutex<HashMap<String, Status>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn change(id: &str, status: Status) {
    jobs().lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_owned(), status);
}
pub fn status(url: &str) -> Option<Status> {
    let id = youtube::video_id(url)?;
    jobs().lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}
pub fn enqueue(
    url: String,
    data_dir: PathBuf,
    registered_roots: Vec<PathBuf>,
    changed: Arc<dyn Fn(bool, &str) + Send + Sync>,
) -> Result<bool, &'static str> {
    let id = youtube::video_id(&url).ok_or("invalid single-video URL")?.to_owned();
    {
        let mut list = jobs().lock().unwrap_or_else(|e| e.into_inner());
        if matches!(list.get(&id), Some(Status::Queued | Status::Downloading(_))) {
            return Ok(false);
        }
        list.insert(id.clone(), Status::Queued);
    }
    changed(false, &url);
    std::thread::spawn(move || {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        change(&id, Status::Downloading(download::Progress::default()));
        changed(false, &url);
        let mut last_notification = 0_u64;
        #[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
        let executable = super::optional_ytdlp::verified_executable(&data_dir)
            .map(|path| path.into_os_string())
            .map_err(|_| "Install the verified yt-dlp in Settings > System first".to_owned());
        #[cfg(not(all(windows, feature = "installed", not(feature = "video-ui-e2e"))))]
        let executable: Result<OsString, String> = Ok(std::env::var_os("AKULA_YT_DLP_BIN")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("yt-dlp")));
        #[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
        let runtime: Result<Option<OsString>, String> = super::optional_ytdlp::quickjs::verified_executable(&data_dir)
            .map(|p| Some(p.into_os_string()))
            .map_err(|_| "Install verified QuickJS in Settings > System first".to_owned());
        #[cfg(not(all(windows, feature = "installed", not(feature = "video-ui-e2e"))))]
        let runtime: Result<Option<OsString>, String> = Ok(None);
        let result = executable.and_then(|executable| runtime.and_then(|runtime| {
            Library::new(&data_dir, &registered_roots)
                .map_err(|e| format!("local video storage rejected: {e}"))
                .and_then(|library| {
                    super::queue_dispatch::download_one(&library, &executable, runtime.as_deref(), &url, |progress| {
                        change(&id, Status::Downloading(progress));
                        if progress.downloaded.saturating_sub(last_notification) >= 1024 * 1024
                            || progress.downloaded == progress.total.unwrap_or(u64::MAX)
                        {
                            last_notification = progress.downloaded;
                            changed(false, &url);
                        }
                    })
                })
        }));
        match result {
            Ok(path) => change(&id, Status::Ready(path)),
            Err(error) => change(&id, Status::Failed(error)),
        }
        changed(true, &url);
    });
    Ok(true)
}
#[cfg(all(test, windows, feature = "installed", not(feature = "video-ui-e2e")))]
#[path = "queue_01k_tests.rs"]
mod installed_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_link_never_creates_a_download_job() {
        let dir = tempfile::tempdir().unwrap();
        let notify = Arc::new(|_: bool, _: &str| {});
        assert!(enqueue(
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ".into(),
            dir.path().to_path_buf(), vec![], notify
        ).is_err());
        assert!(status("https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ").is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
