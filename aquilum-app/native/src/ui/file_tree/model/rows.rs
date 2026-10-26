use std::path::PathBuf;

use aquilum_core::files::models::{FileItem, FileItemType};

use super::counts::IndexedCounts;
use super::{Row, TreeModel, comparable};

impl TreeModel {
    pub(super) fn rebuild(&mut self) {
        let (mut rows, totals) = (Vec::new(), self.counts.indexed());
        if let Some(root) = &self.root
            && let Some(items) = self.directories.get(&comparable(root))
        {
            self.visit(items, 0, &[], &mut rows, &totals);
        }
        self.rows = rows;
    }

    fn visit(&self, items: &[FileItem], depth: usize, continuing: &[usize], rows: &mut Vec<Row>, totals: &IndexedCounts<'_>) {
        let active = self.active.as_deref().map(comparable);
        for (index, item) in items.iter().enumerate() {
            let path = PathBuf::from(&item.id);
            let key = comparable(&path);
            let folder = item.item_type == FileItemType::Folder;
            let expanded = folder && self.expanded.contains_key(&key);
            let count = if folder { totals.total(&key) } else { None };
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
                self.visit(children, depth + 1, &guides, rows, totals);
            }
        }
    }
}
