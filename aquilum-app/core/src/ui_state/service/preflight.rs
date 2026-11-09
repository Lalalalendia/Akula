//! Read-only admission for a future workspace-folder rename.
//! Planning is NOT a filesystem mutation lease, exclusive owner lock, sync HOLD,
//! power-loss journal, or authorization to call std::fs::rename.
use super::*;
use crate::search::paths::identity;
use std::fs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceRenamePlan {
    pub workspace_id: Uuid,
    pub expected_old_path: PathBuf,
    pub destination_path: PathBuf,
}

fn deny(reason: &str) -> UiStateError {
    UiStateError::InvalidInput { message: reason.to_owned() }
}

fn valid_name(name: &str) -> bool {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 240
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return false;
    }
    // Cross-platform restriction: a vault created on Linux must not become
    // a Windows device name after synchronization or export.
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return false;
    }
    if stem.len() == 4 {
        let (prefix, number) = stem.split_at(3);
        if (prefix == "COM" || prefix == "LPT") && ('1'..='9').contains(&number.chars().next().unwrap_or('0')) {
            return false;
        }
    }
    true
}

fn check_real_directory(path: &Path) -> Result<(), UiStateError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(deny("workspace or parent is not a plain directory"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return Err(deny("reparse-point directory is not eligible"));
        }
    }
    Ok(())
}

fn is_inside(child: &Path, parent: &Path) -> bool {
    let child = identity(child);
    let parent = identity(parent);
    child == parent || child.starts_with(&(parent.trim_end_matches('/').to_owned() + "/"))
}

impl UiStateService {
    /// A conservative *read-only* plan. The future executor must recheck every
    /// precondition under a cross-process filesystem custody protocol before
    /// any rename; no filesystem write is performed here.
    pub fn plan_inactive_workspace_rename(
        &self,
        workspace_id: Uuid,
        proposed_name: &str,
        active_workspace: Option<&Path>,
    ) -> Result<WorkspaceRenamePlan, UiStateError> {
        if !valid_name(proposed_name) {
            return Err(deny("invalid or reserved workspace folder name"));
        }
        let all = self.list_workspaces()?;
        let selected = all.iter().find(|w| w.id == workspace_id)
            .ok_or_else(|| deny("workspace ID is no longer registered"))?;
        let source = PathBuf::from(&selected.path);
        check_real_directory(&source)?;
        let canonical = fs::canonicalize(&source)?;
        if identity(&source) != identity(&canonical) {
            return Err(deny("workspace root is not a stable canonical path"));
        }
        let parent = source.parent().ok_or_else(|| deny("cannot rename a volume root"))?;
        check_real_directory(parent)?;
        if identity(&fs::canonicalize(parent)?) != identity(parent) {
            return Err(deny("workspace parent resolves through an alias"));
        }
        if let Some(active) = active_workspace {
            let active = fs::canonicalize(active)?;
            if is_inside(&source, &active) || is_inside(&active, &source) {
                return Err(deny("active workspace must be closed before planning"));
            }
        }
        let destination = parent.join(proposed_name);
        if identity(&source) == identity(&destination) {
            return Err(deny("workspace name does not change its path"));
        }
        // Check existing files, directories and links. Any error other than
        // ENOENT is a denial (including access denied and sharing errors).
        match fs::symlink_metadata(&destination) {
            Ok(_) => return Err(deny("destination already exists")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(UiStateError::from(err)),
        }
        for other in &all {
            if other.id == workspace_id {
                continue;
            }
            let other_path = fs::canonicalize(&other.path)?;
            // A registered descendant or ancestor makes whole-root moves
            // ambiguous for File Gate, history ownership and sync enrollment.
            if is_inside(&source, &other_path)
                || is_inside(&other_path, &source)
                || is_inside(&destination, &other_path)
                || is_inside(&other_path, &destination)
            {
                return Err(deny("nested or overlapping registered workspace"));
            }
        }
        Ok(WorkspaceRenamePlan {
            workspace_id,
            expected_old_path: source,
            destination_path: destination,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, UiStateService, Uuid, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Knowledge");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep.md"), "original").unwrap();
        let service = UiStateService::open(&temp.path().join("state.sqlite"));
        let uuid = service.resolve_workspace(&root.to_string_lossy(), 1).unwrap();
        (temp, service, uuid, root)
    }

    #[test]
    fn admissible_inactive_rename_is_read_only() {
        let (_temp, service, uuid, root) = setup();
        let plan = service.plan_inactive_workspace_rename(uuid, "Renamed", None).unwrap();
        assert_eq!(plan.workspace_id, uuid);
        assert_eq!(plan.expected_old_path, root);
        assert_eq!(plan.destination_path, root.parent().unwrap().join("Renamed"));
        assert!(!plan.destination_path.exists());
        assert_eq!(fs::read_to_string(root.join("keep.md")).unwrap(), "original");
        let known = service.list_workspaces().unwrap();
        assert_eq!(known[0].id, uuid);
        assert_eq!(known[0].path, root.to_string_lossy());
    }

    #[test]
    fn active_and_unsafe_names_are_denied() {
        let (_temp, service, uuid, root) = setup();
        assert!(service.plan_inactive_workspace_rename(uuid, "New", Some(&root)).is_err());
        for name in ["", " ", ".", "..", "../escape", "a/b", r"a\b", "a:", "CON", "Con.txt", "NUL", "LPT1", "COM9.txt", "trailing.", "bad ", "bad\u{0000}"] {
            assert!(service.plan_inactive_workspace_rename(uuid, name, None).is_err(), "unsafe {name}");
        }
        assert!(service.plan_inactive_workspace_rename(uuid, "Knowledge", None).is_err());
        assert!(root.is_dir());
    }

    #[test]
    fn destination_and_nested_registration_are_denied() {
        let (_temp, service, uuid, root) = setup();
        let target = root.parent().unwrap().join("Existing");
        fs::write(&target, "do not overwrite").unwrap();
        assert!(service.plan_inactive_workspace_rename(uuid, "Existing", None).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "do not overwrite");
        let nested = root.join("Nested");
        fs::create_dir(&nested).unwrap();
        service.resolve_workspace(&nested.to_string_lossy(), 2).unwrap();
        assert!(service.plan_inactive_workspace_rename(uuid, "Renamed", None).is_err());
        assert!(root.is_dir());
        assert!(!root.parent().unwrap().join("Renamed").exists());
    }

    #[test]
    fn unregistered_and_missing_workspaces_are_denied() {
        let (_temp, service, uuid, root) = setup();
        assert!(service.plan_inactive_workspace_rename(Uuid::new_v4(), "Other", None).is_err());
        fs::remove_file(root.join("keep.md")).unwrap();
        fs::remove_dir(&root).unwrap();
        assert!(service.plan_inactive_workspace_rename(uuid, "Other", None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_workspace_source_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;
        let (temp, service, _uuid, root) = setup();
        let linked = temp.path().join("Linked");
        symlink(&root, &linked).unwrap();
        let uuid = service.resolve_workspace(&linked.to_string_lossy(), 2).unwrap();
        assert!(service.plan_inactive_workspace_rename(uuid, "LinkedNew", None).is_err());
        assert!(root.join("keep.md").is_file());
        assert!(!temp.path().join("LinkedNew").exists());
    }
}
