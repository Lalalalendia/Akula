//! Explicit, create-only publication of reviewed Notion note candidates.
//! This is not a background sync or a permission to overwrite local edits.
//! The caller must supply a registered workspace and explicit user approval.
//! Every directory/file mutation goes through Core File Gate.
use super::projection::ImportPreview;
use crate::app_core::Core;
use crate::files::gate;
use crate::history::Source;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const IMPORT_DIR: &str = "Notion-import";
const MAX_NOTE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct PublishReport {
    pub created: Vec<String>,
    pub existing_untouched: Vec<String>,
    pub stopped_at: Option<String>,
    pub completed: bool,
}

/// First version: a user-approved one-way import into an explicitly registered
/// local workspace. Existing paths are NEVER overwritten, even on a retry.
/// The report identifies a partially created batch after a File Gate failure.
/// The original Notion snapshot must have been previewed separately.
pub fn publish_approved(
    core: &Core,
    workspace: &Path,
    preview: &ImportPreview,
    approved: bool,
) -> Result<PublishReport, String> {
    if !approved {
        return Err("Notion import needs explicit user approval".into());
    }
    if !preview.publish_allowed || !preview.unresolved.is_empty()
        || preview.candidates.is_empty()
    {
        return Err("incomplete Notion preview cannot be published".into());
    }
    Uuid::parse_str(&preview.source_root).map_err(|_| "invalid source root UUID")?;
    let root = fs::canonicalize(workspace)
        .map_err(|e| format!("cannot resolve selected workspace: {e}"))?;
    if !root.is_dir() {
        return Err("selected workspace is not a directory".into());
    }
    let registered = core.ui_state.workspace_roots()
        .map_err(|e| format!("cannot verify registered workspaces: {e:?}"))?;
    let matches = registered.iter()
        .filter(|known| fs::canonicalize(known).ok().as_deref() == Some(root.as_path()))
        .count();
    if matches != 1 {
        return Err("selected workspace is not one uniquely registered Akula workspace".into());
    }
    // Validate the ENTIRE batch before any mutation. No caller-provided path
    // segment is allowed: names must come from canonical source UUIDs.
    let mut paths = BTreeSet::new();
    let ids: BTreeSet<String> = preview.candidates.iter()
        .map(|item| item.source_id.clone()).collect();
    if preview.candidates.len() != ids.len() {
        return Err("duplicate source identity in import preview".into());
    }
    for item in &preview.candidates {
        let id = Uuid::parse_str(&item.source_id)
            .map_err(|_| "invalid Notion candidate UUID")?.to_string();
        if id != item.source_id {
            return Err("Notion candidate UUID must be canonical".into());
        }
        let page = format!("{IMPORT_DIR}/{id}.md");
        let source = format!("{IMPORT_DIR}/source-{id}.md");
        if item.relative_path != page && item.relative_path != source {
            return Err("Notion import path is not a safe generated UUID path".into());
        }
        if !paths.insert(item.relative_path.as_str()) {
            return Err("duplicate output path in import preview".into());
        }
        if item.markdown.is_empty() || item.markdown.len() > MAX_NOTE_BYTES {
            return Err("Notion note exceeds bounded publication size".into());
        }
        for target in &item.relations {
            let canonical = Uuid::parse_str(target)
                .map_err(|_| "invalid relation target UUID")?.to_string();
            if canonical != *target || !ids.contains(target) {
                return Err("unresolved Notion relation cannot be published".into());
            }
        }
    }

    let folder = root.join(IMPORT_DIR);
    match fs::symlink_metadata(&folder) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("Notion import folder is not a regular directory".into()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            gate::create_folder(core, &folder)
                .map_err(|e| format!("File Gate refused import folder: {e:?}"))?;
        }
        Err(e) => return Err(format!("cannot inspect import folder: {e}")),
    }
    // Refuse a swapped symlink before entering the loop. Cross-process
    // hostile races still require pinned-OS-handle hardening before release.
    if fs::symlink_metadata(&folder).map_err(|e| e.to_string())?
        .file_type().is_symlink()
    {
        return Err("Notion import directory changed to a symlink".into());
    }

    let mut report = PublishReport {
        created: Vec::new(), existing_untouched: Vec::new(),
        stopped_at: None, completed: false,
    };
    for item in &preview.candidates {
        let output = root.join(&item.relative_path);
        // Recheck directory type before every write; do not follow symlinks.
        let folder_state = fs::symlink_metadata(&folder).map_err(|e| e.to_string())?;
        if !folder_state.is_dir() || folder_state.file_type().is_symlink() {
            report.stopped_at = Some("import folder changed type".into());
            return Ok(report);
        }
        match fs::symlink_metadata(&output) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                report.existing_untouched.push(item.relative_path.clone());
                continue;
            }
            Ok(_) => {
                report.stopped_at = Some(format!(
                    "unsafe existing entry: {}",item.relative_path
                ));
                return Ok(report);
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                report.stopped_at = Some(format!("cannot inspect import target: {e}"));
                return Ok(report);
            }
        }
        match gate::create(core, &output, &item.markdown, Source::External, Some("Notion import")) {
            Ok(_) => report.created.push(item.relative_path.clone()),
            Err(e) => {
                // In particular, a concurrent create must not be retried as
                // write/replace. Earlier files are explicitly reported.
                report.stopped_at = Some(format!(
                    "File Gate refused {}: {e:?}",item.relative_path
                ));
                return Ok(report);
            }
        }
    }
    report.completed = true;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use crate::notion_api_probe::projection::ImportCandidate;
    use std::sync::Arc;

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn preview() -> ImportPreview {
        ImportPreview {
            source_root: A.into(),
            candidates: vec![
                ImportCandidate {
                    source_id: A.into(),
                    relative_path: format!("Notion-import/{A}.md"),
                    markdown: format!("# Row A\n\n[Linked](<{B}.md>)\n"),
                    relations: vec![B.into()],
                },
                ImportCandidate {
                    source_id: B.into(),
                    relative_path: format!("Notion-import/{B}.md"),
                    markdown: "# Row B\n".into(),
                    relations: vec![],
                },
            ],
            unresolved: vec![],
            publish_allowed: true,
        }
    }

    #[test]
    fn approved_gate_publication_creates_linked_notes_once_without_overwrite() {
        let temp=tempfile::tempdir().unwrap();
        let workspace=temp.path().join("workspace");
        let app=temp.path().join("app");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&app).unwrap();
        let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
        core.ui_state.resolve_workspace(workspace.to_str().unwrap(),1).unwrap();
        let data=preview();
        assert!(publish_approved(&core,&workspace,&data,false).is_err());
        assert!(!workspace.join(IMPORT_DIR).exists());

        let first=publish_approved(&core,&workspace,&data,true).unwrap();
        assert!(first.completed);
        assert_eq!(first.created.len(),2);
        assert!(fs::read_to_string(workspace.join(format!("Notion-import/{A}.md")))
            .unwrap().contains(&format!("<{B}.md>")));

        let second=publish_approved(&core,&workspace,&data,true).unwrap();
        assert!(second.completed);
        assert!(second.created.is_empty());
        assert_eq!(second.existing_untouched.len(),2);

        let path=workspace.join(format!("Notion-import/{A}.md"));
        gate::write(&core,&path,"user edit",None,Source::Me,None).unwrap();
        let third=publish_approved(&core,&workspace,&data,true).unwrap();
        assert!(third.completed);
        assert_eq!(third.existing_untouched.len(),2);
        assert_eq!(fs::read_to_string(path).unwrap(),"user edit");
    }

    #[test]
    fn invalid_path_and_incomplete_preview_fail_before_creating_folder() {
        let temp=tempfile::tempdir().unwrap();
        let workspace=temp.path().join("workspace");
        let app=temp.path().join("app");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&app).unwrap();
        let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
        core.ui_state.resolve_workspace(workspace.to_str().unwrap(),1).unwrap();
        let mut data=preview();
        data.candidates[1].relative_path="../outside.md".into();
        assert!(publish_approved(&core,&workspace,&data,true).is_err());
        assert!(!workspace.join(IMPORT_DIR).exists());
        data=preview();
        data.publish_allowed=false;
        assert!(publish_approved(&core,&workspace,&data,true).is_err());
        assert!(!workspace.join(IMPORT_DIR).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_import_directory_is_rejected() {
        use std::os::unix::fs::symlink;
        let temp=tempfile::tempdir().unwrap();
        let workspace=temp.path().join("workspace");
        let other=temp.path().join("outside");
        let app=temp.path().join("app");
        for path in [&workspace,&other,&app] {fs::create_dir(path).unwrap();}
        symlink(&other,workspace.join(IMPORT_DIR)).unwrap();
        let core=Core::open(&app,Arc::new(|_:CoreEvent|{}));
        core.ui_state.resolve_workspace(workspace.to_str().unwrap(),1).unwrap();
        assert!(publish_approved(&core,&workspace,&preview(),true).is_err());
        assert_eq!(fs::read_dir(other).unwrap().count(),0);
    }
}
