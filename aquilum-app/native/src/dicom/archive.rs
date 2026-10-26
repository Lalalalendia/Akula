//! Bounded read-only ZIP access: never extracts entries to filesystem.
//! Only STORE and DEFLATE member codecs are enabled in Cargo.
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const MAX_ENTRIES: usize = 10_000;
const MAX_CANDIDATES: usize = 512;
const MAX_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

pub fn is_zip(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("zip"))
}

// ZIP names are logical references, not paths on disk. We still reject
// traversal and absolute names even though nothing gets extracted.
pub fn safe_member(name: &str) -> bool {
    if name.is_empty() || name.len() > 512 || name.contains('\\')
       || name.starts_with('/') || name.contains('\0') { return false; }
    name.split('/').all(|part| !part.is_empty() && part != "." && part != ".."
        && !part.contains(':') && !part.chars().any(|c| c.is_control()))
}

pub fn candidates(path: &Path) -> Result<Vec<String>, String> {
    let f = File::open(path).map_err(|e| format!("ZIP: {e}"))?;
    let mut zip = ZipArchive::new(f).map_err(|e| format!("ZIP: неверный каталог: {e}"))?;
    if zip.len() > MAX_ENTRIES { return Err("ZIP: слишком много файлов".into()); }
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut sum = 0_u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("ZIP: запись {i}: {e}"))?;
        if entry.is_dir() { continue; }
        let name = entry.name().to_owned();
        if !safe_member(&name) {
            return Err(format!("ZIP: небезопасный путь внутри архива: {name:?}"));
        }
        if !seen.insert(name.clone()) {
            return Err("ZIP: повторяющиеся имена файлов".into());
        }
        if name.rsplit('/').next().is_some_and(|s| s.eq_ignore_ascii_case("DICOMDIR")) {
            continue;
        }
        let extension = Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("");
        if !extension.is_empty()
            && !extension.eq_ignore_ascii_case("dcm")
            && !extension.eq_ignore_ascii_case("dicom") { continue; }
        if entry.size() < 132 { continue; }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err("ZIP: DICOM-файл превышает лимит 128 МиБ".into());
        }
        let mut magic = [0_u8; 132];
        entry.read_exact(&mut magic).map_err(|e| format!("ZIP: повреждённая запись: {e}"))?;
        if &magic[128..132] != b"DICM" { continue; }
        sum = sum.checked_add(entry.size()).ok_or("ZIP: переполнение объёма")?;
        if sum > MAX_TOTAL_BYTES { return Err("ZIP: суммарные DICOM-данные превышают 512 МиБ".into()); }
        names.push(name);
        if names.len() > MAX_CANDIDATES { return Err("ZIP: больше 512 изображений".into()); }
    }
    if names.is_empty() { return Err("ZIP: нет изображений DICOM".into()); }
    Ok(names)
}

pub fn read_member(path: &Path, name: &str) -> Result<Vec<u8>, String> {
    if !safe_member(name) { return Err("ZIP: небезопасный путь".into()); }
    let mut zip = ZipArchive::new(File::open(path).map_err(|e| format!("ZIP: {e}"))?)
        .map_err(|e| format!("ZIP: {e}"))?;
    let mut member = zip.by_name(name).map_err(|e| format!("ZIP: нет изображения {name}: {e}"))?;
    if member.size() < 132 || member.size() > MAX_ENTRY_BYTES {
        return Err("ZIP: недопустимый размер изображения".into());
    }
    let expected = member.size();
    let mut bytes = Vec::with_capacity(usize::try_from(expected).map_err(|e| e.to_string())?);
    member.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|e| format!("ZIP: ошибка декодирования: {e}"))?;
    if bytes.len() as u64 != expected { return Err("ZIP: неполное чтение изображения".into()); }
    if &bytes[128..132] != b"DICM" { return Err("ZIP: неверная сигнатура DICOM".into()); }
    Ok(bytes)
}

pub fn read_metadata(path: &Path, name: &str) -> Result<dicom_object::DefaultDicomObject, String> {
    if !safe_member(name) { return Err("ZIP: небезопасный путь".into()); }
    let mut zip = ZipArchive::new(File::open(path).map_err(|e| format!("ZIP: {e}"))?)
        .map_err(|e| format!("ZIP: {e}"))?;
    let member = zip.by_name(name).map_err(|e| format!("ZIP: {e}"))?;
    if member.size() > MAX_ENTRY_BYTES { return Err("ZIP: изображение слишком большое".into()); }
    dicom_object::OpenFileOptions::new().read_until(dicom_core::Tag(0x7fe0, 0x0010))
        .from_reader(member).map_err(|e| format!("ZIP: заголовок DICOM: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_names_are_rejected() {
        for name in ["../evil", "A/../B", "/absolute", "C:\\evil", "bad//file", "x:drive",
                     "ok/./bad", "", "dir\0name"] {
            assert!(!safe_member(name), "{name:?}");
        }
        assert!(safe_member("PATIENT/STUDY/IMAGE01"));
        assert!(safe_member("CT/series/0001.dcm"));
    }
}
