use super::*;
use std::collections::HashSet;

impl TreeModel {
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

}
