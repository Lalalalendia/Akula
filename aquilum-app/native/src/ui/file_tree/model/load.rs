#[cfg(feature = "dicom-preview")] mod dicom;
mod changes;
mod expand;
#[cfg(target_os = "windows")]
mod win7_scan;
use std::path::{Path, PathBuf};

use aquilum_core::files::models::{FileItem, FileItemType};
use aquilum_core::files::workspace::read_directory_impl;

use super::{TreeModel, comparable};

impl TreeModel {
    pub fn reload_all(&mut self) {
        self.directories.clear();
        self.times.clear();
        if let Some(root) = self.root.clone() {
            self.load(&root);
        }
        self.rebuild();
    }

    pub(super) fn load(&mut self, dir: &Path) {
        match read_directory_impl(dir) {
            Ok(mut items) => {
                #[cfg(feature = "dicom-preview")]
                dicom::append_dicom_entries(dir, &mut items);
                #[cfg(target_os = "windows")]
                win7_scan::trace(self.root.as_deref(), dir, &items);
                if self.sort.by_time() {
                    self.stamp(&items);
                }
                self.order(&mut items);
                self.directories.insert(comparable(dir), items);
            }
            Err(e) => {
                self.directories.remove(&comparable(dir));
                report!("папка {} не прочитана: {e}", dir.display());
            }
        }
        let nested: Vec<PathBuf> = self.directories.get(&comparable(dir)).into_iter().flatten()
            .filter(|i| i.item_type == FileItemType::Folder && self.expanded.contains_key(&comparable(Path::new(&i.id))))
            .filter(|i| !self.directories.contains_key(&comparable(Path::new(&i.id))))
            .map(|i| PathBuf::from(&i.id))
            .collect();
        for folder in nested {
            self.load(&folder);
        }
    }

    pub(super) fn rebuild(&mut self) {
        let mut rows = Vec::new();
        if let Some(root) = self.root.clone() {
            let key = comparable(&root);
            if self.filtering() {
                let kept = self.survivors(&root);
                if let Some(items) = kept.get(&key) {
                    self.visit(&kept, items, 0, &[], &mut rows);
                }
            } else if let Some(items) = self.directories.get(&key) {
                self.visit(&self.directories, items, 0, &[], &mut rows);
            }
        }
        self.rows = rows;
    }

}
