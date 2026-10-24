//! Strict bounded read-only ZIP inspection. ZIP64, descriptors, encryption,
//! non-UTF8 names, symlinks, overlaps and extensions are rejected.
use super::*;
use flate2::read::DeflateDecoder;
use std::io::{Cursor, Read};

fn u16le(a: &[u8], p: usize) -> Result<u16, String> {
    let s = a.get(p..p + 2).ok_or("truncated ZIP u16")?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32le(a: &[u8], p: usize) -> Result<u32, String> {
    let s = a.get(p..p + 4).ok_or("truncated ZIP u32")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn sum(a: usize, b: usize) -> Result<usize, String> {
    a.checked_add(b).ok_or_else(|| "ZIP offset overflow".into())
}
fn safe_name(name: &str) -> Result<String, String> {
    if name.starts_with('/') || name.starts_with('\\') || name.contains('\\')
        || name.contains('\0') || name.contains(':') {
        return Err("unsafe ZIP path".into());
    }
    let mut parts = Vec::new();
    for p in name.trim_end_matches('/').split('/') {
        if p.is_empty() || p == "." || p == ".." || p.chars().any(char::is_control) {
            return Err("unsafe ZIP path component".into());
        }
        parts.push(p);
    }
    if parts.is_empty() { return Err("empty ZIP path".into()) }
    Ok(parts.join("/"))
}
pub(super) fn inspect_zip(path: &Path, limits: ScanLimits) -> Result<InventoryReport, String> {
    inspect_zip_inner(path, limits, false).map(|(report, _)| report)
}

/// Re-parse the entire ZIP under the same strict checks, collecting only Markdown.
/// The returned report is compared against the first inventory before use.
pub(super) fn inspect_zip_with_text(path: &Path, limits: ScanLimits)
    -> Result<(InventoryReport, BTreeMap<String, String>), String>
{
    inspect_zip_inner(path, limits, true)
}

fn inspect_zip_inner(path: &Path, limits: ScanLimits, collect_text: bool)
    -> Result<(InventoryReport, BTreeMap<String, String>), String>
{
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("ZIP must be a regular non-symlink file".into());
    }
    if meta.len() > limits.max_total_bytes {
        return Err("ZIP compressed size exceeds input budget".into());
    }
    if collect_text && meta.len() > super::content::MAX_STAGING_INPUT_BYTES {
        return Err("ZIP exceeds bounded content-staging input budget".into());
    }
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    f.take(limits.max_total_bytes + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != meta.len() {
        return Err("ZIP size changed during scan".into());
    }
    // EOCD must be the final record; don't accidentally accept trailing payloads.
    let scan_start = bytes.len().saturating_sub(65_557);
    let eocd = (scan_start..bytes.len().saturating_sub(21))
        .rev().find(|&p| bytes.get(p..p + 4) == Some(&b"PK\x05\x06"[..])
            && u16le(&bytes, p + 20).is_ok_and(|n| p + 22 + n as usize == bytes.len()))
        .ok_or("ZIP end-of-central-directory missing")?;
    if u16le(&bytes, eocd + 4)? != 0 || u16le(&bytes, eocd + 6)? != 0 {
        return Err("multi-disk ZIP unsupported".into());
    }
    let count = u16le(&bytes, eocd + 10)? as usize;
    if count == u16::MAX as usize || u16le(&bytes, eocd + 8)? as usize != count {
        return Err("ZIP64 or inconsistent entry count unsupported".into());
    }
    if count > limits.max_files { return Err("ZIP exceeds file count limit".into()); }
    let directory_size = u32le(&bytes, eocd + 12)? as usize;
    let directory_at = u32le(&bytes, eocd + 16)? as usize;
    if directory_at == u32::MAX as usize || directory_size == u32::MAX as usize
        || sum(directory_at, directory_size)? != eocd {
        return Err("ZIP64, prefix, or inconsistent directory unsupported".into());
    }
    let mut cursor = directory_at;
    let mut total = 0_u64;
    let mut references = Vec::new();
    let mut entries = Vec::new();
    let mut staged = BTreeMap::new();
    let mut staged_bytes = 0_u64;
    let mut used = Vec::<(usize, usize)>::new();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..count {
        if bytes.get(cursor..cursor + 4) != Some(&b"PK\x01\x02"[..]) {
            return Err("malformed central directory".into());
        }
        let made_by = u16le(&bytes, cursor + 4)?;
        let flags = u16le(&bytes, cursor + 8)?;
        let method = u16le(&bytes, cursor + 10)?;
        let crc = u32le(&bytes, cursor + 16)?;
        let compressed = u32le(&bytes, cursor + 20)? as usize;
        let unpacked = u32le(&bytes, cursor + 24)? as usize;
        let name_len = u16le(&bytes, cursor + 28)? as usize;
        let extra_len = u16le(&bytes, cursor + 30)? as usize;
        let comment_len = u16le(&bytes, cursor + 32)? as usize;
        let external_mode = u32le(&bytes, cursor + 38)?;
        let offset = u32le(&bytes, cursor + 42)? as usize;
        let name_start = sum(cursor, 46)?;
        let name_end = sum(name_start, name_len)?;
        let next = sum(sum(name_end, extra_len)?, comment_len)?;
        if next > sum(directory_at, directory_size)? { return Err("central record overruns directory".into()); }
        if flags & !0x0800 != 0 || !matches!(method, 0 | 8) {
            return Err("unsupported ZIP flags, encryption, descriptor or codec".into());
        }
        if made_by >> 8 == 3 && ((external_mode >> 16) & 0o170000) == 0o120000 {
            return Err("symlinks in ZIP prohibited".into());
        }
        if compressed == u32::MAX as usize || unpacked == u32::MAX as usize
            || offset == u32::MAX as usize || u16le(&bytes, cursor + 34)? != 0 {
            return Err("ZIP64 or multi-disk member prohibited".into());
        }
        if extra_len != 0 { return Err("ZIP extra fields unsupported in strict probe".into()); }
        let raw_name = bytes.get(name_start..name_end).ok_or("ZIP truncated name")?;
        let name = std::str::from_utf8(raw_name).map_err(|_| "non-UTF-8 ZIP filename")?;
        let is_dir = name.ends_with('/');
        let safe = safe_name(name)?;
        if safe.split('/').count() > limits.max_depth {
            return Err("ZIP nesting exceeds limit".into());
        }
        if !seen.insert(safe.to_ascii_lowercase()) { return Err("duplicate/case-colliding ZIP path".into()); }
        if bytes.get(offset..offset + 4) != Some(&b"PK\x03\x04"[..]) {
            return Err("missing local ZIP header".into());
        }
        if u16le(&bytes, offset + 6)? != flags || u16le(&bytes, offset + 8)? != method
            || u32le(&bytes, offset + 14)? != crc
            || u32le(&bytes, offset + 18)? as usize != compressed
            || u32le(&bytes, offset + 22)? as usize != unpacked {
            return Err("central/local header mismatch".into());
        }
        let local_name_len = u16le(&bytes, offset + 26)? as usize;
        if local_name_len != name_len || u16le(&bytes, offset + 28)? != 0 {
            return Err("local header length mismatch".into());
        }
        let local_name_start = sum(offset, 30)?;
        if bytes.get(local_name_start..sum(local_name_start, name_len)?)
            != Some(raw_name) { return Err("local header filename mismatch".into()); }
        let data_at = sum(local_name_start, local_name_len)?;
        let data_end = sum(data_at, compressed)?;
        if data_end > directory_at { return Err("ZIP entry overlaps central directory".into()); }
        if used.iter().any(|(a, b)| offset < *b && data_end > *a) {
            return Err("ZIP compressed entries overlap".into());
        }
        used.push((offset, data_end));
        cursor = next;
        if is_dir {
            if compressed != 0 || unpacked != 0 { return Err("ZIP directory carries data".into()); }
            continue;
        }
        total = total.checked_add(unpacked as u64).ok_or("unpacked size overflow")?;
        if total > limits.max_total_bytes { return Err("unpacked ZIP exceeds total byte budget".into()); }
        let kind = kind_for(&safe);
        // A diagnostic need not allocate multi-GB attachments in memory.
        // Large inputs fail closed until a streaming ZIP reader is reviewed.
        if unpacked as u64 > limits.max_text_bytes {
            return Err("ZIP entry exceeds per-entry memory limit".into());
        }
        let payload = bytes.get(data_at..data_end).ok_or("ZIP truncated payload")?;
        let mut content = Vec::new();
        if method == 0 {
            if payload.len() != unpacked { return Err("stored ZIP length mismatch".into()); }
            content.extend_from_slice(payload);
        } else {
            let mut decoder = DeflateDecoder::new(Cursor::new(payload));
            decoder.take(unpacked as u64 + 1).read_to_end(&mut content)
                .map_err(|_| "ZIP DEFLATE decode failed")?;
        }
        if content.len() != unpacked { return Err("ZIP uncompressed length mismatch".into()); }
        if crc32fast::hash(&content) != crc { return Err("ZIP CRC mismatch".into()); }
        let source_id = trailing_id_from_path(&safe);
        let hash = blake3::hash(&content).to_hex().to_string();
        let csv = if matches!(kind, EntryKind::MarkdownPage | EntryKind::CsvDatabaseSnapshot) {
            let text = std::str::from_utf8(&content).map_err(|_| "non-UTF-8 Notion text")?
                .trim_start_matches('\u{feff}');
            if kind == EntryKind::CsvDatabaseSnapshot { Some(summarize_csv(text)?) }
            else {
                for event in Parser::new_ext(text, Options::all()) {
                    let (kind_ref, href) = match event {
                        Event::Start(Tag::Link { dest_url, .. }) => (ReferenceKind::Link, dest_url),
                        Event::Start(Tag::Image { dest_url, .. }) => (ReferenceKind::Image, dest_url),
                        _ => continue,
                    };
                    if references.len() >= limits.max_references { return Err("ZIP exceeds reference limit".into()); }
                    references.push(Reference { source_path: safe.clone(), source_id: source_id.clone(),
                        href: href.to_string(), kind: kind_ref, target_id: None,
                        resolution: Resolution::Pending, resolved_path: None });
                }
                None
            }
        } else { None };
        if collect_text && kind == EntryKind::MarkdownPage {
            staged_bytes = staged_bytes.checked_add(content.len() as u64)
                .ok_or("staged Markdown byte budget overflow")?;
            if staged_bytes > super::content::MAX_STAGING_TEXT_BYTES {
                return Err("verified Markdown staging exceeds memory budget".into());
            }
            let markdown = String::from_utf8(content)
                .map_err(|_| "non-UTF-8 staged Markdown")?;
            if staged.insert(safe.clone(), markdown).is_some() {
                return Err("duplicate staged Markdown path".into());
            }
        }
        entries.push(InventoryEntry { path: safe, kind, source_id, bytes: unpacked as u64, blake3: hash, csv });
    }
    if cursor != sum(directory_at, directory_size)? { return Err("unaccounted ZIP central records".into()); }
    Ok((finalize_report(entries, references, "notion_zip_inventory_v1"), staged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn put_u16(v: &mut Vec<u8>, n: u16) { v.extend_from_slice(&n.to_le_bytes()); }
    fn put_u32(v: &mut Vec<u8>, n: u32) { v.extend_from_slice(&n.to_le_bytes()); }

    // Small deliberately transparent stored ZIP generator for control fixtures.
    fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for &(name, data) in files {
            let offset = bytes.len() as u32;
            let crc = crc32fast::hash(data);
            bytes.extend_from_slice(b"PK\x03\x04");
            put_u16(&mut bytes, 20);
            put_u16(&mut bytes, 0);
            put_u16(&mut bytes, 0);
            put_u16(&mut bytes, 0);
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, crc);
            put_u32(&mut bytes, data.len() as u32);
            put_u32(&mut bytes, data.len() as u32);
            put_u16(&mut bytes, name.len() as u16);
            put_u16(&mut bytes, 0);
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(data);

            central.extend_from_slice(b"PK\x01\x02");
            put_u16(&mut central, 20);
            put_u16(&mut central, 20);
            put_u16(&mut central, 0);
            put_u16(&mut central, 0);
            put_u16(&mut central, 0);
            put_u16(&mut central, 0);
            put_u32(&mut central, crc);
            put_u32(&mut central, data.len() as u32);
            put_u32(&mut central, data.len() as u32);
            put_u16(&mut central, name.len() as u16);
            put_u16(&mut central, 0);
            put_u16(&mut central, 0);
            put_u16(&mut central, 0);
            put_u16(&mut central, 0);
            put_u32(&mut central, 0);
            put_u32(&mut central, offset);
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = bytes.len() as u32;
        bytes.extend_from_slice(&central);
        bytes.extend_from_slice(b"PK\x05\x06");
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, files.len() as u16);
        put_u16(&mut bytes, files.len() as u16);
        put_u32(&mut bytes, central.len() as u32);
        put_u32(&mut bytes, cd_offset);
        put_u16(&mut bytes, 0);
        bytes
    }

    #[test]
    fn zip_uuid_backlinks_without_extraction() {
        let temp = tempfile::tempdir().unwrap();
        let zip_file = temp.path().join("notion.zip");
        // Derive the encoded link and ZIP entry from the same source UUID.
        // An earlier fixture accidentally omitted two digits from its href.
        let target_id = "22222222222222222222222222222222";
        let target_path = format!("Note {target_id}.md");
        let source_text = format!("[B](Note%20{target_id}.md)");
        let bytes = stored_zip(&[
            ("Note 11111111111111111111111111111111.md", source_text.as_bytes()),
            (target_path.as_str(), b"ok"),
            ("Data 33333333333333333333333333333333.csv",
                b"Title,Notes\r\nA,\"hello,\nworld\"\r\n"),
        ]);
        fs::write(&zip_file, bytes).unwrap();
        let report = inspect_zip(&zip_file, ScanLimits::default()).unwrap();
        assert_eq!(report.entries.len(), 3);
        assert_eq!(report.backlinks.len(), 1, "unresolved ZIP references: {:?}", report.references);
        assert_eq!(report.backlinks[0].target_id, target_id);
        assert_eq!(report.backlinks[0].target_path, target_path);
        assert_eq!(report.references[0].target_id.as_deref(), Some(target_id));
        assert_eq!(report.references[0].resolution, Resolution::Resolved);
        assert_eq!(report.entries.iter().filter(|e| e.csv.is_some()).count(), 1);
        assert_eq!(report.notion_workspace_completeness_verified, false);
    }

    #[test]
    fn zip_rejects_traversal_and_corrupt_crc() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad.zip");
        fs::write(&path, stored_zip(&[("../secret.md", b"x")])).unwrap();
        assert!(inspect_zip(&path, ScanLimits::default()).is_err());
        let mut bytes = stored_zip(&[("safe.md", b"x")]);
        let pos = bytes.windows(1).position(|x| x == b"x").unwrap();
        bytes[pos] ^= 0x20;
        fs::write(&path, bytes).unwrap();
        assert!(inspect_zip(&path, ScanLimits::default()).is_err());
    }

    #[test]
    fn zip_rejects_too_large_aggregate_and_invalid_eocd() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad.zip");
        fs::write(&path, stored_zip(&[("abc.md", b"123456")])).unwrap();
        assert!(inspect_zip(&path, ScanLimits { max_total_bytes: 5, ..ScanLimits::default() }).is_err());
        fs::write(&path, b"not a zip").unwrap();
        assert!(inspect_zip(&path, ScanLimits::default()).is_err());
    }

    #[test]
    fn zip_stage_verifies_raw_markdown_and_never_extracts_to_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let zip_path = temp.path().join("sample.zip");
        let first = "Page 11111111111111111111111111111111.md";
        let second = "Page 22222222222222222222222222222222.md";
        let original = "[B](Page%2022222222222222222222222222222222.md)";
        fs::write(&zip_path, stored_zip(&[(first, original.as_bytes()), (second, b"# Beta")])).unwrap();
        let staged = super::super::content::stage_read_only(
            &zip_path, ScanLimits::default()).unwrap();
        assert_eq!(staged.pages.len(), 2);
        let first_page = staged.pages.iter().find(|page|
            page.source_id == "11111111111111111111111111111111").unwrap();
        assert_eq!(first_page.markdown, original);
        assert_eq!(first_page.outgoing_markdown_page_ids,
            vec!["22222222222222222222222222222222".to_owned()]);
        let receipt = serde_json::to_string(&staged.receipt()).unwrap();
        assert!(!receipt.contains(original));
        assert!(receipt.contains("\"publication_supported\":false"));
        assert!(!temp.path().join("Notion-import").exists());
    }

    #[test]
    fn zip_stage_rejects_crc_corruption_before_any_content_publication() {
        let temp = tempfile::tempdir().unwrap();
        let zip_path = temp.path().join("corrupt.zip");
        let name = "Page 11111111111111111111111111111111.md";
        let mut archive = stored_zip(&[(name, b"sample")]);
        let payload_start = 30 + name.len();
        archive[payload_start] ^= 1;
        fs::write(&zip_path, archive).unwrap();
        assert!(super::super::content::stage_read_only(
            &zip_path, ScanLimits::default()).is_err());
        assert!(!temp.path().join("Notion-import").exists());
    }

    #[test]
    fn deflated_zip_is_verified_without_extraction() {
        let mut compressed = Vec::new();
        {
            let mut encoder = flate2::write::DeflateEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(b"hello").unwrap();
            encoder.finish().unwrap();
        }
        let mut archive = stored_zip(&[("file.md", b"hello")]);
        // Replace the single local payload and central record fields consistently.
        let local_len = 30 + "file.md".len();
        let old_cd_at = local_len + 5;
        archive.splice(local_len..old_cd_at, compressed.iter().copied());
        let cd_at = local_len + compressed.len();
        archive[8..10].copy_from_slice(&8_u16.to_le_bytes());
        archive[18..22].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
        archive[cd_at + 10..cd_at + 12].copy_from_slice(&8_u16.to_le_bytes());
        archive[cd_at + 20..cd_at + 24].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
        let eocd = cd_at + 46 + "file.md".len();
        archive[eocd + 16..eocd + 20].copy_from_slice(&(cd_at as u32).to_le_bytes());
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("deflate.zip");
        fs::write(&path, archive).unwrap();
        let report = inspect_zip(&path, ScanLimits::default()).unwrap();
        assert_eq!(report.entries.len(), 1);
    }
}
