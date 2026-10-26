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
        "C5b4 · сравнение снимков и ручное разрешение совместимых конфликтов, с подтверждением",
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
        let manual_possible=report.conflicted_columns==0
            && report.conflicted_cells>0 && report.conflicted_cells<=10
            && report.cells.iter().all(|cell|match cell.disposition {
                RebaseDisposition::Eligible=>true,
                RebaseDisposition::SourceValueChanged=>cell.manual_review_supported,
                _=>false,
            });
        if manual_possible {
            column=column.with_fixed(selectable(
                "Для каждого конфликта выберите: сохранить локальное или принять новое Notion. Принятие Notion требует проверки архивной копии прежних локальных правок.",
                |t|t.text_secondary,None,Leading::Times(1.3),
            ));
            for cell in report.cells.iter().filter(|cell|
                cell.disposition==RebaseDisposition::SourceValueChanged) {
                let marked=bases.keep_local.iter().any(|(row,prop)|
                    row==&cell.row_id && prop==&cell.property_id);
                let accepted=bases.accept_notion.iter().any(|(row,prop)|
                    row==&cell.row_id && prop==&cell.property_id);
                column=column.with_fixed(selectable(
                    &format!("Было в Notion: {} · Стало в Notion: {} · Локально: {}",
                        cell.previous_source.as_deref().unwrap_or("недоступно"),
                        cell.updated_source.as_deref().unwrap_or("недоступно"),
                        cell.local_override.as_deref().unwrap_or("недоступно")),
                    |t|t.text_secondary,None,Leading::Times(1.3),
                ));
                column=column.with_fixed(action_button(
                    format!("{}: сохранить локальное · {} / {}",
                        if marked {"Выбрано"} else {"Выбрать"},
                        cell.row_id.chars().take(10).collect::<String>(),
                        cell.property_id.chars().take(10).collect::<String>()),
                    Action::NotionBasesToggleKeepLocal(
                        cell.row_id.clone(),cell.property_id.clone()),bindings,
                ));
                column=column.with_fixed(action_button(
                    format!("{}: принять новое значение Notion (с архивом)",
                        if accepted {"Выбрано"} else {"Выбрать"}),
                    Action::NotionBasesToggleAcceptNotion(
                        cell.row_id.clone(),cell.property_id.clone()),bindings,
                ));
            }
            if crate::notion_bases::rebase::manual_choices_complete(bases) {
                column=column.with_fixed(action_button(
                    "Подтвердить решения и применить через File Gate CAS".into(),
                    Action::NotionBasesApplyReviewedRebase,bindings,
                ));
            }
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
    column=column.with_fixed(action_button(
        "Найти архивы локальных правок для восстановления".into(),
        Action::NotionBasesScanArchives,bindings,
    ));
    for archive in &bases.archives {
        column=column.with_fixed(selectable(
            &format!("Проверенный архив: {} · исходный снимок {}",
                archive.path,archive.old_digest.chars().take(10).collect::<String>()),
            |t|t.text_secondary,None,Leading::Times(1.3),
        ));
        for cell in &archive.cells {
            column=column.with_fixed(selectable(
                &format!("Прежнее локальное значение: {}",cell.value_preview),
                |t|t.text_primary,None,Leading::Times(1.3),
            ));
            column=column.with_fixed(action_button(
                format!("Восстановить из архива · {} / {}",
                    cell.row_id.chars().take(10).collect::<String>(),
                    cell.property_id.chars().take(10).collect::<String>()),
                Action::NotionBasesRestoreListed(
                    archive.path.clone(),cell.row_id.clone(),
                    cell.property_id.clone()),bindings,
            ));
        }
    }
    if let Some(archive)=&bases.last_archive {
        if !bases.archives.iter().any(|listed|listed.path==archive.path) {
        column=column.with_fixed(selectable(
            &format!("Архив прежних локальных правок: {} (в Vault; не удалять)",
                archive.path),
            |t|t.text_secondary,None,Leading::Times(1.3),
        ));
        for (row,prop) in &archive.accepted {
            column=column.with_fixed(action_button(
                format!("Восстановить локальное значение из архива · {} / {}",
                    row.chars().take(10).collect::<String>(),
                    prop.chars().take(10).collect::<String>()),
                Action::NotionBasesRestoreArchived(row.clone(),prop.clone()),bindings,
            ));
        }
        }
    }
    column
}

#[cfg(test)]
#[path = "rebase/tests.rs"]
mod tests;
