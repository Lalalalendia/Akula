use super::*;

#[cfg(feature = "dicom-preview")]
pub(super) fn append_dicom_entries(dir: &Path, items: &mut Vec<FileItem>) {
    // Core directory enumeration deliberately ignores unknown extensions.
    // The experimental medical viewer opts in only in the native file tree;
    // default/installed Akula and core file APIs retain unchanged behavior.
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue; };
        if !file_type.is_file() || file_type.is_symlink()
            || entry.file_name().to_string_lossy().starts_with('.')
            || aquilum_core::search::paths::is_hidden_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        if !crate::dicom::is_candidate(&path) { continue; }
        let id = path.to_string_lossy().into_owned();
        if items.iter().any(|existing| existing.id == id) { continue; }
        items.push(FileItem {
            id,
            name: entry.file_name().to_string_lossy().into_owned(),
            item_type: FileItemType::File,
        });
    }
}

#[cfg(all(test, feature = "dicom-preview"))]
mod dicom_file_tree_tests {
    use super::*;

    #[test]
    fn dicomdir_dcm_and_extensionless_images_are_visible_only_by_opt_in() {
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path();
        std::fs::write(root.join("DICOMDIR"), vec![0; 256]).unwrap();
        std::fs::write(root.join("test.dCm"), vec![0; 256]).unwrap();
        let mut header = vec![0; 132];
        header[128..132].copy_from_slice(b"DICM");
        std::fs::write(root.join("IM000001"), header).unwrap();
        std::fs::write(root.join("other_binary"), vec![0; 132]).unwrap();
        std::fs::write(root.join(".hidden.dcm"), vec![0; 256]).unwrap();
        let mut files = Vec::new();
        append_dicom_entries(root, &mut files);
        let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"DICOMDIR"));
        assert!(names.contains(&"test.dCm"));
        assert!(names.contains(&"IM000001"));
        assert!(!names.contains(&"other_binary"));
        assert!(!names.contains(&".hidden.dcm"));
    }
}
