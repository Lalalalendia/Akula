//! Experimental, strictly local and read-only DICOM preview.
//! Never expose this as a diagnostic or clinically approved application.
//! The normal PNG/JPEG image pipeline is deliberately kept separate.
use dicom_core::Tag;
use dicom_object::{open_file, OpenFileOptions};
use dicom_pixeldata::PixelDecoder;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "dicom/worker.rs"]
mod worker;
pub use worker::{DicomWorker, FrameUpdate, Navigation};
#[path = "dicom/archive.rs"]
mod archive;

const PIXEL_DATA: Tag = Tag(0x7fe0, 0x0010);
const DIRECTORY_RECORD_SEQUENCE: Tag = Tag(0x0004, 0x1220);
const DIRECTORY_RECORD_TYPE: Tag = Tag(0x0004, 0x1430);
const REFERENCED_FILE_ID: Tag = Tag(0x0004, 0x1500);
const SERIES_INSTANCE_UID: Tag = Tag(0x0020, 0x000e);
const INSTANCE_NUMBER: Tag = Tag(0x0020, 0x0013);
const NUMBER_OF_FRAMES: Tag = Tag(0x0028, 0x0008);
const ROWS: Tag = Tag(0x0028, 0x0010);
const COLUMNS: Tag = Tag(0x0028, 0x0011);
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 10_000;
const MAX_DIRECTORY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 4_194_304;
const MAX_REFERENCES: usize = 512;
const MAX_TOTAL_FRAMES: usize = 4096;

pub fn is_candidate(path: &Path) -> bool {
    if archive::is_zip(path) { return true; }
    if path.is_dir() { return path.join("DICOMDIR").is_file(); }
    if path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("DICOMDIR")) {
        return true;
    }
    if path.extension().and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dcm") || e.eq_ignore_ascii_case("dicom")) {
        return true;
    }
    // DICOM CD records normally have no extension. Probe only extensionless
    // files, never arbitrary named file types.
    if path.extension().is_some() { return false; }
    let Ok(mut file) = File::open(path) else { return false };
    let mut header = [0u8; 132];
    file.read_exact(&mut header).is_ok() && &header[128..132] == b"DICM"
}

/// Trust boundary for files already brought into the open Akula vault.
pub fn admit(vault: &Path, source: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(vault).map_err(|e| e.to_string())?;
    let meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("DICOM: символические ссылки не разрешены".into());
    }
    if !meta.is_file() && !meta.is_dir() {
        return Err("DICOM: необходим обычный файл или каталог".into());
    }
    if meta.is_file() {
        let max = if is_dicomdir(source) { MAX_DIRECTORY_BYTES } else if archive::is_zip(source) { MAX_ARCHIVE_BYTES } else { MAX_SOURCE_BYTES };
        if meta.len() < 132 || meta.len() > max {
            return Err("DICOM: файл пустой или превышает лимит предварительного просмотра".into());
        }
    }
    let actual = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !actual.starts_with(&root) {
        return Err("DICOM: файл должен находиться внутри открытой базы Akula".into());
    }
    Ok(actual)
}

fn is_dicomdir(path: &Path) -> bool {
    path.file_name().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("DICOMDIR"))
}

fn safe_file_id(root: &Path, values: &[String]) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    let mut depth = 0;
    for value in values {
        for part in value.split('\\') {
            let part = part.trim_matches(|c: char| c == ' ' || c == '\0');
            if part.is_empty() || part.len() > 8 ||
                !part.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_') {
                return Err("DICOMDIR: небезопасный идентификатор файла".into());
            }
            path.push(part);
            depth += 1;
        }
    }
    if depth == 0 || depth > 8 {
        return Err("DICOMDIR: неверная вложенность".into());
    }
    Ok(path)
}

fn file_set(vault: &Path, index: &Path) -> Result<Vec<PathBuf>, String> {
    let index = admit(vault, index)?;
    let parent = index.parent().ok_or("DICOMDIR: нет каталога")?;
    let obj = open_file(&index).map_err(|e| format!("DICOMDIR: {e}"))?;
    let rows = obj.element(DIRECTORY_RECORD_SEQUENCE).map_err(|e| format!("DICOMDIR: {e}"))?;
    let records = rows.items().ok_or("DICOMDIR: отсутствует список записей")?;
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for record in records {
        let kind = record.element(DIRECTORY_RECORD_TYPE).map_err(|e| format!("DICOMDIR: {e}"))?
            .to_str().map_err(|e| format!("DICOMDIR: {e}"))?;
        if kind.trim() != "IMAGE" { continue; }
        if paths.len() >= MAX_REFERENCES { return Err("DICOMDIR: слишком много изображений для превью".into()); }
        let ids = record.element(REFERENCED_FILE_ID)
            .map_err(|e| format!("DICOMDIR: {e}"))?
            .to_multi_str().map_err(|e| format!("DICOMDIR: {e}"))?;
        let candidate = safe_file_id(parent, ids.as_ref())?;
        let checked = admit(vault, &candidate)?;
        if !checked.starts_with(parent) { return Err("DICOMDIR: ссылка вне носителя".into()); }
        if seen.insert(checked.clone()) { paths.push(checked); }
    }
    if paths.is_empty() { return Err("DICOMDIR: нет ссылок IMAGE".into()); }
    Ok(paths)
}

/// A directory is a valid source even when it has no DICOMDIR. Recursively
/// enumerate bounded candidate files, without following symlinks or writing.
fn scan_folder(vault: &Path, folder: &Path) -> Result<Vec<PathBuf>, String> {
    let mut stack = vec![(folder.to_path_buf(), 0_usize)];
    let mut result = Vec::new();
    let mut visited = 0_usize;
    while let Some((directory, depth)) = stack.pop() {
        // Check scope before enumerating Windows junctions and reparse points.
        admit(vault, &directory)?;
        if depth > 8 { return Err("DICOM: слишком глубокое дерево папок".into()); }
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            visited += 1;
            if visited > MAX_DIRECTORY_ENTRIES { return Err("DICOM: слишком много файлов в папке".into()); }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() { continue; }
            let path = entry.path();
            if kind.is_dir() {
                stack.push((path, depth + 1));
            } else if kind.is_file() && is_dicom_image(&path) {
                result.push(admit(vault, &path)?);
                if result.len() > MAX_REFERENCES { return Err("DICOM: больше 512 изображений в папке".into()); }
            }
        }
    }
    result.sort();
    if result.is_empty() { return Err("DICOM: в папке нет изображений".into()); }
    Ok(result)
}

fn is_dicom_image(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else { return false; };
    let mut header = [0u8; 132];
    file.read_exact(&mut header).is_ok() && &header[128..132] == b"DICM"
}

#[derive(Clone, Debug)]
struct FrameRef {
    path: PathBuf,
    member: Option<String>,
    frame: u32,
    order: i32,
}

#[derive(Clone, Debug)]
struct Series {
    frames: Vec<FrameRef>,
}

#[derive(Clone, Debug)]
pub struct Study {
    root: PathBuf,
    series: Vec<Series>,
    selected_series: usize,
    selected_frame: usize,
}

pub struct RenderedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn read_frames(path: PathBuf, member: Option<String>) -> Result<(String, Vec<FrameRef>), String> {
    let obj = if let Some(name) = &member {
        archive::read_metadata(&path, name)?
    } else {
        OpenFileOptions::new().read_until(PIXEL_DATA)
            .open_file(&path).map_err(|e| format!("DICOM: не удалось прочесть заголовок: {e}"))?
    };
    let read_u16 = |tag: Tag| obj.element(tag).ok().and_then(|e| e.to_int::<u16>().ok());
    let rows = read_u16(ROWS).ok_or("DICOM: отсутствует Rows")?;
    let cols = read_u16(COLUMNS).ok_or("DICOM: отсутствует Columns")?;
    let pixels = u64::from(rows) * u64::from(cols);
    if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
        return Err("DICOM: размеры изображения превышают лимит превью".into());
    }
    let number = obj.element(NUMBER_OF_FRAMES).ok()
        .and_then(|e| e.to_str().ok()).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1);
    if number == 0 || number as usize > MAX_TOTAL_FRAMES {
        return Err("DICOM: слишком много кадров".into());
    }
    let series = obj.element(SERIES_INSTANCE_UID).ok()
        .and_then(|e| e.to_str().ok()).map_or_else(|| "single".to_string(), |v| v.trim().to_string());
    let order = obj.element(INSTANCE_NUMBER).ok()
        .and_then(|e| e.to_str().ok()).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
    let frames = (0..number).map(|frame| FrameRef { path: path.clone(), member: member.clone(), frame, order }).collect();
    Ok((series, frames))
}

impl Study {
    pub fn open(vault: &Path, source: &Path) -> Result<Self, String> {
        let source = admit(vault, source)?;
        // User-selected folders, ZIP archives and DICOMDIR trees are accessed
        // in-place. No extraction, copying or import is performed.
        let files: Vec<(PathBuf, Option<String>)> = if source.is_dir() {
            let index = source.join("DICOMDIR");
            if index.is_file() {
                file_set(vault, &index)?.into_iter().map(|p| (p, None)).collect()
            } else {
                scan_folder(vault, &source)?.into_iter().map(|p| (p, None)).collect()
            }
        } else if is_dicomdir(&source) {
            file_set(vault, &source)?.into_iter().map(|p| (p, None)).collect()
        } else if archive::is_zip(&source) {
            archive::candidates(&source)?.into_iter()
                .map(|member| (source.clone(), Some(member))).collect()
        } else {
            vec![(source, None)]
        };
        let mut groups: BTreeMap<String, Vec<FrameRef>> = BTreeMap::new();
        let mut total = 0usize;
        for (file, member) in files {
            let (key, mut frames) = read_frames(file, member)?;
            total = total.checked_add(frames.len()).ok_or("DICOM: переполнение счётчика кадров")?;
            if total > MAX_TOTAL_FRAMES { return Err("DICOM: исследование превышает лимит кадров".into()); }
            groups.entry(key).or_default().append(&mut frames);
        }
        let mut series = Vec::new();
        for (_, mut frames) in groups {
            frames.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.path.cmp(&b.path)).then(a.frame.cmp(&b.frame)));
            series.push(Series { frames });
        }
        if series.is_empty() { return Err("DICOM: исследование пустое".into()); }
        Ok(Self { root: fs::canonicalize(vault).map_err(|e| e.to_string())?, series, selected_series: 0, selected_frame: 0 })
    }

    pub fn advance_frame(&mut self, delta: i32) -> bool {
        let len = self.series[self.selected_series].frames.len();
        let next = if delta < 0 { self.selected_frame.checked_sub(1) }
                   else { self.selected_frame.checked_add(1).filter(|v| *v < len) };
        let Some(next) = next else { return false };
        self.selected_frame = next;
        true
    }

    pub fn advance_series(&mut self, delta: i32) -> bool {
        let next = if delta < 0 { self.selected_series.checked_sub(1) }
                   else { self.selected_series.checked_add(1).filter(|v| *v < self.series.len()) };
        let Some(next) = next else { return false };
        self.selected_series = next;
        self.selected_frame = 0;
        true
    }

    pub fn label(&self) -> String {
        format!("DICOM · серия {}/{}, срез {}/{} · ←/→ срез, PageUp/PageDown серия · только чтение",
            self.selected_series + 1, self.series.len(), self.selected_frame + 1,
            self.series[self.selected_series].frames.len())
    }

    pub fn render(&self) -> Result<RenderedFrame, String> {
        let selected = &self.series[self.selected_series].frames[self.selected_frame];
        // Re-validate every file before opening: a previously admitted path
        // may change between reading the DICOMDIR and decoding the frame.
        // Recheck scope on every frame, including after removable media changes.
        admit(&self.root, &selected.path)?;
        let obj = if let Some(name) = &selected.member {
            let bytes = archive::read_member(&selected.path, name)?;
            dicom_object::from_reader(bytes.as_slice()).map_err(|e| format!("ZIP/DICOM: {e}"))?
        } else {
            open_file(&selected.path).map_err(|e| format!("DICOM: ошибка чтения: {e}"))?
        };
        let pixels = obj.decode_pixel_data_frame(selected.frame)
            .map_err(|e| format!("DICOM: декодирование кадра: {e}"))?;
        let (width, height) = (pixels.columns(), pixels.rows());
        if width == 0 || height == 0 || u64::from(width)*u64::from(height) > MAX_IMAGE_PIXELS {
            return Err("DICOM: недопустимый размер кадра".into());
        }
        let image = pixels.to_dynamic_image(0)
            .map_err(|e| format!("DICOM: преобразование Window/Level: {e}"))?
            .to_rgba8();
        Ok(RenderedFrame { width, height, rgba: image.into_raw() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_ids_reject_traversal_and_nonstandard_components() {
        let root = Path::new("/tmp/akula");
        assert_eq!(safe_file_id(root, &["PT000000".into(), "ST000000\\IM000001".into()]).unwrap(),
                   root.join("PT000000").join("ST000000").join("IM000001"));
        for bad in [vec!["..".into()], vec!["a".into()], vec!["BAD NAME".into()],
                    vec!["A/BC".into()], vec!["ABCDEFGHI".into()], vec!["".into()]] {
            assert!(safe_file_id(root, &bad).is_err());
        }
    }

    #[test]
    fn cannot_admit_file_outside_vault() {
        let vault = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("x.dcm");
        fs::write(&file, vec![0u8; 132]).unwrap();
        assert!(admit(vault.path(), &file).is_err());
    }

    #[test]
    fn dicom_route_uses_signature_for_extensionless_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("IM000123");
        let mut raw = vec![0u8; 132];
        raw[128..].copy_from_slice(b"DICM");
        fs::write(&path, raw).unwrap();
        assert!(is_candidate(&path));
        assert!(is_candidate(Path::new("report.DCM")));
        assert!(is_candidate(Path::new("DICOMDIR")));
        assert!(!is_candidate(Path::new("report.pdf")));
    }

    #[test]
    fn navigation_is_bounded_and_series_switch_resets_slice() {
        let empty = FrameRef { path: PathBuf::from("x"), member: None, frame: 0, order: 1 };
        let mut study = Study {
            root: PathBuf::from("/tmp/akula"),
            series: vec![Series { frames: vec![empty.clone(), empty.clone()] },
                         Series { frames: vec![empty] }],
            selected_series: 0, selected_frame: 0
        };
        assert!(!study.advance_frame(-1));
        assert!(study.advance_frame(1));
        assert!(!study.advance_frame(1));
        assert!(study.advance_series(1));
        assert_eq!(study.selected_frame, 0);
        assert!(!study.advance_series(1));
    }
}
