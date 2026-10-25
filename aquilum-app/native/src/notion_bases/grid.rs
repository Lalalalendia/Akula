//! Bounded, native-only selection model for Akula Bases cells.
//! A selection contains UI metadata, not a write authorization. Every
//! mutation is revalidated against the effective model and File Gate CAS.
use super::{BasesSession, Stage};

#[derive(Clone, Debug)]
pub struct SelectedCell {
    pub row_id: String,
    pub row_label: String,
    pub property_id: String,
    pub column_name: String,
    pub value_preview: String,
    pub edit_text: bool,
    pub toggle: bool,
    pub revert: bool,
    pub local_only: bool,
}

impl BasesSession {
    /// Selection must belong to the currently visible bounded grid window.
    /// Arbitrary IDs from an old popup cannot select another source/row.
    pub fn select_cell(&mut self, row_id: &str, property_id: &str) -> bool {
        if self.view.stage != Stage::Ready || self.view.busy
            || self.view.draft.is_some() || self.view.schema_draft.is_some() {
            return false;
        }
        let window = self.view();
        let Some(column) = window.columns.iter()
            .find(|col| col.property_id == property_id) else {
            return false;
        };
        let Some(row) = window.rows.iter()
            .find(|row| row.row_id == row_id) else {
            return false;
        };
        let Some(cell) = row.cells.iter()
            .find(|cell| cell.property_id == property_id) else {
            return false;
        };
        self.view.selected = Some(SelectedCell {
            row_id: row_id.to_owned(),
            row_label: row.label.clone(),
            property_id: property_id.to_owned(),
            column_name: column.name.clone(),
            value_preview: cell.text.clone(),
            edit_text: cell.edit_text,
            toggle: cell.toggle_to.is_some(),
            revert: cell.revert,
            local_only: column.local_only,
        });
        true
    }

    pub fn clear_selection(&mut self) -> bool {
        self.view.selected.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::notion_api_probe::bases_edits::{
        EffectiveBase, EffectiveCell, EffectiveColumn, EffectiveRow, EffectiveValue,
    };
    use std::sync::Arc;

    fn session() -> BasesSession {
        let mut session = BasesSession::new(Arc::new(|| {}));
        session.view.stage = Stage::Ready;
        session.effective = Some(EffectiveBase {
            base_id: "base".into(),
            notion_source_id: "source".into(),
            source_root: "root".into(),
            source_digest: "a".repeat(64),
            local_revision: 0,
            columns: vec![
                EffectiveColumn {
                    property_id: "c1".into(), name: "Name".into(),
                    kind: "rich_text".into(), local_only: false,
                    editable_locally: true,
                },
                EffectiveColumn {
                    property_id: "c2".into(), name: "Formula".into(),
                    kind: "formula".into(), local_only: false,
                    editable_locally: false,
                },
            ],
            rows: vec![EffectiveRow {
                akula_id: "r1".into(), notion_page_id: "n1".into(),
                cells: vec![
                    EffectiveCell {
                        property_id: "c1".into(),
                        value: EffectiveValue::Source(serde_json::json!({
                            "type":"rich_text","rich_text":[{"plain_text":"Alpha"}]
                        })),
                    },
                    EffectiveCell {
                        property_id: "c2".into(),
                        value: EffectiveValue::Source(serde_json::json!({
                            "type":"formula","formula":{"number":10}
                        })),
                    },
                ],
            }],
            saved_source_views: 1, native_views_supported: false,
        });
        session
    }

    #[test]
    fn visible_cell_selection_is_bounded_and_not_write_authority() {
        let mut s = session();
        assert!(!s.select_cell("missing", "c1"));
        assert!(!s.select_cell("r1", "missing"));
        assert!(s.select_cell("r1", "c1"));
        let selected = s.view().selected.unwrap();
        assert!(selected.edit_text);
        assert_eq!(selected.value_preview, "Alpha");
        assert_eq!(selected.row_id, "r1");
        assert!(!selected.local_only);
        assert!(s.select_cell("r1", "c2"));
        let formula = s.view().selected.unwrap();
        assert!(!formula.edit_text && !formula.toggle);
        assert_eq!(formula.property_id, "c2");
        assert!(s.clear_selection());
        assert!(!s.clear_selection());
    }

    #[test]
    fn selection_cannot_escape_active_page_or_dirty_draft() {
        let mut s = session();
        assert!(s.select_cell("r1", "c1"));
        s.view.row_start = 10; // Simulate a page no longer containing row r1.
        assert!(!s.select_cell("r1", "c1"));
        s.view.row_start = 0;
        s.view.busy = true;
        assert!(!s.select_cell("r1", "c1"));
        s.view.busy = false;
        s.view.stage = Stage::NeedsReview;
        assert!(!s.select_cell("r1", "c1"));
    }
}
