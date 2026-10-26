use super::*;

impl App {
    pub(super) fn preview_surface_available(&self) -> bool {
        #[cfg(feature = "dicom-preview")]
        { is_preview_surface_available(self.tree.root(), self.tabs.active().path.as_deref(), &self.dicom_external_sources) }
        #[cfg(not(feature = "dicom-preview"))]
        { self.tree.root().is_some() }
    }
}

#[cfg(feature = "dicom-preview")]
fn is_preview_surface_available(
    workspace: Option<&Path>, active: Option<&Path>, granted: &HashSet<PathBuf>,
) -> bool {
    workspace.is_some() || active.is_some_and(|p| granted.contains(p))
}

#[cfg(all(test, feature = "dicom-preview"))]
mod dicom_external_surface_tests {
    use super::*;

    #[test]
    fn explicitly_selected_dicom_is_visible_without_opening_vault() {
        let folder = PathBuf::from("test.zip");
        let mut granted = HashSet::new();
        assert!(!is_preview_surface_available(None, Some(&folder), &granted));
        granted.insert(folder.clone());
        assert!(is_preview_surface_available(None, Some(&folder), &granted));
        assert!(!is_preview_surface_available(None, Some(Path::new("unselected.zip")), &granted));
        assert!(is_preview_surface_available(Some(Path::new("workspace")), None, &granted));
        assert!(!is_preview_surface_available(None, None, &granted));
    }
}
