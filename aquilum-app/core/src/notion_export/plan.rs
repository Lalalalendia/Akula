//! Metadata-only, fail-closed conversion plan for an inventoried Notion export.
//! This is NOT an importer: no Markdown bytes are staged and no File Gate write
//! authority is created. A ZIP/CSV export does not prove original Notion types.
use super::{EntryKind, InventoryReport, ReferenceKind, Resolution};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct PageCandidate {
    pub source_id: String,
    pub source_path: String,
    pub destination_path: String,
    pub source_blake3: String,
    /// Markdown links, not Notion typed relations or database properties.
    pub outgoing_markdown_page_ids: Vec<String>,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct PlanBlocker {
    pub source_path: String,
    pub code: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ImportPlan {
    pub format: &'static str,
    pub source_format: &'static str,
    pub pages: Vec<PageCandidate>,
    pub blockers: Vec<PlanBlocker>,
    /// Only safe to attempt a later verified read-only content staging step.
    pub ready_for_content_staging: bool,
    /// Never true: an inventory does not authorize or implement publication.
    pub publication_supported: bool,
    pub typed_relations_preserved: bool,
    pub workspace_completeness_proven: bool,
}

fn block(blockers: &mut Vec<PlanBlocker>, path: &str, code: &'static str) {
    blockers.push(PlanBlocker { source_path: path.to_owned(), code });
}

/// Plan stable per-page paths without reading another byte or creating output.
/// In particular, a resolved Markdown link is not a typed Notion relation.
pub fn prepare_plan(report: &InventoryReport) -> ImportPlan {
    let mut pages = Vec::new();
    let mut blockers = Vec::new();
    let mut source_to_page = BTreeMap::new();
    let mut known_page_ids = BTreeSet::new();
    if !report.scanned_all_entries_under_limits {
        block(&mut blockers, "", "incomplete_inventory");
    }
    if report.entries.is_empty() {
        block(&mut blockers, "", "empty_inventory");
    }
    for entry in &report.entries {
        match entry.kind {
            EntryKind::MarkdownPage => {
                let Some(id) = entry.source_id.as_deref() else {
                    block(&mut blockers, &entry.path, "missing_page_uuid");
                    continue;
                };
                let canonical = uuid::Uuid::parse_str(id)
                    .map(|v| v.simple().to_string() == id)
                    .unwrap_or(false);
                if !canonical {
                    block(&mut blockers, &entry.path, "invalid_page_uuid");
                    continue;
                }
                if report.duplicate_source_ids.contains_key(id)
                    || !known_page_ids.insert(id.to_owned())
                {
                    block(&mut blockers, &entry.path, "duplicate_page_uuid");
                    continue;
                }
                source_to_page.insert(entry.path.clone(), pages.len());
                pages.push(PageCandidate {
                    source_id: id.to_owned(),
                    source_path: entry.path.clone(),
                    destination_path: format!("Notion-import/{id}.md"),
                    source_blake3: entry.blake3.clone(),
                    outgoing_markdown_page_ids: Vec::new(),
                });
            }
            EntryKind::CsvDatabaseSnapshot =>
                block(&mut blockers, &entry.path, "csv_has_no_typed_schema"),
            EntryKind::HtmlPageUnparsed =>
                block(&mut blockers, &entry.path, "html_not_parsed"),
            EntryKind::NestedZipUnparsed =>
                block(&mut blockers, &entry.path, "nested_zip_not_parsed"),
            EntryKind::Attachment =>
                block(&mut blockers, &entry.path, "attachment_not_staged"),
        }
    }
    // Duplicate identifiers may belong to different object kinds; never
    // silently assign one source identity to a random destination.
    for paths in report.duplicate_source_ids.values() {
        for path in paths {
            block(&mut blockers, path, "duplicate_source_uuid");
        }
    }
    for link in &report.references {
        let Some(&page_index) = source_to_page.get(&link.source_path) else {
            block(&mut blockers, &link.source_path, "reference_source_not_unique_page");
            continue;
        };
        if link.kind == ReferenceKind::Image {
            block(&mut blockers, &link.source_path, "image_not_staged");
            continue;
        }
        match link.resolution {
            Resolution::External | Resolution::AnchorOnly => {}
            Resolution::Resolved => {
                let Some(id) = link.target_id.as_ref() else {
                    block(&mut blockers, &link.source_path, "resolved_link_missing_uuid");
                    continue;
                };
                if !known_page_ids.contains(id) {
                    block(&mut blockers, &link.source_path, "link_target_not_markdown_page");
                } else {
                    pages[page_index].outgoing_markdown_page_ids.push(id.clone());
                }
            }
            Resolution::MissingFromExport =>
                block(&mut blockers, &link.source_path, "link_target_missing"),
            Resolution::AmbiguousId =>
                block(&mut blockers, &link.source_path, "link_target_ambiguous"),
            Resolution::LocalWithoutId =>
                block(&mut blockers, &link.source_path, "local_link_without_uuid"),
            Resolution::AttachmentNotVerified =>
                block(&mut blockers, &link.source_path, "attachment_link_not_verified"),
            Resolution::Pending =>
                block(&mut blockers, &link.source_path, "unresolved_reference"),
        }
    }
    for page in &mut pages {
        page.outgoing_markdown_page_ids.sort();
        page.outgoing_markdown_page_ids.dedup();
    }
    pages.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    blockers.sort_by(|a, b| (&a.source_path, a.code).cmp(&(&b.source_path, b.code)));
    blockers.dedup();
    let ready_for_content_staging = !pages.is_empty()
        && blockers.is_empty()
        && report.scanned_all_entries_under_limits;
    ImportPlan {
        format: "akula_notion_export_plan_v1",
        source_format: report.format,
        pages,
        blockers,
        ready_for_content_staging,
        publication_supported: false,
        typed_relations_preserved: false,
        workspace_completeness_proven: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{inspect_directory, ScanLimits};
    use std::fs;

    const A: &str = "11111111111111111111111111111111";
    const B: &str = "22222222222222222222222222222222";

    #[test]
    fn repeated_titles_keep_uuid_paths_and_deduplicated_markdown_links() {
        let tmp = tempfile::tempdir().unwrap();
        let a = format!("Same {A}.md");
        let b = format!("Same {B}.md");
        fs::write(tmp.path().join(&a),
            format!("[B](Same%20{B}.md) [again](Same%20{B}.md)")).unwrap();
        fs::write(tmp.path().join(&b), "content").unwrap();
        let report = inspect_directory(tmp.path(), ScanLimits::default()).unwrap();
        let plan = prepare_plan(&report);
        assert!(plan.ready_for_content_staging);
        assert!(!plan.publication_supported);
        assert!(!plan.typed_relations_preserved);
        assert!(!plan.workspace_completeness_proven);
        assert!(plan.blockers.is_empty());
        assert_eq!(plan.pages.len(), 2);
        let first = plan.pages.iter().find(|p| p.source_id == A).unwrap();
        assert_eq!(first.destination_path, format!("Notion-import/{A}.md"));
        assert_eq!(first.outgoing_markdown_page_ids, vec![B.to_owned()]);
        assert_ne!(plan.pages[0].destination_path, plan.pages[1].destination_path);
    }

    #[test]
    fn csv_media_and_unknown_local_links_are_explicitly_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(format!("Page {A}.md")),
            "![image](asset.png)\n[missing](missing.md)").unwrap();
        fs::write(tmp.path().join(format!("Database {B}.csv")), "Name\nA\n").unwrap();
        fs::write(tmp.path().join("asset.png"), b"png").unwrap();
        let report = inspect_directory(tmp.path(), ScanLimits::default()).unwrap();
        let plan = prepare_plan(&report);
        assert!(!plan.ready_for_content_staging);
        let codes: BTreeSet<_> = plan.blockers.iter().map(|b| b.code).collect();
        for code in [
            "csv_has_no_typed_schema", "attachment_not_staged",
            "image_not_staged", "local_link_without_uuid",
        ] {
            assert!(codes.contains(code), "missing explicit blocker {code}");
        }
    }

    #[test]
    fn duplicate_uuid_never_selects_arbitrary_source() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(format!("A {A}.md")), "first").unwrap();
        fs::write(tmp.path().join(format!("B {A}.md")), "second").unwrap();
        let report = inspect_directory(tmp.path(), ScanLimits::default()).unwrap();
        let plan = prepare_plan(&report);
        assert!(plan.pages.is_empty());
        assert!(!plan.ready_for_content_staging);
        assert!(plan.blockers.iter().any(|b| b.code == "duplicate_source_uuid"));
    }

    #[test]
    fn markdown_link_to_csv_is_not_invented_as_typed_relation() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(format!("Page {A}.md")),
            format!("[row](Data%20{B}.csv)")).unwrap();
        fs::write(tmp.path().join(format!("Data {B}.csv")), "Name\nvalue\n").unwrap();
        let report = inspect_directory(tmp.path(), ScanLimits::default()).unwrap();
        let plan = prepare_plan(&report);
        assert!(!plan.ready_for_content_staging);
        assert!(plan.blockers.iter().any(|b| b.code == "link_target_not_markdown_page"));
        assert!(!plan.publication_supported);
    }

    #[test]
    fn empty_export_cannot_claim_readiness() {
        let tmp = tempfile::tempdir().unwrap();
        let report = inspect_directory(tmp.path(), ScanLimits::default()).unwrap();
        let plan = prepare_plan(&report);
        assert!(!plan.ready_for_content_staging);
        assert_eq!(plan.blockers[0].code, "empty_inventory");
    }
}
