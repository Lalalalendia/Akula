//! Explicit QA-only migration of a verified Notion API cache to Akula-owned notes.
//! No credentials or network traffic. Heavy SQLite read and File Gate writes
//! run on workers; the native event loop receives metadata-only completions.
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc::{self, Receiver, Sender}};
use aquilum_core::{Core, notion_api_probe::{
    cached_import::{approved_cached_preview, read_verified_cached, CachedImportReview},
    publish::publish_approved, structured_bundle, identity_registry, bases_model,
}};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage { #[default] Idle, Reviewing, Ready, Publishing, Complete, NeedsReview }

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Самостоятельный импорт ещё не проверялся",
            Self::Reviewing => "Проверяю снимок API в отдельном потоке...",
            Self::Ready => "Снимок проверен. Для импорта требуется отдельное подтверждение.",
            Self::Publishing => "Создаю независимые заметки через File Gate...",
            Self::Complete => "Заметки, типизированный JSON и граф ID сохранены; локальная модель баз читается без Notion. Редактирование и медиа ещё не восстановлены.",
            Self::NeedsReview => "Импорт приостановлен: источник, база или права требуют проверки.",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportView {
    pub root: Option<String>,
    pub target: Option<PathBuf>,
    pub stage: Stage,
    pub busy: bool,
    pub pages: usize,
    pub links: usize,
    pub digest: Option<String>,
    pub created: usize,
    pub existing: usize,
    pub conflicts: usize,
    pub structured_saved: bool,
    pub identities_saved: bool,
    pub identities_need_review: usize,
    pub base_schemas: usize,
    pub base_rows: usize,
    pub base_views: usize,
    pub base_issues: usize,
    pub bases_reconstructible: bool,
}

impl ImportView {
    pub fn ready_for(&self, root: &str, target: &Path) -> bool {
        self.stage == Stage::Ready && !self.busy
            && self.root.as_deref() == Some(root)
            && self.target.as_deref() == Some(target)
            && self.digest.as_ref().is_some_and(|d| d.len() == 64)
    }
}

#[derive(Debug)]
struct Summary {
    created: usize,
    existing: usize,
    conflicts: usize,
    completed: bool,
    structured_saved: bool,
    identities_saved: bool,
    identities_need_review: usize,
    base_issues: usize,
    bases_reconstructible: bool,
}
#[derive(Clone, Copy, Debug, Default)]
struct BaseStats {
    schemas: usize,
    rows: usize,
    views: usize,
    issues: usize,
}
impl BaseStats {
    fn from_model(model: &bases_model::BasesModel) -> Self {
        Self {
            schemas: model.bases.len(), rows: model.rows.len(),
            views: model.views.len(),
            issues: model.fields_needing_review + model.unbound_rows + model.unbound_views,
        }
    }
}
enum ResultKind { Review(CachedImportReview, BaseStats), Published(Summary) }
struct Completion {
    revision: u64,
    root: String,
    target: PathBuf,
    result: Result<ResultKind, ()>,
}
pub struct ImportSession {
    revision: u64,
    active: bool,
    view: ImportView,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl ImportSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { revision: 0, active: false, view: ImportView::default(), tx, rx, wake }
    }
    pub fn view(&self) -> ImportView { self.view.clone() }
    pub fn busy(&self) -> bool { self.active }
    pub fn refused(&mut self) {
        if !self.active { self.view.stage = Stage::NeedsReview; }
    }

    fn start(
        &mut self, root: String, target: PathBuf, stage: Stage,
        job: impl FnOnce() -> Result<ResultKind, ()> + Send + 'static,
    ) -> Result<(), &'static str> {
        if self.active { return Err("NOTION_IMPORT_BUSY"); }
        if uuid::Uuid::parse_str(&root).map(|u| u.to_string()).ok().as_deref()
            != Some(root.as_str())
        {
            return Err("NOTION_IMPORT_INVALID_ROOT");
        }
        if target.as_os_str().is_empty() { return Err("NOTION_IMPORT_NO_WORKSPACE"); }
        let revision = self.revision.checked_add(1).ok_or("NOTION_IMPORT_SEQUENCE_EXHAUSTED")?;
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let reply_root = root.clone();
        let reply_target = target.clone();
        std::thread::Builder::new()
            .name("akula-notion-independent-import".into())
            .spawn(move || {
                // Never expose source text, token or OS paths in a panic message.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                    .unwrap_or(Err(()));
                if tx.send(Completion {
                    revision, root: reply_root, target: reply_target, result,
                }).is_ok() { wake(); }
            })
            .map_err(|_| "NOTION_IMPORT_WORKER_UNAVAILABLE")?;
        self.revision = revision;
        self.active = true;
        self.view = ImportView {
            root: Some(root), target: Some(target), stage, busy: true,
            ..ImportView::default()
        };
        Ok(())
    }

    /// Preview is read-only, token-free and bound to the currently selected Vault.
    pub fn review(&mut self, data_dir: PathBuf, root: String, target: PathBuf)
        -> Result<(), &'static str>
    {
        let source = root.clone();
        self.start(root, target, Stage::Reviewing, move || {
            let cache=data_dir.join("ui-state.sqlite3");
            let (review, _) = read_verified_cached(&cache, &source).map_err(|_| ())?;
            if !review.can_publish_notes { return Err(()); }
            let model=bases_model::from_verified_cache(&cache,&source,&review.digest)
                .map_err(|_| ())?;
            let stats=BaseStats::from_model(&model);
            Ok(ResultKind::Review(review,stats))
        })
    }

    /// Only the exact approved snapshot and selected Vault may be published.
    /// The Core File Gate rechecks registration, path safety and create-only.
    pub fn publish(
        &mut self, core: Arc<Core>, data_dir: PathBuf,
        root: String, target: PathBuf,
    ) -> Result<(), &'static str> {
        if !self.view.ready_for(&root, &target) {
            return Err("NOTION_IMPORT_REVIEW_REQUIRED");
        }
        let digest = self.view.digest.clone().ok_or("NOTION_IMPORT_NO_DIGEST")?;
        let source = root.clone();
        let chosen = target.clone();
        self.start(root, target, Stage::Publishing, move || {
            let cache = data_dir.join("ui-state.sqlite3");
            // Prebuild and validate the complete typed JSON source BEFORE any
            // Markdown File Gate mutation. Exact digest fences a concurrent refresh.
            let bundle = structured_bundle::prepare_from_cached(&cache, &source, &digest)
                .map_err(|_| ())?;
            // Register identities and resolve graph edges before ANY Vault write.
            // A partial schema is stored explicitly, never promoted to Bases.
            let identities = identity_registry::prepare_from_cached(&cache, &source, &digest)
                .map_err(|_| ())?;
            let identities_need_review = identities.review().unresolved_edge_count
                + identities.review().unmatched_row_property_count
                + identities.review().unidentified_schema_property_count
                + identities.review().unresolved_source_target_count;
            let expected_bases=bases_model::from_verified_cache(&cache,&source,&digest)
                .map_err(|_| ())?;
            let expected_stats=BaseStats::from_model(&expected_bases);
            let preview = approved_cached_preview(&cache, &source, &digest)
                .map_err(|_| ())?;
            let report = publish_approved(&core, &chosen, &preview, true).map_err(|_| ())?;
            // Retain one source JSON. A separate fourth user-data copy is not
            // needed: rebuild the normalized model after File Gate readback.
            let structured_receipt=if report.completed {
                structured_bundle::publish_approved(&core,&chosen,&bundle,true).ok()
            } else { None };
            let structured_saved=structured_receipt.is_some();
            let bases_reconstructible=structured_receipt.as_ref().is_some_and(|saved| {
                bases_model::from_independent_bundle(&chosen.join(&saved.relative_path))
                    .ok().is_some_and(|offline| {
                        offline.source_root==source && offline.source_digest==digest
                            && offline.bases.len()==expected_stats.schemas
                            && offline.rows.len()==expected_stats.rows
                            && offline.views.len()==expected_stats.views
                            && BaseStats::from_model(&offline).issues==expected_stats.issues
                    })
            });
            let identities_saved = bases_reconstructible
                && identity_registry::publish_approved(&core, &chosen, &identities, true).is_ok();
            Ok(ResultKind::Published(Summary {
                created: report.created.len(),
                existing: report.existing_untouched.len(),
                conflicts: report.conflicts_untouched.len(),
                // Unresolved source relations or unmatched typed property IDs
                // are a review requirement, not a successful Bases migration.
                completed: report.completed && structured_saved && bases_reconstructible
                    && identities_saved && identities_need_review == 0
                    && expected_stats.issues == 0,
                structured_saved, identities_saved, identities_need_review,
                bases_reconstructible, base_issues:expected_stats.issues,
            }))
        })
    }

    /// Constant-time GUI poll; never performs IO and discards stale responses.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            if !self.active || done.revision != self.revision
                || self.view.root.as_deref() != Some(done.root.as_str())
                || self.view.target.as_deref() != Some(done.target.as_path())
            { continue; }
            self.active = false;
            self.view.busy = false;
            match (self.view.stage, done.result) {
                (Stage::Reviewing, Ok(ResultKind::Review(review,stats)))
                    if review.can_publish_notes =>
                {
                    self.view.pages = review.page_count;
                    self.view.links = review.graph_link_count;
                    self.view.base_schemas=stats.schemas;
                    self.view.base_rows=stats.rows;
                    self.view.base_views=stats.views;
                    self.view.base_issues=stats.issues;
                    self.view.digest = Some(review.digest);
                    self.view.stage = Stage::Ready;
                }
                (Stage::Publishing, Ok(ResultKind::Published(summary))) => {
                    self.view.created = summary.created;
                    self.view.existing = summary.existing;
                    self.view.conflicts = summary.conflicts;
                    self.view.structured_saved = summary.structured_saved;
                    self.view.identities_saved = summary.identities_saved;
                    self.view.identities_need_review = summary.identities_need_review;
                    self.view.bases_reconstructible=summary.bases_reconstructible;
                    self.view.base_issues=summary.base_issues;
                    self.view.stage = if summary.completed {
                        Stage::Complete
                    } else { Stage::NeedsReview };
                }
                _ => self.view.stage = Stage::NeedsReview,
            }
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::mpsc, time::Duration};

    const ROOT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    fn review() -> CachedImportReview {
        CachedImportReview {
            root: ROOT.into(), digest: "a".repeat(64), saved_at_ms: 1,
            page_count: 2, graph_link_count: 1, unresolved_count: 0,
            can_publish_notes: true, full_notion_migration_complete: false,
        }
    }
    #[test]
    fn review_requires_matching_root_target_and_single_flight() {
        let (tx, rx) = mpsc::channel();
        let mut s = ImportSession::new(Arc::new(move || { let _ = tx.send(()); }));
        let target = PathBuf::from("synthetic-workspace");
        s.start(ROOT.into(), target.clone(), Stage::Reviewing, || {
            Ok(ResultKind::Review(review(),BaseStats::default()))
        }).unwrap();
        assert!(!s.view().ready_for(ROOT, &target));
        assert!(s.start(ROOT.into(), target.clone(), Stage::Reviewing, || {
            Ok(ResultKind::Review(review(),BaseStats::default()))
        }).is_err());
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(s.poll());
        assert!(s.view().ready_for(ROOT, &target));
        assert!(!s.view().ready_for(ROOT, Path::new("other-workspace")));
        assert!(!s.view().ready_for("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", &target));
        assert_eq!(s.view().digest.as_deref(), Some("a".repeat(64).as_str()));
        assert_eq!(s.view().pages, 2);
    }
    #[test]
    fn failed_read_or_incomplete_source_never_enables_publish() {
        let temp = tempfile::tempdir().unwrap();
        let mut s = ImportSession::new(Arc::new(|| {}));
        s.review(temp.path().into(), ROOT.into(), temp.path().into()).unwrap();
        for _ in 0..150 {
            if s.poll() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.view().stage, Stage::NeedsReview);
        assert!(!s.view().ready_for(ROOT, temp.path()));
        assert!(!temp.path().join("ui-state.sqlite3").exists());
        assert!(!temp.path().join("Notion-import").exists());
    }
    #[test]
    fn publish_receipt_never_treats_partial_writes_as_complete() {
        let (tx, rx) = mpsc::channel();
        let mut s = ImportSession::new(Arc::new(move || { let _ = tx.send(()); }));
        let target = PathBuf::from("selected-vault");
        s.start(ROOT.into(), target.clone(), Stage::Reviewing,
            || Ok(ResultKind::Review(review(),BaseStats::default()))).unwrap();
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        s.poll();
        s.start(ROOT.into(), target, Stage::Publishing, || {
            Ok(ResultKind::Published(Summary {
                created: 1, existing: 0, conflicts: 1, completed: false,
                structured_saved: false,
                identities_saved: false, identities_need_review: 1,
                bases_reconstructible:false,base_issues:2,
            }))
        }).unwrap();
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(s.poll());
        assert_eq!(s.view().stage, Stage::NeedsReview);
        assert_eq!(s.view().created, 1);
        assert_eq!(s.view().conflicts, 1);
        assert!(!s.view().structured_saved);
        assert!(!s.view().identities_saved);
        assert_eq!(s.view().identities_need_review, 1);
        assert!(!s.view().bases_reconstructible);
        assert_eq!(s.view().base_issues, 2);
    }
}
