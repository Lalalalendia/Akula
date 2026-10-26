use std::collections::HashSet;
use std::path::{Path, PathBuf};

use aquilum_core::files::models::{FileItem, FileItemType};
use aquilum_core::files::workspace::read_directory_impl;

use super::{TreeModel, comparable};

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

    pub fn refresh_paths(&mut self, changed: &[PathBuf]) {
        let mut dirs = HashSet::new();
        for path in changed {
            let mut current = path.parent();
            while let Some(dir) = current {
                if self.directories.contains_key(&comparable(dir)) {
                    dirs.insert(dir.to_path_buf());
                    break;
                }
                current = dir.parent();
            }
        }
        for dir in dirs {
            self.load(&dir);
        }
        self.recount(changed);
        self.rebuild();
    }

    fn recount(&mut self, changed: &[PathBuf]) {
        let known: HashSet<PathBuf> = changed.iter().filter_map(|path| path.ancestors().skip(1).find(|dir| self.counts.knows(dir))).map(Path::to_path_buf).collect();
        for dir in known {
            self.counts.refresh(&dir);
        }
    }

    pub(super) fn load(&mut self, dir: &Path) {
        match read_directory_impl(dir) {
            Ok(mut items) => {
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

}
