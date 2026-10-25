//! Native bounded row-by-column grid for the Notion-independent Akula Bases QA
//! preview. Fixed-width cells and horizontal scrolling avoid multi-line action
//! stacks or unbounded widget trees. Each cell is keyboard-accessible as a button.
use super::*;
use crate::notion_bases::{BasesView, CellView, ColumnView};
use crate::ui::widgets::ButtonSize;
use masonry::widgets::SizedBox;

const ROW_LABEL_WIDTH: f64 = 166.0;
const CELL_WIDTH: f64 = 172.0;
const CELL_LABEL_CHARS: usize = 32;

fn abbreviate(s: &str) -> String {
    let mut chars = s.chars();
    let value = chars.by_ref().take(CELL_LABEL_CHARS).collect::<String>();
    if chars.next().is_some() { format!("{value}…") } else { value }
}
fn header(label: &str, width: f64) -> NewWidget<SizedBox> {
    let text = selectable(label, |t| t.text_secondary, None, Leading::Times(1.1));
    NewWidget::new(SizedBox::new(text).width(Length::px(width)))
}
fn grid_cell(
    cell: &CellView, selected: bool, bindings: &mut HashMap<WidgetId, Binding>,
) -> NewWidget<SizedBox> {
    let text = abbreviate(&cell.text.replace(['\r', '\n', '\t'], " "));
    let button = NewWidget::new(
        TextButton::new(text, Variant::Ghost).pressed(selected).size(ButtonSize::Xs),
    );
    bindings.insert(
        button.id(),
        Binding::Button(Action::NotionBasesSelectCell(
            cell.row_id.clone(), cell.property_id.clone(),
        )),
    );
    NewWidget::new(SizedBox::new(button).width(Length::px(CELL_WIDTH)))
}
fn header_row(columns: &[ColumnView]) -> Flex {
    let mut row = Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .with_fixed(header("Строка / колонка", ROW_LABEL_WIDTH));
    for col in columns {
        row = row.with_fixed(header(&abbreviate(&col.name), CELL_WIDTH));
    }
    row
}
pub(super) fn append(
    mut column: Flex, bases: &BasesView, bindings: &mut HashMap<WidgetId, Binding>,
) -> Flex {
    column = column.with_fixed(selectable(
        &format!("Таблица: строки {}–{} из {}, колонки {}–{} из {}",
            if bases.total_rows==0 {0} else {bases.row_start+1},
            bases.row_start+bases.rows.len(), bases.total_rows,
            if bases.total_columns==0 {0} else {bases.column_start+1},
            bases.column_start+bases.columns.len(), bases.total_columns),
        |t| t.text_muted, None, Leading::Times(1.3),
    ));
    let mut grid = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(NewWidget::new(header_row(&bases.columns)));
    for (i, record) in bases.rows.iter().enumerate() {
        let label = format!("{}  {}", bases.row_start+i+1, abbreviate(&record.label));
        let mut line = Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .with_fixed(header(&label, ROW_LABEL_WIDTH));
        for cell in &record.cells {
            let selected = bases.selected.as_ref().is_some_and(|v|
                v.row_id==cell.row_id && v.property_id==cell.property_id);
            line = line.with_fixed(grid_cell(cell, selected, bindings));
        }
        grid = grid.with_fixed(NewWidget::new(line));
    }
    // The provider modal itself is vertically scrollable. Only the bounded
    // grid scrolls horizontally; it never renders off-page rows or columns.
    column = column.with_fixed(NewWidget::new(
        ScrollArea::horizontal(NewWidget::new(grid)),
    ));
    column = column.with_fixed(selectable(
        "Выберите ячейку мышью или клавиатурой. Изменения сохраняются отдельно через File Gate.",
        |t|t.text_muted,None,Leading::Times(1.3),
    ));
    if let Some(selected) = &bases.selected {
        column = column.with_fixed(selectable(
            &format!("Выбрано: {} · {} = {}",
                selected.row_label, selected.column_name, selected.value_preview),
            |t|t.text_primary,None,Leading::Times(1.3),
        ));
        if selected.edit_text {
            column = column.with_fixed(action_button(
                "Редактировать выбранную ячейку".into(),
                Action::NotionBasesBeginEdit(
                    selected.row_id.clone(), selected.property_id.clone(),
                ), bindings,
            ));
        }
        if selected.toggle {
            column = column.with_fixed(action_button(
                "Переключить выбранный флажок".into(),
                Action::NotionBasesChange(
                    selected.row_id.clone(), selected.property_id.clone(), false,
                ), bindings,
            ));
        }
        if selected.revert {
            column = column.with_fixed(action_button(
                "Отменить локальную правку выбранной ячейки".into(),
                Action::NotionBasesChange(
                    selected.row_id.clone(), selected.property_id.clone(), true,
                ), bindings,
            ));
        }
        if selected.local_only {
            column = column.with_fixed(action_button(
                "Переименовать выбранную локальную колонку".into(),
                Action::NotionBasesRenameColumn(selected.property_id.clone()),
                bindings,
            ));
        }
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mounted_grid_creates_actions_only_for_the_bounded_visible_window() {
        use crate::notion_bases::{ColumnView, RowView};
        let mut view = BasesView::default();
        view.total_rows = 1000;
        view.total_columns = 100;
        view.columns = (0..6).map(|i| ColumnView {
            name: format!("Column {i}"), property_id: format!("column-{i}"),
            kind: "rich_text".into(), local_only: false,
        }).collect();
        view.rows = (0..10).map(|i| {
            let row_id = format!("row-{i}");
            RowView {
                row_id: row_id.clone(), label: format!("Row {i}"),
                cells: view.columns.iter().map(|col| CellView {
                    row_id: row_id.clone(), property_id: col.property_id.clone(),
                    text: format!("{row_id}:{}", col.property_id),
                    toggle_to: None, revert: false, edit_text: true,
                }).collect(),
            }
        }).collect();
        let mut bindings = HashMap::new();
        let _ = append(Flex::column(), &view, &mut bindings);
        assert_eq!(bindings.len(),60, "only ten by six visible cells are mounted");
        assert!(bindings.values().any(|action|matches!(action,
            Binding::Button(Action::NotionBasesSelectCell(row,col))
                if row=="row-9" && col=="column-5")));
        assert!(!bindings.values().any(|action|matches!(action,
            Binding::Button(Action::NotionBasesSelectCell(row,_))
                if row=="row-10")));
    }
    #[test]
    fn grid_abbreviation_is_unicode_safe() {
        assert_eq!(abbreviate("Привет"),"Привет");
        let long = "🍂".repeat(CELL_LABEL_CHARS+1);
        assert_eq!(abbreviate(&long),format!("{}…","🍂".repeat(CELL_LABEL_CHARS)));
    }
}
