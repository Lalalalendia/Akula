//! Single-flight, user-triggered Notion read-only refresh coordinator.
//! HTTP, OS token retrieval, SQLite migrations, and snapshot publication run
//! off the GUI thread. The UI observes only a fixed, non-secret result.
use std::path::PathBuf;
use std::sync::{Arc, mpsc::{self, Receiver, Sender}};
use std::time::{SystemTime, UNIX_EPOCH};

use aquilum_core::notion_api_probe::{
    http::HttpReadClient, refresh::{refresh_root, RefreshChange}, snapshot::SnapshotStore,
    ProbeLimits,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Idle, Reading, FirstCopy, Updated, Unchanged, Failed, CredentialUnavailable,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Notion не обновлялся в этом сеансе",
            Self::Reading => "Читаю Notion через API (только чтение)...",
            Self::FirstCopy => "Первая локальная копия сохранена",
            Self::Updated => "Локальная копия обновлена",
            Self::Unchanged => "Изменений нет",
            Self::Failed => "Обновление не удалось; прежняя копия сохранена",
            Self::CredentialUnavailable => "Ключ Notion не найден либо недоступен",
        }
    }
}
#[derive(Clone, Debug)]
pub struct SyncView {
    pub root: Option<String>,
    pub status: Status,
    pub busy: bool,
}
impl Default for SyncView {
    fn default() -> Self {
        Self { root: None, status: Status::Idle, busy: false }
    }
}
struct Completion {
    generation: u64,
    root: String,
    outcome: Status,
}
pub struct RefreshSession {
    generation: u64,
    in_flight: bool,
    view: SyncView,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl RefreshSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            generation: 0, in_flight: false, view: SyncView::default(),
            tx, rx, wake,
        }
    }
    pub fn view(&self) -> SyncView { self.view.clone() }
    pub fn busy(&self) -> bool { self.in_flight }

    pub fn credential_failure(&mut self) {
        if !self.in_flight { self.view.status = Status::CredentialUnavailable; }
    }
    pub fn forgotten(&mut self, root: &str) {
        if !self.in_flight && self.view.root.as_deref() == Some(root) {
            self.view = SyncView::default();
        }
    }

    /// Explicit user action only. Start rejects parallel credential changes or
    /// fetches; multiple roots are supported sequentially.
    pub fn start(&mut self, data_dir: PathBuf, root: String) -> Result<(), &'static str> {
        self.start_operation(root.clone(), move || fetch(data_dir, root))
    }
    fn start_operation(
        &mut self,
        root: String,
        operation: impl FnOnce() -> Status + Send + 'static,
    ) -> Result<(), &'static str> {
        if self.in_flight { return Err("NOTION_REFRESH_BUSY"); }
        if uuid::Uuid::parse_str(&root).map(|id| id.to_string()).ok().as_deref()
            != Some(root.as_str())
        {
            return Err("INVALID_NOTION_ROOT");
        }
        let generation = self.generation.checked_add(1).ok_or("SEQUENCE_EXHAUSTED")?;
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let reply_root = root.clone();
        // Untrusted HTTP errors and panic payloads must not reach UI logs.
        std::thread::Builder::new()
            .name("akula-notion-manual-read".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                    .unwrap_or(Status::Failed);
                if tx.send(Completion {
                    generation, root: reply_root, outcome,
                }).is_ok() {
                    wake();
                }
            })
            .map_err(|_| "NOTION_WORKER_UNAVAILABLE")?;
        self.generation = generation;
        self.in_flight = true;
        self.view = SyncView {
            root: Some(root), status: Status::Reading, busy: true,
        };
        Ok(())
    }

    /// GUI polling is bounded and involves no network or SQLite operations.
    /// Successful completions return a root which the existing offline
    /// NotionSession can re-open and independently hash-verify.
    pub fn poll(&mut self) -> (bool, Option<String>) {
        let mut changed = false;
        let mut confirmed_root = None;
        while let Ok(reply) = self.rx.try_recv() {
            if reply.generation != self.generation
                || self.view.root.as_deref() != Some(&reply.root)
                || !self.in_flight
            { continue; }
            self.in_flight = false;
            self.view.busy = false;
            self.view.status = reply.outcome;
            if matches!(reply.outcome,
                Status::FirstCopy | Status::Updated | Status::Unchanged)
            {
                confirmed_root = Some(reply.root);
            }
            changed = true;
        }
        (changed, confirmed_root)
    }
}
fn fetch(data_dir: PathBuf, root: String) -> Status {
    let result = crate::notion_credentials::with_token(&root, |token| {
        let mut client = HttpReadClient::new(token.to_owned()).map_err(|_| ())?;
        let mut store = SnapshotStore::open(&data_dir.join("ui-state.sqlite3"))
            .map_err(|_| ())?;
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| ())?
            .as_millis();
        let ms = i64::try_from(ms).map_err(|_| ())?;
        refresh_root(&mut client, &mut store, &root, ProbeLimits::default(), ms)
            .map(|receipt| receipt.change).map_err(|_| ())
    });
    match result {
        Ok(Ok(RefreshChange::FirstCopy)) => Status::FirstCopy,
        Ok(Ok(RefreshChange::Changed)) => Status::Updated,
        Ok(Ok(RefreshChange::Unchanged)) => Status::Unchanged,
        Ok(Err(())) => Status::Failed,
        Err(_) => Status::CredentialUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    #[test]
    fn one_refresh_at_a_time_and_no_stale_ui_reply() {
        let (wake, rx) = mpsc::channel();
        let mut s = RefreshSession::new(Arc::new(move || {
            let _ = wake.send(());
        }));
        s.start_operation(A.into(), || Status::Updated).unwrap();
        assert_eq!(s.view().status, Status::Reading);
        assert!(s.start_operation(B.into(), || Status::FirstCopy).is_err());
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (changed, root) = s.poll();
        assert!(changed);
        assert_eq!(root.as_deref(), Some(A));
        assert_eq!(s.view().status, Status::Updated);
        s.start_operation(B.into(), || Status::Failed).unwrap();
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(s.poll(), (true, None));
        assert_eq!(s.view().status, Status::Failed);
    }

    #[test]
    fn invalid_root_never_starts_network_worker() {
        let mut session = RefreshSession::new(Arc::new(|| {}));
        assert!(session.start_operation("../../../escape".into(), || Status::Updated).is_err());
        assert!(!session.busy());
        assert!(session.view().root.is_none());
    }
}
