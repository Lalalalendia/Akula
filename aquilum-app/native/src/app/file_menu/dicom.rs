use super::*;

#[cfg(feature = "dicom-preview")]
pub(super) fn readonly_dicom_target(clicked: &Path, targets: &[PathBuf]) -> bool {
    crate::dicom::is_candidate(clicked) || targets.iter().any(|path| crate::dicom::is_candidate(path))
}

#[cfg(all(test, feature = "dicom-preview"))]
mod dicom_menu_readonly_tests {
    use super::*;

    #[test]
    fn medical_zip_dicomdir_and_multiselect_cannot_offer_destructive_actions() {
        let base = tempfile::tempdir().unwrap();
        let study = base.path().join("study");
        std::fs::create_dir(&study).unwrap();
        std::fs::write(study.join("DICOMDIR"), [0u8; 132]).unwrap();
        assert!(readonly_dicom_target(&study, &[]));
        assert!(readonly_dicom_target(Path::new("case.zip"), &[]));
        assert!(readonly_dicom_target(Path::new("slice.DCM"), &[]));
        let candidates = vec![PathBuf::from("note.md"), PathBuf::from("other.dcm")];
        assert!(readonly_dicom_target(Path::new("note.md"), &candidates));
        assert!(!readonly_dicom_target(Path::new("ordinary.md"), &[]));
    }
}
