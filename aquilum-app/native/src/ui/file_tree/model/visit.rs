use std::path::PathBuf;
use aquilum_core::files::models::{FileItem, FileItemType};
use super::{Lists, Row, TreeModel, comparable};

impl TreeModel {
    pub(super) fn visit(&self, lists: &Lists, items: &[FileItem], depth: usize, continuing: &[usize], rows: &mut Vec<Row>) {
        let active = self.active.as_deref().map(comparable);
        for (index, item) in items.iter().enumerate() {
            let path = PathBuf::from(&item.id);
            let key = comparable(&path);
            let folder = item.item_type == FileItemType::Folder;
            let expanded = folder && if self.filter.is_some() { lists.contains_key(&key) } else { self.expanded.contains_key(&key) };
            let count = if folder && self.filter.is_none() { self.counts.total(&path) } else { None };
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
                marks: self.filter.as_ref().map_or_else(Vec::new, |filter| filter.marks(&item.name)),
                active: active.as_deref() == Some(key.as_str()),
                selected: self.selected.contains(&key),
            });
            if expanded && let Some(children) = lists.get(&key).filter(|c| !c.is_empty()) {
                let is_last = index == items.len() - 1;
                let mut guides: Vec<usize> = continuing
                    .iter()
                    .copied()
                    .filter(|&g| !(is_last && depth > 0 && g == depth - 1))
                    .collect();
                guides.push(depth);
                self.visit(lists, children, depth + 1, &guides, rows);
            }
        }
    }
}
