#[cfg(feature = "dicom-preview")] mod dicom;
mod changes;
use std::path::{Path, PathBuf};

use aquilum_core::files::models::{FileItem, FileItemType};
use aquilum_core::files::workspace::read_directory_impl;

use super::{Row, TreeModel, comparable};

impl TreeModel {
    pub fn expand_to(&mut self, path: &Path) {
        let Some(root) = self.root.clone() else { return };
        let mut folders: Vec<PathBuf> = path.ancestors().skip(1).take_while(|dir| dir.starts_with(&root) && *dir != root.as_path()).map(Path::to_path_buf).collect();
        folders.reverse();
        for folder in folders {
            let key = comparable(&folder);
            if self.expanded.insert(key.clone(), folder.clone()).is_none() && !self.directories.contains_key(&key) {
                self.load(&folder);
            }
        }
        self.rebuild();
    }

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
                // Win7-only release evidence: distinguish a successful disk
                // enumeration from a blank/non-rendering sidebar. Log counts,
                // never user filenames or document contents.
                #[cfg(target_os = "windows")]
                if crate::host::native_frame() {
                    let txt = items.iter().filter(|item| {
                        item.item_type == FileItemType::File
                            && aquilum_core::search::paths::is_plain_text(Path::new(&item.id))
                    }).count();
                    if txt > 0 || self.root.as_deref() == Some(dir) {
                        report!(
                            "WIN7_TREE_SCAN root={} entries={} txt={}",
                            self.root.as_deref() == Some(dir),
                            items.len(),
                            txt
                        );
                    }
                }
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
        if let Some(root) = &self.root
            && let Some(items) = self.directories.get(&comparable(root))
        {
            self.visit(items, 0, &[], &mut rows);
        }
        self.rows = rows;
    }

    fn visit(&self, items: &[FileItem], depth: usize, continuing: &[usize], rows: &mut Vec<Row>) {
        let active = self.active.as_deref().map(comparable);
        for (index, item) in items.iter().enumerate() {
            let path = PathBuf::from(&item.id);
            let key = comparable(&path);
            let folder = item.item_type == FileItemType::Folder;
            let expanded = folder && self.expanded.contains_key(&key);
            let count = if folder { self.counts.total(&path) } else { None };
            let color = if folder { self.colors.get(&key).map(|(_, color)| *color) } else { None };
            rows.push(Row {
                path,
                name: item.name.clone(),
                folder,
                depth,
                expanded,
                color,
                items: count,
                guides: continuing.to_vec(),
                active: active.as_deref() == Some(key.as_str()),
                selected: self.selected.contains(&key),
            });
            if expanded && let Some(children) = self.directories.get(&key).filter(|c| !c.is_empty()) {
                let is_last = index == items.len() - 1;
                let mut guides: Vec<usize> = continuing
                    .iter()
                    .copied()
                    .filter(|&g| !(is_last && depth > 0 && g == depth - 1))
                    .collect();
                guides.push(depth);
                self.visit(children, depth + 1, &guides, rows);
            }
        }
    }
}
