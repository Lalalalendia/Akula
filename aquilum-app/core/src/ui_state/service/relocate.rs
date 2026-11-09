//! UI-state wrapper for the UUID-preserving registry CAS.
//! Filesystem changes must be made by a separately audited coordinator.
use super::*;

impl UiStateService {
    /// Change the stored path **only after** a verified on-disk relocation.
    /// No filesystem operations, new workspace records, or migrations occur.
    pub fn replace_workspace_path_after_move(
        &self,
        workspace_id: Uuid,
        expected_old_path: &str,
        destination_path: &str,
    ) -> Result<(), UiStateError> {
        self.with_database(|database| {
            database.replace_workspace_path_after_move(
                workspace_id, expected_old_path, destination_path,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_service_uses_same_uuid_and_rejects_stale_cas() {
        let temp = tempfile::tempdir().unwrap();
        let service = UiStateService::open(&temp.path().join("state.sqlite"));
        let uuid = service.resolve_workspace("C:/knowledge", 1).unwrap();
        service.replace_workspace_path_after_move(uuid, "C:/knowledge", "C:/new").unwrap();
        let known = service.list_workspaces().unwrap();
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].id, uuid);
        assert_eq!(known[0].path, "C:/new");
        assert!(service.replace_workspace_path_after_move(uuid, "C:/knowledge", "C:/oops").is_err());
    }
}
