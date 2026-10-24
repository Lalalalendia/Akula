//! Read-only, bounded in-memory staging of Markdown from a verified Notion export.
//! No output files, Vault access, credential loading or File Gate publication.
//! Source directories must be immutable isolated copies; hostile concurrent
//! filesystem replacement remains outside this diagnostic's trust model.
use super::{inspect_path, plan::{prepare_plan, ImportPlan}, ScanLimits};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) const MAX_STAGING_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const MAX_STAGING_TEXT_BYTES: u64 = 32 * 1024 * 1024;

pub struct VerifiedMarkdownPage {
    pub source_id: String,
    pub destination_path: String,
    pub source_blake3: String,
    pub markdown: String,
    pub outgoing_markdown_page_ids: Vec<String>,
}

/// Sensitive Markdown is intentionally not Serialize/Debug; only summary is public.
pub struct ReadOnlyStage {
    pub pages: Vec<VerifiedMarkdownPage>,
}
#[derive(Serialize)]
pub struct PageReceipt {
    pub source_id: String,
    pub destination_path: String,
    pub source_blake3: String,
    pub bytes: usize,
    pub outgoing_markdown_page_ids: Vec<String>,
}
#[derive(Serialize)]
pub struct StageReceipt {
    pub format: &'static str,
    pub pages: Vec<PageReceipt>,
    pub verified_page_count: usize,
    pub publication_supported: bool,
    pub typed_relations_preserved: bool,
    pub workspace_completeness_proven: bool,
}

impl ReadOnlyStage {
    /// Does not emit Markdown, access tokens, backlinks' text or source content.
    pub fn receipt(&self) -> StageReceipt {
        StageReceipt {
            format: "akula_notion_verified_readonly_stage_v1",
            pages: self.pages.iter().map(|page| PageReceipt {
                source_id: page.source_id.clone(),
                destination_path: page.destination_path.clone(),
                source_blake3: page.source_blake3.clone(),
                bytes: page.markdown.len(),
                outgoing_markdown_page_ids: page.outgoing_markdown_page_ids.clone(),
            }).collect(),
            verified_page_count: self.pages.len(),
            publication_supported: false,
            typed_relations_preserved: false,
            workspace_completeness_proven: false,
        }
    }
}

/// Read a complete inventory, reject an incomplete plan, then independently
/// verify content and inventory again. No filesystem or application writes.
pub fn stage_read_only(input: &Path, limits: ScanLimits) -> Result<ReadOnlyStage, String> {
    let metadata = fs::symlink_metadata(input)
        .map_err(|_| "cannot inspect source for bounded staging")?;
    if metadata.is_file() && metadata.len() > MAX_STAGING_INPUT_BYTES {
        return Err("ZIP exceeds bounded content-staging input budget".into());
    }
    let first = inspect_path(input, limits)?;
    let plan = prepare_plan(&first);
    if !plan.ready_for_content_staging {
        return Err(format!(
            "Notion export has {} blockers; inspect --plan, no content staged",
            plan.blockers.len()
        ));
    }
    verify_against_inventory(input, limits, &first, &plan)
}

fn staged_directory_text(root: &Path, limits: ScanLimits, plan: &ImportPlan)
    -> Result<BTreeMap<String, String>, String>
{
    let mut text = BTreeMap::new();
    let mut total = 0_u64;
    for candidate in &plan.pages {
        let mut relative = PathBuf::new();
        for part in candidate.source_path.split('/') {
            if part.is_empty() || matches!(part, "." | "..")
                || part.contains('\\') || part.contains(':')
                || part.chars().any(char::is_control)
            {
                return Err("unsafe Markdown source path".into());
            }
            relative.push(part);
        }
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "cannot inspect staged Markdown source")?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("staged Markdown source is not a regular file".into());
        }
        if metadata.len() > limits.max_text_bytes {
            return Err("Markdown source exceeds per-file budget".into());
        }
        total = total.checked_add(metadata.len()).ok_or("staging byte counter overflow")?;
        if total > MAX_STAGING_TEXT_BYTES {
            return Err("verified Markdown staging exceeds memory budget".into());
        }
        let file = File::open(&path).map_err(|_| "cannot open staged Markdown source")?;
        if !file.metadata().map_err(|_| "cannot stat open Markdown handle")?.is_file() {
            return Err("staged Markdown handle is not a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(limits.max_text_bytes.saturating_add(1))
            .read_to_end(&mut bytes).map_err(|_| "cannot read staged Markdown")?;
        if bytes.len() as u64 != metadata.len() {
            return Err("Markdown changed size during readback".into());
        }
        let markdown = String::from_utf8(bytes)
            .map_err(|_| "staged Markdown is not UTF-8")?;
        if blake3::hash(markdown.as_bytes()).to_hex().to_string() != candidate.source_blake3 {
            return Err("Markdown content digest changed after inventory".into());
        }
        if text.insert(candidate.source_path.clone(), markdown).is_some() {
            return Err("duplicate staged Markdown source path".into());
        }
    }
    Ok(text)
}

fn verify_against_inventory(input: &Path, limits: ScanLimits,
    first: &super::InventoryReport, plan: &ImportPlan)
    -> Result<ReadOnlyStage, String>
{
    let metadata = fs::symlink_metadata(input)
        .map_err(|_| "input changed during read-only staging")?;
    if metadata.file_type().is_symlink() {
        return Err("staged source became a symlink".into());
    }
    let (second, mut text) = if metadata.is_dir() {
        let text = staged_directory_text(input, limits, plan)?;
        (inspect_path(input, limits)?, text)
    } else if metadata.is_file() {
        super::zip_archive::inspect_zip_with_text(input, limits)?
    } else {
        return Err("staged input changed type".into());
    };
    if serde_json::to_vec(first).map_err(|_| "cannot encode first inventory")?
        != serde_json::to_vec(&second).map_err(|_| "cannot encode verified inventory")?
    {
        return Err("Notion export inventory changed during content readback".into());
    }
    let mut pages = Vec::with_capacity(plan.pages.len());
    for candidate in &plan.pages {
        let markdown = text.remove(&candidate.source_path)
            .ok_or("verified Markdown source missing from second inventory")?;
        if blake3::hash(markdown.as_bytes()).to_hex().to_string() != candidate.source_blake3 {
            return Err("verified Markdown digest mismatch".into());
        }
        pages.push(VerifiedMarkdownPage {
            source_id: candidate.source_id.clone(),
            destination_path: candidate.destination_path.clone(),
            source_blake3: candidate.source_blake3.clone(),
            markdown,
            outgoing_markdown_page_ids: candidate.outgoing_markdown_page_ids.clone(),
        });
    }
    if !text.is_empty() {
        return Err("unaccounted staged Markdown content".into());
    }
    Ok(ReadOnlyStage { pages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const A: &str = "11111111111111111111111111111111";
    const B: &str = "22222222222222222222222222222222";

    fn sample(root: &Path) -> (PathBuf, PathBuf) {
        let a = root.join(format!("Page {A}.md"));
        let b = root.join(format!("Page {B}.md"));
        fs::write(&a, format!("# Alpha\n[B](Page%20{B}.md)\n")).unwrap();
        fs::write(&b, "\u{feff}# Beta\n").unwrap();
        (a, b)
    }

    #[test]
    fn directory_content_verified_with_uuid_and_raw_bom_without_output_file() {
        let tmp = tempfile::tempdir().unwrap();
        sample(tmp.path());
        let stage = stage_read_only(tmp.path(), ScanLimits::default()).unwrap();
        assert_eq!(stage.pages.len(), 2);
        let page_a = stage.pages.iter().find(|p| p.source_id == A).unwrap();
        assert!(page_a.markdown.starts_with("# Alpha"));
        assert_eq!(page_a.outgoing_markdown_page_ids, vec![B.to_owned()]);
        assert_eq!(page_a.source_blake3, blake3::hash(page_a.markdown.as_bytes()).to_hex().to_string());
        let page_b = stage.pages.iter().find(|p| p.source_id == B).unwrap();
        assert!(page_b.markdown.starts_with('\u{feff}'));
        let receipt = serde_json::to_string(&stage.receipt()).unwrap();
        assert!(!receipt.contains("# Alpha"));
        assert!(!receipt.contains("# Beta"));
        assert!(receipt.contains("\"publication_supported\":false"));
        assert!(!tmp.path().join("Notion-import").exists());
    }

    #[test]
    fn same_length_changes_after_inventory_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, _) = sample(tmp.path());
        let first = inspect_path(tmp.path(), ScanLimits::default()).unwrap();
        let plan = prepare_plan(&first);
        let original = fs::read_to_string(&a).unwrap();
        let changed = original.replace("Alpha", "Omega");
        assert_eq!(original.len(), changed.len());
        fs::write(&a, changed).unwrap();
        assert!(verify_against_inventory(tmp.path(), ScanLimits::default(), &first, &plan)
            .err().expect("changed Markdown must be refused").contains("digest"));
    }

    #[test]
    fn partial_exports_or_unknown_links_cannot_stage_any_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(format!("Page {A}.md")), "[bad](no-uuid.md)").unwrap();
        assert!(stage_read_only(tmp.path(), ScanLimits::default())
            .err().expect("partial export must be refused").contains("blockers"));
        assert!(!tmp.path().join("Notion-import").exists());
    }

    #[test]
    fn aggregate_budget_blocks_large_markdown_in_memory() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(format!("Page {A}.md")),
            "x".repeat((MAX_STAGING_TEXT_BYTES as usize) / 2 + 1)).unwrap();
        fs::write(tmp.path().join(format!("Page {B}.md")),
            "x".repeat((MAX_STAGING_TEXT_BYTES as usize) / 2 + 1)).unwrap();
        assert!(stage_read_only(tmp.path(), ScanLimits {
            max_text_bytes: 20 * 1024 * 1024, ..ScanLimits::default()
        }).err().expect("oversized stage must be refused").contains("memory budget"));
    }
}
