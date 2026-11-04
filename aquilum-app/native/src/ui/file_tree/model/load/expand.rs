use super::*;

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
}
