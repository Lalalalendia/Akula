//! Per-tab PDF state and out-of-process page rasterization.
//! PDF source bytes are read-only. Worker is not yet an OS security sandbox.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use masonry::app::RenderRoot;
use masonry::core::ErasedAction;
use masonry::peniko::ImageBrush;
use uuid::Uuid;
use super::App;
use crate::ui::pdf::{self, Display};

const MAX_PNG: u64 = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(25);
pub(super) struct State {
    views: HashMap<Uuid, View>,
    jobs: Sender<RenderRequest>,
    rx: Receiver<ResultPage>,
    desired: Arc<AtomicU64>,
    generation: u64,
}
pub(super) struct View {
    path: PathBuf,
    pub(super) current: usize,
    pub(super) total: usize,
    pub(super) zoom: f64,
    pub(super) rotation: i32,
    pub(super) scroll: f64,
    pub(super) image: Option<ImageBrush>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) generation: u64,
}
impl View {
    // Reusing one Akula tab for another PDF must not show the old raster.
    // A new request gets a fresh generation; older replies are discarded.
    fn retarget(&mut self, path: &Path) -> bool {
        if self.path == path { return false; }
        self.path = path.to_path_buf();
        self.current = 0;
        self.total = 1;
        self.zoom = 1.0;
        self.rotation = 0;
        self.scroll = 0.0;
        self.image = None;
        self.loading = false;
        self.error = None;
        true
    }
}
struct RenderRequest {
    tab: Uuid,
    generation: u64,
    path: PathBuf,
    page: usize,
    width: u32,
    rotation: i32,
    wake: Arc<dyn Fn() + Send + Sync>,
}
struct ResultPage {
    tab: Uuid,
    generation: u64,
    image: Result<(Vec<u8>, usize), String>,
}
fn superseded(desired: &AtomicU64, generation: u64) -> bool {
    desired.load(Ordering::Acquire) != generation
}

pub(super) fn is_pdf(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

pub(super) fn trusted_source(vault: &Path, source: &Path) -> bool {
    let Ok(info) = std::fs::symlink_metadata(source) else { return false };
    if !info.file_type().is_file() || !(16..=256 * 1024 * 1024).contains(&info.len()) {
        return false;
    }
    match (vault.canonicalize(), source.canonicalize()) {
        (Ok(root), Ok(path)) if path.starts_with(&root) => {
            std::fs::File::open(&path).ok().and_then(|mut f| {
                use std::io::Read;
                let mut head = [0; 5];
                f.read_exact(&mut head).ok().map(|()| head == *b"%PDF-")
            }).unwrap_or(false)
        }
        _ => false,
    }
}

impl State {
    pub(super) fn new() -> Self {
        let (results, rx) = std::sync::mpsc::channel::<ResultPage>();
        let (jobs, pending) = std::sync::mpsc::channel::<RenderRequest>();
        let desired = Arc::new(AtomicU64::new(0));
        let cancel = Arc::clone(&desired);
        // Exactly one PDFium child at a time, regardless of rapid navigation.
        std::thread::Builder::new().name("akula-pdf-render".into()).spawn(move || {
            while let Ok(mut request) = pending.recv() {
                // Keep only the newest pending request when pages are flipped quickly.
                while let Ok(next) = pending.try_recv() { request = next; }
                if superseded(&cancel, request.generation) { continue; }
                let image = render(
                    &request.path, request.page, request.width, request.rotation,
                    &cancel, request.generation,
                );
                if superseded(&cancel, request.generation) { continue; }
                if results.send(ResultPage {
                    tab: request.tab, generation: request.generation, image,
                }).is_err() { break; }
                (request.wake)();
            }
        }).expect("PDF render worker thread");
        Self { views: HashMap::new(), jobs, rx, desired, generation: 0 }
    }
    pub(super) fn clear(&mut self) {
        self.views.clear();
        self.generation = self.generation.wrapping_add(1);
        self.desired.store(self.generation, Ordering::Release);
    }
    pub(super) fn open(&mut self, tab: Uuid, path: &Path, wake: &std::sync::Arc<dyn Fn() + Send + Sync>) {
        // Only the visible PDF retains a decoded page image. A 16-Mpixel
        // raster can consume 64 MiB: never retain one per inactive tab.
        for (id, other) in &mut self.views {
            if *id != tab { other.image = None; }
        }
        let view = self.views.entry(tab).or_insert_with(|| View {
            path: path.to_path_buf(), current: 0, total: 1, zoom: 1.0, rotation: 0, scroll: 0.0,
            image: None, loading: false, error: None, generation: 0,
        });
        view.retarget(path);
        // A previous tab's newer generation cancels this tab's old
        // in-flight request. Reactivating it must queue a fresh render.
        if view.loading && superseded(&self.desired, view.generation) {
            view.loading = false;
        }
        if view.image.is_none() && !view.loading && view.error.is_none() {
            self.request(tab, wake);
        }
    }
    pub(super) fn display(&self, tab: Uuid) -> Option<Display> {
        let view = self.views.get(&tab)?;
        Some(Display {
            image: view.image.clone(),
            name: view.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            current: view.current, total: view.total, zoom: view.zoom,
            rotation: view.rotation, scroll: view.scroll, loading: view.loading,
            error: view.error.clone(),
        })
    }
    fn request(&mut self, tab: Uuid, wake: &std::sync::Arc<dyn Fn() + Send + Sync>) {
        let Some(view) = self.views.get_mut(&tab) else { return };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        view.generation = generation;
        view.loading = true;
        view.error = None;
        self.desired.store(generation, Ordering::Release);
        let job = RenderRequest {
            tab, generation, path: view.path.clone(), page: view.current,
            rotation: view.rotation,
            width: (1200.0 * view.zoom).round().clamp(320.0, 3200.0) as u32,
            wake: Arc::clone(wake),
        };
        if self.jobs.send(job).is_err() {
            view.loading = false;
            view.error = Some("PDF render queue was closed".into());
        }
    }
    pub(super) fn act(&mut self, tab: Uuid, action: pdf::Action, wake: &std::sync::Arc<dyn Fn() + Send + Sync>) {
        let Some(view) = self.views.get_mut(&tab) else { return };
        let mut reload = false;
        match action {
            pdf::Action::Step(delta) => {
                let next = (view.current as i64 + i64::from(delta)).clamp(0, view.total.saturating_sub(1) as i64) as usize;
                if next != view.current { view.current = next; reload = true; }
            }
            pdf::Action::First => { if view.current != 0 { view.current = 0; reload = true; } }
            pdf::Action::Last => {
                let last = view.total.saturating_sub(1);
                if last != view.current { view.current = last; reload = true; }
            }
            pdf::Action::Zoom(delta) => {
                let zoom = (view.zoom * if delta > 0 { 1.25 } else { 0.8 }).clamp(0.35, 2.5);
                if (zoom - view.zoom).abs() > 0.001 { view.zoom = zoom; reload = true; }
            }
            pdf::Action::Rotate => { view.rotation = (view.rotation + 1) % 4; reload = true; }
            pdf::Action::Scroll(delta) => {
                let new_scroll = (view.scroll + delta).max(0.0);
                if delta > 0.0 && new_scroll == view.scroll + delta && view.scroll > 2000.0 {
                    let last = view.total.saturating_sub(1);
                    if view.current < last { view.current += 1; reload = true; }
                } else if delta < 0.0 && view.scroll == 0.0 && view.current > 0 {
                    view.current -= 1; reload = true;
                } else { view.scroll = new_scroll.min(100_000.0); }
            }
            pdf::Action::Retry => { reload = true; }
        }
        if reload {
            view.image = None; view.scroll = 0.0; view.error = None;
            self.request(tab, wake);
        }
    }
    pub(super) fn poll(&mut self, active: Uuid) -> bool {
        let mut updated = false;
        while let Ok(done) = self.rx.try_recv() {
            let Some(view) = self.views.get_mut(&done.tab) else { continue };
            if view.generation != done.generation { continue; }
            view.loading = false;
            if done.tab != active {
                // No hidden, already-decoded PDF bitmaps accumulating in RAM.
                view.image = None;
                continue;
            }
            match done.image {
                Ok((png, total)) => {
                    view.total = total;
                    if let Some(image) = crate::images::decode(&png) {
                        view.image = Some(image);
                    } else { view.error = Some("Не удалось декодировать страницу PDF".into()); }
                }
                Err(error) => view.error = Some(error),
            }
            updated |= done.tab == active;
        }
        updated
    }
}

fn render(
    path: &Path, page: usize, width: u32, rotation: i32,
    desired: &AtomicU64, generation: u64,
) -> Result<(Vec<u8>, usize), String> {
    let data_dir = std::env::temp_dir().join(format!("akula-pdf-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&data_dir).map_err(|e| e.to_string())?;
    let png = data_dir.join("page.png");
    let state = data_dir.join("status.txt");
    let result = run_worker(path, page, width, rotation, &png, &state, desired, generation);
    let _ = std::fs::remove_dir_all(&data_dir);
    result
}
fn run_worker(
    path: &Path, page: usize, width: u32, rotation: i32, png: &Path,
    status: &Path, desired: &AtomicU64, generation: u64,
) -> Result<(Vec<u8>, usize), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(exe)
        .arg("--pdf-worker").arg(path).arg(page.to_string()).arg(width.to_string())
        .arg(rotation.to_string()).arg(png).arg(status)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let result = loop {
        if superseded(desired, generation) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("PDF render superseded by another request".into());
        }
        match child.try_wait() {
            Ok(Some(exit)) => break exit.success(),
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(25)),
            _ => { let _ = child.kill(); let _ = child.wait(); break false; }
        }
    };
    let text = std::fs::read_to_string(status).unwrap_or_default();
    if !result { return Err(if text.starts_with("ERROR ") { text } else { "PDF worker failed or timed out".into() }); }
    let mut parts = text.split_whitespace();
    if parts.next() != Some("PDF_VIEW_V1") { return Err("Invalid PDF worker status".into()); }
    let total: usize = parts.next().ok_or("PDF page count missing")?.parse().map_err(|_| "Bad page count")?;
    let w: u32 = parts.next().ok_or("PDF width missing")?.parse().map_err(|_| "Bad width")?;
    let h: u32 = parts.next().ok_or("PDF height missing")?.parse().map_err(|_| "Bad height")?;
    if !(1..=100_000).contains(&total) || u64::from(w) * u64::from(h) > 16_000_000 {
        return Err("Invalid PDF image limits".into());
    }
    let info = std::fs::metadata(png).map_err(|e| e.to_string())?;
    if info.len() > MAX_PNG { return Err("PDF page raster is too large".into()); }
    let bytes = std::fs::read(png).map_err(|e| e.to_string())?;
    Ok((bytes, total))
}

impl App {
    pub(super) fn show_pdf_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        use crate::ui::widgets::Slot;
        self.note = None; self.note_ids = None; self.embeds = None;
        self.open_error = None; self.open_error_ids = None;
        let id = self.tabs.active_id();
        if self.tree.root().is_some_and(|root| trusted_source(root, path)) {
            self.pdf.open(id, path, &self.workspace.wake);
            if let Some(display) = self.pdf.display(id) {
                if let Some(chrome) = &self.chrome {
                    chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, pdf::PdfPage::new(display).erased()));
                }
            }
        } else {
            self.show_open_error(root, path, "PDF недоступен: файл должен быть обычным документом внутри открытой базы");
        }
    }
    pub(super) fn on_pdf_action(&mut self, root: &mut RenderRoot, action: &ErasedAction) -> bool {
        let Some(action) = action.downcast_ref::<pdf::Action>() else { return false };
        if !self.tabs.active().path.as_deref().is_some_and(is_pdf) { return false; }
        let id = self.tabs.active_id();
        self.pdf.act(id, *action, &self.workspace.wake);
        if let Some(path) = self.tabs.active().path.clone() {
            self.show_pdf_readonly(root, &path);
        }
        true
    }
    pub(super) fn poll_pdf(&mut self, root: &mut RenderRoot) {
        if self.pdf.poll(self.tabs.active_id())
            && let Some(path) = self.tabs.active().path.clone().filter(|p| is_pdf(p))
        { self.show_pdf_readonly(root, &path); }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_suffix_is_case_insensitive_and_not_arbitrary_file() {
        assert!(is_pdf(Path::new("x.PDF")));
        assert!(!is_pdf(Path::new("x.pdf.exe")));
    }
    #[test]
    fn only_latest_page_generation_is_eligible_for_delivery() {
        let current = AtomicU64::new(8);
        assert!(!superseded(&current, 8));
        current.store(9, Ordering::Release);
        assert!(superseded(&current, 8));
        assert!(!superseded(&current, 9));
    }

    #[test]
    fn reusing_tab_for_new_pdf_clears_stale_page_and_pending_render() {
        let mut view = View {
            path: PathBuf::from("old.pdf"), current: 8, total: 15,
            zoom: 2.0, rotation: 3, scroll: 700.0,
            image: None, loading: true, error: Some("old error".into()), generation: 42,
        };
        assert!(view.retarget(Path::new("new.pdf")));
        assert_eq!(view.path, PathBuf::from("new.pdf"));
        assert_eq!(view.current, 0);
        assert_eq!(view.total, 1);
        assert_eq!(view.zoom, 1.0);
        assert_eq!(view.rotation, 0);
        assert_eq!(view.scroll, 0.0);
        assert!(!view.loading, "old worker must not block next PDF request");
        assert!(view.image.is_none());
        assert!(view.error.is_none());
        assert!(!view.retarget(Path::new("new.pdf")), "same PDF keeps tab state");
    }
}