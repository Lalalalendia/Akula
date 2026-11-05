#[cfg(feature = "dicom-preview")] mod dicom;
#[cfg(feature = "html-preview")] mod html;
mod changes;
mod expand;
#[cfg(target_os = "windows")]
mod win7_scan;
use std::path::{Path, PathBuf};
use aquilum_core::files::models::{FileItem, FileItemType};
use aquilum_core::files::workspace::read_directory_impl;
use super::{Row, TreeModel, comparable};
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
                #[cfg(feature = "html-preview")]
                html::append_html_entries(dir, &mut items);
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
