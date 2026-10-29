mod tree_open;
use std::io::Read;
use std::path::{Path, PathBuf};

use masonry::app::RenderRoot;
use masonry::kurbo::Point;

use super::App;
use crate::images::{MediaKind, kind};
use crate::ui::editor::{TextEditor, media_line};
use crate::vault_files::{self, Source};

fn natural_width(source: &Source<'_>) -> Option<f64> {
    let (width, _) = match source {
        Source::Path(path) => image::image_dimensions(path).ok()?,
        Source::Bytes(bytes, _) => crate::images::dimensions(bytes)?,
    };
    Some(f64::from(width))
}

// Keep the first standalone-image preview bounded and inside the open vault.
// It is a preflight guard, not a replacement for an OS-level decode sandbox.
const MAX_IMAGE_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;
// An input-byte limit does not bound image::load_from_memory's native-pixel
// allocation before the shown-size thumbnail is created. This is a temporary
// whole-source ceiling until a resource-budgeted decode worker exists.
const MAX_IMAGE_PREVIEW_PIXELS: u64 = 24_000_000;

fn preview_dimensions_allowed((width, height): (u32, u32)) -> bool {
    width > 0 && height > 0
        && u64::from(width).checked_mul(u64::from(height))
            .is_some_and(|pixels| pixels <= MAX_IMAGE_PREVIEW_PIXELS)
}

pub(in crate::app) fn local_image_preview(vault: &Path, source: &Path) -> Option<masonry::peniko::ImageBrush> {
    // The tree filters symlink entries; enforce the boundary again at open time.
    if !std::fs::symlink_metadata(source).ok()?.file_type().is_file() {
        return None;
    }
    let root = std::fs::canonicalize(vault).ok()?;
    let canonical = std::fs::canonicalize(source).ok()?;
    if !canonical.starts_with(&root) {
        return None;
    }
    let file = std::fs::File::open(canonical).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE_PREVIEW_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_PREVIEW_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_IMAGE_PREVIEW_BYTES {
        return None;
    }
    // Check source dimensions *before* decoding. A small, highly compressed
    // GIF/WebP/interlaced PNG can otherwise expand to a huge temporary RGBA
    // buffer in the generic fallback, despite a bounded compressed input.
    let dimensions = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format().ok()?.into_dimensions().ok()?;
    if !preview_dimensions_allowed(dimensions) {
        return None;
    }
    // This still is NOT an OS memory sandbox or a full decoded-RSS cap.
    crate::images::decode(&bytes)
}

impl App {
    pub(in crate::app) fn image_tab_widget(&self) -> Option<masonry::core::NewWidget<dyn masonry::core::Widget>> {
        let path = self.tabs.active().path.as_deref()?;
        if kind(&path.to_string_lossy()) != Some(MediaKind::Image) {
            return None;
        }
        let image = local_image_preview(self.tree.root()?, path)?;
        let title = path.file_name()?.to_string_lossy();
        Some(crate::ui::viewer::ImagePage::new(image, &title).erased())
    }

    pub fn paste_image(&mut self, root: &mut RenderRoot, png: &[u8]) -> bool {
        if self.note_ids.as_ref().is_none_or(|ids| root.focused_widget() != Some(ids.body.id())) {
            return false;
        }
        self.insert_media(root, &[Source::Bytes(png, "image.png")], None);
        true
    }

    pub fn drop_files(&mut self, root: &mut RenderRoot, paths: &[PathBuf], at: Point) {
        let media: Vec<Source<'_>> = paths.iter().filter(|p| kind(&p.to_string_lossy()).is_some()).map(|p| Source::Path(p)).collect();
        self.insert_media(root, &media, Some(at));
    }

    fn insert_media(&mut self, root: &mut RenderRoot, sources: &[Source<'_>], at: Option<Point>) {
        let (Some(ids), Some(workspace)) = (&self.note_ids, self.tree.root()) else { return };
        let Some(body) = root.get_widget(ids.body.id()) else { return };
        let column = body.ctx().bounding_box().width();
        let editor = body.downcast::<TextEditor>().map(|e| e.inner());
        let Some(offset) = editor.map(|e| at.map_or(e.caret_offset(), |p| e.offset_at(body.ctx().window_transform().inverse(), p))) else { return };
        let lines: Vec<String> = sources
            .iter()
            .filter_map(|source| {
                let video = match source {
                    Source::Path(path) => kind(&path.to_string_lossy()) == Some(MediaKind::Video),
                    Source::Bytes(_, name) => kind(name) == Some(MediaKind::Video),
                };
                let (name, ext) = if video { ("video", "mp4") } else { ("image", "png") };
                let relative = vault_files::import(&self.workspace.core, workspace, *source, name, ext).ok()?;
                let width = if video { Some(column) } else { natural_width(source).map(|w| w.min(column)) };
                Some(media_line(&relative, width))
            })
            .collect();
        if !lines.is_empty() {
            ids.body.edit(root, |mut editor| TextEditor::insert_block(&mut editor, offset, &lines));
        }
    }
}

#[cfg(test)]
mod preview_tests;
