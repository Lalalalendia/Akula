//! C5b4 comparison panel; only a conflict-free, explicit CAS action can apply.
use super::*;
use aquilum_core::notion_api_probe::bases_edits::RebaseDisposition;
use crate::notion_bases::BasesView;
use std::collections::BTreeSet;

fn conflict_text(kind: RebaseDisposition) -> &'static str {
    match kind {
        RebaseDisposition::Eligible => "исходное значение без изменений",
        RebaseDisposition::RowRemoved => "строка исчезла из нового снимка",
        RebaseDisposition::RowMoved => "строка перемещена или изменила идентичность",
        RebaseDisposition::PropertyRemoved => "исходная колонка исчезла",
        RebaseDisposition::PropertyTypeChanged => "тип исходной колонки изменён",
        RebaseDisposition::PropertyDefinitionChanged => "параметры исходной колонки изменены",
        RebaseDisposition::SourceValueChanged => "в Notion изменено исходное значение",
        RebaseDisposition::LocalColumnConflict => "конфликт локальной схемы",
        RebaseDisposition::InvalidNewTarget => "локальное значение несовместимо с новым снимком",
    }
}
pub(super) fn append(
    mut column: Flex, bases: &BasesView,
    bindings: &mut HashMap<WidgetId, Binding>,
) -> Flex {
    let (Some(base_id), Some(old_digest)) =
        (bases.base_id.as_deref(),bases.digest.as_deref()) else {return column;};
    column=column.with_fixed(selectable(
        "C5b4 · предварительное сравнение Notion; перенос возможен только после подтверждения и при отсутствии конфликтов",
        |t|t.text_secondary,None,Leading::Times(1.3),
    ));
    let mut candidates=BTreeSet::new();
    for choice in &bases.choices {
        if choice.base_id==base_id && choice.digest!=old_digest
            && candidates.insert(choice.digest.as_str()) {
            column=column.with_fixed(action_button(
                format!("Сравнить снимок {} с текущим", &choice.digest[..10]),
                Action::NotionBasesPreviewRebase(choice.digest.clone()),bindings,
            ));
        }
    }
    if candidates.is_empty() {
        column=column.with_fixed(selectable(
            "Нет второго проверенного снимка той же базы",
            |t|t.text_muted,None,Leading::Times(1.3),
        ));
    }
    if let Some(report)=&bases.rebase_preview {
        column=column.with_fixed(selectable(
            &format!("Снимки: {} → {} · локальная ревизия: {} · готовы: {} · конфликты ячеек: {} · конфликты колонок: {}",
                &report.old_digest[..10],&report.new_digest[..10],
                report.local_revision,report.ready_cells,
                report.conflicted_cells,report.conflicted_columns),
            |t|t.text_primary,None,Leading::Times(1.3),
        ));
        let mut displayed=0usize;
        for col in report.columns.iter().filter(|c|!c.compatible).take(10) {
            column=column.with_fixed(selectable(
                &format!("Конфликт колонки: {} (локальный ID сохранён)",col.name),
                |t|t.text_secondary,None,Leading::Times(1.3),
            ));
            displayed+=1;
        }
        for cell in report.cells.iter()
            .filter(|c|c.disposition!=RebaseDisposition::Eligible)
            .take(10usize.saturating_sub(displayed)) {
            column=column.with_fixed(selectable(
                &format!("Конфликт {} / {}: {}",
                    cell.row_id.chars().take(12).collect::<String>(),
                    cell.property_id.chars().take(12).collect::<String>(),
                    conflict_text(cell.disposition)),
                |t|t.text_secondary,None,Leading::Times(1.3),
            ));
        }
        if report.can_offer_approval() {
            column=column.with_fixed(action_button(
                "Применить совместимый снимок через File Gate (с подтверждением)".into(),
                Action::NotionBasesApplyRebase,bindings,
            ));
            column=column.with_fixed(selectable(
                "Перед записью источник и хеш будут проверены повторно. Это локальный перенос; данные Notion не меняются.",
                |t|t.text_muted,None,Leading::Times(1.3),
            ));
        } else {
            column=column.with_fixed(selectable(
                "Есть конфликты. Перенос заблокирован, старые локальные правки не изменены.",
                |t|t.text_muted,None,Leading::Times(1.3),
            ));
        }
    }
    column
}
