//! Offline, read-only inventory of an ALREADY EXTRACTED Notion Markdown/CSV export.
//! No workspace writes, Notion tokens, network requests, File Gate calls or Sync authority.
//! A successful inventory is NOT proof that Notion exported every accessible object.
use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path};
use walkdir::WalkDir;

#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub max_text_bytes: u64,
    pub max_depth: usize,
    pub max_references: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_files: 100_000,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_text_bytes: 16 * 1024 * 1024,
            max_depth: 64,
            max_references: 1_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    MarkdownPage,
    CsvDatabaseSnapshot,
    HtmlPageUnparsed,
    NestedZipUnparsed,
    Attachment,
}

#[derive(Debug, Serialize)]
pub struct CsvSummary {
    pub columns: Vec<String>,
    pub data_rows: u64,
    pub irregular_rows: u64,
}

#[derive(Debug, Serialize)]
pub struct InventoryEntry {
    pub path: String,
    pub kind: EntryKind,
    pub source_id: Option<String>,
    pub bytes: u64,
    pub blake3: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv: Option<CsvSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Link,
    Image,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Pending,
    Resolved,
    MissingFromExport,
    AmbiguousId,
    External,
    LocalWithoutId,
    AnchorOnly,
    AttachmentNotVerified,
}

#[derive(Debug, Serialize)]
pub struct Reference {
    pub source_path: String,
    pub source_id: Option<String>,
    pub href: String,
    pub kind: ReferenceKind,
    pub target_id: Option<String>,
    pub resolution: Resolution,
    pub resolved_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Backlink {
    pub target_id: String,
    pub target_path: String,
    pub source_path: String,
}

#[derive(Debug, Serialize)]
pub struct InventoryReport {
    pub format: &'static str,
    pub scanned_all_entries_under_limits: bool,
    pub notion_workspace_completeness_verified: bool,
    pub entries: Vec<InventoryEntry>,
    pub references: Vec<Reference>,
    pub backlinks: Vec<Backlink>,
    pub duplicate_source_ids: BTreeMap<String, Vec<String>>,
    pub limitations: Vec<&'static str>,
}

#[path = "notion_export_zip.rs"]
mod zip_archive;

/// Inspect a Notion export in an extracted directory OR a bounded top-level ZIP.
/// This operation never extracts, writes into a vault, or uses network credentials.
pub fn inspect_path(input: &Path, limits: ScanLimits) -> Result<InventoryReport, String> {
    let metadata = fs::symlink_metadata(input)
        .map_err(|e| format!("cannot inspect input: {e}"))?;
    if metadata.file_type().is_symlink() {
        return Err("input path must not be a symbolic link".into());
    }
    if metadata.is_dir() {
        return inspect_directory(input, limits);
    }
    if metadata.is_file() && input.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        return zip_archive::inspect_zip(input, limits);
    }
    Err("input must be an extracted directory or a regular .zip file".into())
}

pub fn inspect_directory(root: &Path, limits: ScanLimits) -> Result<InventoryReport, String> {
    let root_meta = fs::symlink_metadata(root)
        .map_err(|e| format!("cannot inspect input directory: {e}"))?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err("input must be a real directory, not a link or file".into());
    }

    let mut entries = Vec::new();
    let mut references = Vec::new();
    let mut total = 0_u64;

    for next in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = next.map_err(|e| format!("cannot scan export: {e}"))?;
        if entry.depth() > limits.max_depth {
            return Err(format!("export directory depth exceeds {}", limits.max_depth));
        }
        let kind_of_entry = entry.file_type();
        if kind_of_entry.is_symlink() {
            return Err(format!("export contains symlink: {}", entry.path().display()));
        }
        if kind_of_entry.is_dir() {
            continue;
        }
        if !kind_of_entry.is_file() {
            return Err(format!("export contains non-regular entry: {}", entry.path().display()));
        }
        if entries.len() >= limits.max_files {
            return Err(format!("export exceeds {} files", limits.max_files));
        }
        let relative = entry.path().strip_prefix(root)
            .map_err(|_| "entry escaped export root".to_string())?;
        let path = checked_relative(relative)?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("cannot stat {path}: {e}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!("export entry changed type during scan: {path}"));
        }
        total = total.checked_add(metadata.len())
            .ok_or_else(|| "export byte counter overflow".to_string())?;
        if total > limits.max_total_bytes {
            return Err(format!("export exceeds {} bytes", limits.max_total_bytes));
        }

        let kind = kind_for(&path);
        let source_id = match kind {
            EntryKind::MarkdownPage | EntryKind::CsvDatabaseSnapshot | EntryKind::HtmlPageUnparsed =>
                trailing_id_from_path(&path),
            _ => None,
        };
        let (hash, csv) = if matches!(kind, EntryKind::MarkdownPage | EntryKind::CsvDatabaseSnapshot) {
            if metadata.len() > limits.max_text_bytes {
                return Err(format!("text entry exceeds {} bytes: {path}", limits.max_text_bytes));
            }
            let mut bytes = Vec::new();
            File::open(entry.path()).map_err(|e| format!("cannot open {path}: {e}"))?
                .take(limits.max_text_bytes + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            if bytes.len() as u64 != metadata.len() {
                return Err(format!("entry changed size during scan: {path}"));
            }
            let hash = blake3::hash(&bytes).to_hex().to_string();
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| format!("non-UTF-8 text in {path}: {e}"))?
                .trim_start_matches('\u{feff}');
            if kind == EntryKind::MarkdownPage {
                for event in Parser::new_ext(text, Options::all()) {
                    let (reference_kind, href) = match event {
                        Event::Start(Tag::Link { dest_url, .. }) => (ReferenceKind::Link, dest_url),
                        Event::Start(Tag::Image { dest_url, .. }) => (ReferenceKind::Image, dest_url),
                        _ => continue,
                    };
                    if references.len() >= limits.max_references {
                        return Err(format!("more than {} references", limits.max_references));
                    }
                    references.push(Reference {
                        source_path: path.clone(),
                        source_id: source_id.clone(),
                        href: href.to_string(),
                        kind: reference_kind,
                        target_id: None,
                        resolution: Resolution::Pending,
                        resolved_path: None,
                    });
                }
                (hash, None)
            } else {
                (hash, Some(summarize_csv(text)?))
            }
        } else {
            (hash_streaming(entry.path(), metadata.len())?, None)
        };
        entries.push(InventoryEntry {
            path, kind, source_id, bytes: metadata.len(), blake3: hash, csv,
        });
    }
    Ok(finalize_report(entries, references, "notion_extracted_directory_inventory_v1"))
}

fn finalize_report(mut entries: Vec<InventoryEntry>, mut references: Vec<Reference>, format: &'static str) -> InventoryReport {
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    // Pass 1: keep source identities independent of titles and local paths.
    let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in &entries {
        if let Some(id) = &entry.source_id {
            by_id.entry(id.clone()).or_default().push(entry.path.clone());
        }
    }
    let duplicate_source_ids = by_id.iter().filter(|(_, paths)| paths.len() > 1)
        .map(|(id, paths)| (id.clone(), paths.clone())).collect();

    // Pass 2: resolve by Notion UUID, never guess by a non-unique title.
    let mut backlinks = Vec::new();
    for reference in &mut references {
        if reference.kind == ReferenceKind::Image {
            reference.resolution = Resolution::AttachmentNotVerified;
        } else if reference.href.starts_with('#') {
            reference.resolution = Resolution::AnchorOnly;
        } else if is_external_non_notion(&reference.href) {
            reference.resolution = Resolution::External;
        } else {
            reference.target_id = target_id_from_href(&reference.href);
            if let Some(id) = &reference.target_id {
                match by_id.get(id).map(Vec::as_slice) {
                    Some([target]) => {
                        reference.resolution = Resolution::Resolved;
                        reference.resolved_path = Some(target.clone());
                        backlinks.push(Backlink {
                            target_id: id.clone(),
                            target_path: target.clone(),
                            source_path: reference.source_path.clone(),
                        });
                    }
                    Some([]) | None => reference.resolution = Resolution::MissingFromExport,
                    Some(_) => reference.resolution = Resolution::AmbiguousId,
                }
            } else {
                reference.resolution = Resolution::LocalWithoutId;
            }
        }
    }
    references.sort_by(|a, b| (&a.source_path, &a.href).cmp(&(&b.source_path, &b.href)));
    backlinks.sort_by(|a, b| (&a.target_path, &a.source_path).cmp(&(&b.target_path, &b.source_path)));

    InventoryReport {
        format,
        scanned_all_entries_under_limits: true,
        notion_workspace_completeness_verified: false,
        entries,
        references,
        backlinks,
        duplicate_source_ids,
        limitations: vec![
            "Supports directory and top-level ZIP inputs. Nested ZIP data remains unparsed.",
            "HTML content, nested ZIP archives and attachment links are not interpreted.",
            "CSV is a typed-unknown snapshot; relations, formulas, rollups and database views cannot be reconstructed from it.",
            "Only Markdown link/image syntax is scanned, not plain text URLs, rich Notion block IDs or HTML embedded in Markdown.",
            "This is a local graph diagnostic, not an importer, a Notion API reader or a claim of complete source permissions.",
        ],
    }
}

fn checked_relative(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let segment = part.to_str().ok_or("non-UTF-8 filename")?;
                if segment.contains('\\') || segment.is_empty() {
                    return Err("ambiguous filename separator".into());
                }
                parts.push(segment);
            }
            _ => return Err("export contains unsafe relative path component".into()),
        }
    }
    Ok(parts.join("/"))
}

fn kind_for(path: &str) -> EntryKind {
    let ext = path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("md") | Some("markdown") => EntryKind::MarkdownPage,
        Some("csv") => EntryKind::CsvDatabaseSnapshot,
        Some("html") | Some("htm") => EntryKind::HtmlPageUnparsed,
        Some("zip") => EntryKind::NestedZipUnparsed,
        _ => EntryKind::Attachment,
    }
}

fn hash_streaming(path: &Path, expected: u64) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut size = 0_u64;
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let len = file.read(&mut buf).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if len == 0 {
            break;
        }
        size = size.checked_add(len as u64).ok_or("file size counter overflow")?;
        if size > expected {
            return Err(format!("entry grew during scan: {}", path.display()));
        }
        hasher.update(&buf[..len]);
    }
    if size != expected {
        return Err(format!("entry shrank during scan: {}", path.display()));
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn trailing_id_from_path(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?;
    let (stem, ext) = filename.rsplit_once('.')?;
    if !matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "csv" | "html" | "htm") {
        return None;
    }
    trailing_id(stem)
}

fn trailing_id(value: &str) -> Option<String> {
    let value = value.trim_end_matches('/');
    for width in [36_usize, 32] {
        let Some(start) = value.len().checked_sub(width) else { continue };
        let Some(candidate) = value.get(start..) else { continue };
        let bytes = candidate.as_bytes();
        let valid = if width == 32 {
            bytes.iter().all(u8::is_ascii_hexdigit)
        } else {
            bytes.iter().enumerate().all(|(at, byte)| {
                if matches!(at, 8 | 13 | 18 | 23) {
                    *byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
        };
        let separator = start == 0 || value[..start].as_bytes().last()
            .is_some_and(|byte| matches!(byte, b' ' | b'-' | b'_'));
        if valid && separator {
            return Some(candidate.chars().filter(|ch| *ch != '-')
                .collect::<String>().to_ascii_lowercase());
        }
    }
    None
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hi = bytes.get(at + 1)?.to_ascii_lowercase();
            let lo = bytes.get(at + 2)?.to_ascii_lowercase();
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                _ => None,
            };
            decoded.push((hex(hi)? << 4) | hex(lo)?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn target_id_from_href(href: &str) -> Option<String> {
    let path = href.split(['#', '?']).next()?;
    let decoded = percent_decode(path)?;
    let last = decoded.trim_end_matches('/').rsplit('/').next()?;
    let stem = last.rsplit_once('.').and_then(|(stem, ext)| {
        matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "csv" | "html" | "htm")
            .then_some(stem)
    }).unwrap_or(last);
    trailing_id(stem)
}

fn is_external_non_notion(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("mailto:") || lower.starts_with("tel:") || lower.starts_with("data:") {
        return true;
    }
    let Some((scheme, rest)) = lower.split_once("://") else { return false };
    if scheme != "https" && scheme != "http" {
        return true;
    }
    let host = rest.split('/').next().unwrap_or("").split('@').next_back()
        .unwrap_or("").split(':').next().unwrap_or("");
    !["notion.so", "notion.site", "notion.com"].iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CsvState { Start, Unquoted, Quoted, AfterQuote }

#[derive(Default)]
struct CsvCounter {
    columns: Vec<String>,
    header_cells: Vec<String>,
    current_cell: String,
    current_fields: usize,
    data_rows: u64,
    irregular_rows: u64,
    has_header: bool,
}

impl CsvCounter {
    fn push(&mut self, c: char) {
        if !self.has_header {
            self.current_cell.push(c);
        }
    }

    fn finish_field(&mut self) -> Result<(), String> {
        self.current_fields += 1;
        if self.current_fields > 4096 {
            return Err("CSV exceeds 4096 columns".into());
        }
        if !self.has_header {
            self.header_cells.push(std::mem::take(&mut self.current_cell));
        }
        Ok(())
    }

    fn finish_record(&mut self) -> Result<(), String> {
        self.finish_field()?;
        if !self.has_header {
            self.columns = std::mem::take(&mut self.header_cells);
            self.has_header = true;
        } else {
            self.data_rows += 1;
            if self.data_rows > 1_000_000 {
                return Err("CSV exceeds one million records".into());
            }
            if self.current_fields != self.columns.len() {
                self.irregular_rows += 1;
            }
        }
        self.current_fields = 0;
        Ok(())
    }
}

fn summarize_csv(text: &str) -> Result<CsvSummary, String> {
    let mut counter = CsvCounter::default();
    let mut state = CsvState::Start;
    let mut pending = false;
    let mut chars = text.chars().peekable();
    while let Some(original) = chars.next() {
        let ch = if original == '\r' && state != CsvState::Quoted {
            if chars.peek() == Some(&'\n') { chars.next(); }
            '\n'
        } else { original };
        match (state, ch) {
            (CsvState::Start, '"') => { state = CsvState::Quoted; pending = true; }
            (CsvState::Start | CsvState::Unquoted | CsvState::AfterQuote, ',') => {
                counter.finish_field()?;
                state = CsvState::Start;
                pending = true;
            }
            (CsvState::Start | CsvState::Unquoted | CsvState::AfterQuote, '\n') => {
                counter.finish_record()?;
                state = CsvState::Start;
                pending = false;
            }
            (CsvState::Quoted, '"') => { state = CsvState::AfterQuote; }
            (CsvState::AfterQuote, '"') => { counter.push('"'); state = CsvState::Quoted; }
            (CsvState::Start | CsvState::Unquoted, c) => {
                counter.push(c);
                state = CsvState::Unquoted;
                pending = true;
            }
            (CsvState::Quoted, c) => { counter.push(c); pending = true; }
            (CsvState::AfterQuote, _) => return Err("CSV contains trailing characters after quote".into()),
        }
    }
    if state == CsvState::Quoted {
        return Err("CSV has an unterminated quoted field".into());
    }
    if pending {
        counter.finish_record()?;
    }
    Ok(CsvSummary {
        columns: counter.columns,
        data_rows: counter.data_rows,
        irregular_rows: counter.irregular_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const A: &str = "11111111111111111111111111111111";
    const B: &str = "22222222222222222222222222222222";
    const C: &str = "33333333333333333333333333333333";

    #[test]
    fn stable_uuid_graph_and_backlinks_survive_duplicate_titles() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("One")).unwrap();
        fs::create_dir(dir.path().join("Two")).unwrap();
        fs::write(dir.path().join(format!("One/Topic {A}.md")),
            format!("[same title](../Two/Topic%20{B}.md)\n[missing](other%20{C}.md)\n[web](https://example.org)")).unwrap();
        fs::write(dir.path().join(format!("Two/Topic {B}.md")), "text").unwrap();
        let report = inspect_directory(dir.path(), ScanLimits::default()).unwrap();
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.backlinks.len(), 1);
        assert_eq!(report.backlinks[0].target_id, B);
        assert_eq!(report.backlinks[0].source_path, format!("One/Topic {A}.md"));
        assert_eq!(report.references.iter().filter(|r| r.resolution == Resolution::MissingFromExport).count(), 1);
        assert_eq!(report.references.iter().filter(|r| r.resolution == Resolution::External).count(), 1);
        assert!(!report.notion_workspace_completeness_verified);
    }

    #[test]
    fn duplicate_uuid_does_not_choose_a_random_target() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(format!("First {A}.md")), "one").unwrap();
        fs::write(dir.path().join(format!("Another {A}.md")), "two").unwrap();
        fs::write(dir.path().join(format!("Source {B}.md")), format!("[link](First%20{A}.md)")).unwrap();
        let report = inspect_directory(dir.path(), ScanLimits::default()).unwrap();
        assert_eq!(report.duplicate_source_ids[A].len(), 2);
        assert_eq!(report.references[0].resolution, Resolution::AmbiguousId);
        assert!(report.backlinks.is_empty());
    }

    #[test]
    fn csv_with_quoted_comma_newline_and_escaped_quote_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(format!("Tasks {A}.csv")),
            "\"Title\",\"Detail, with comma\"\r\n\"Row\none\",\"Say \"\"hi\"\"\"\r\n").unwrap();
        let report = inspect_directory(dir.path(), ScanLimits::default()).unwrap();
        let csv = report.entries[0].csv.as_ref().unwrap();
        assert_eq!(csv.columns, ["Title", "Detail, with comma"]);
        assert_eq!(csv.data_rows, 1);
        assert_eq!(csv.irregular_rows, 0);
    }

    #[test]
    fn malformed_csv_and_text_limits_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(format!("Data {A}.csv"));
        fs::write(&p, "\"unterminated").unwrap();
        assert!(inspect_directory(dir.path(), ScanLimits::default()).is_err());
        fs::write(&p, "some,text").unwrap();
        assert!(inspect_directory(dir.path(), ScanLimits { max_text_bytes: 3, ..ScanLimits::default() }).is_err());
    }

    #[test]
    fn nested_zip_and_images_are_not_silently_marked_imported() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("inside.zip"), b"example").unwrap();
        fs::write(dir.path().join(format!("Page {A}.md")), "![pic](images/pic.png)").unwrap();
        let report = inspect_directory(dir.path(), ScanLimits::default()).unwrap();
        assert!(report.entries.iter().any(|e| e.kind == EntryKind::NestedZipUnparsed));
        assert_eq!(report.references[0].resolution, Resolution::AttachmentNotVerified);
        assert!(report.backlinks.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cannot_escape_the_read_only_inventory_root() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "private").unwrap();
        symlink(outside.path().join("secret.md"), dir.path().join("entry.md")).unwrap();
        assert!(inspect_directory(dir.path(), ScanLimits::default()).is_err());
    }

    #[test]
    fn filename_id_is_not_inferred_from_an_arbitrary_title() {
        assert_eq!(trailing_id_from_path(&format!("Emoji 😎 {A}.md")), Some(A.into()));
        assert_eq!(trailing_id_from_path(&format!("NoSeparator{A}.md")), None);
        assert_eq!(target_id_from_href(&format!("https://www.notion.so/Notes-{B}#abc")), Some(B.into()));
        assert_eq!(target_id_from_href("relative/plain.md"), None);
    }
}
