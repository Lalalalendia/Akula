//! Experimental local Akula Bases panel. Never reads the Notion API.
use super::*;

pub(super) fn append(
    mut column: Flex, ctx: &Context,
    bindings: &mut HashMap<WidgetId, Binding>,
    selected_root: Option<&String>,
) -> Flex {
    // Independent offline Akula Bases, separate from the Notion API cache.
    if let (Some(source), Some(target)) = (selected_root,ctx.workspace.as_deref()) {
        let bases=&ctx.notion_bases;
        if !bases.busy {
            column=column.with_fixed(action_button(
                "Открыть локальные Akula Bases".into(),
                Action::NotionBasesOpen(source.clone()),bindings,
            ));
        }
        if bases.source.as_deref()==Some(source.as_str())
            && bases.target.as_deref()==Some(target)
        {
            column=column.with_fixed(selectable(
                match bases.stage {
                    crate::notion_bases::Stage::Idle=>"Локальные базы ещё не открыты",
                    crate::notion_bases::Stage::Loading=>"Читаю независимые базы в рабочем потоке...",
                    crate::notion_bases::Stage::Ready=>"Локальная модель Akula Bases",
                    crate::notion_bases::Stage::Saving=>"Сохраняю через File Gate CAS...",
                    crate::notion_bases::Stage::NeedsReview=>"База или ревизия изменилась. Требуется проверка.",
                },|t|t.text_secondary,None,Leading::Times(1.3),
            ));
            if let Some(notice)=bases.notice {
                column=column.with_fixed(selectable(
                    notice,|t|t.text_muted,None,Leading::Times(1.3),
                ));
            }
            if bases.stage==crate::notion_bases::Stage::Ready {
                for choice in &bases.choices {
                    column=column.with_fixed(action_button(
                        format!("{} · {}",choice.label,&choice.digest[..10]),
                        Action::NotionBasesSelect(choice.digest.clone(),choice.base_id.clone()),
                        bindings,
                    ));
                }
                if bases.base_id.is_some() {
                    column=column.with_fixed(selectable(
                        &format!("Строк: {} · колонок: {} · локальная ревизия: {} · исходных views: {} (только снимки)",
                            bases.total_rows,bases.total_columns,bases.local_revision,bases.saved_source_views),
                        |t|t.text_primary,None,Leading::Times(1.3),
                    ));
                    for (i,row) in bases.rows.iter().enumerate() {
                        column=column.with_fixed(selectable(
                            &format!("{}. {}",bases.row_start+i+1,row.label),
                            |t|t.text_secondary,None,Leading::Times(1.3),
                        ));
                        for (col,cell) in bases.columns.iter().zip(&row.cells) {
                            column=column.with_fixed(selectable(
                                &format!("{}: {}",col.name,cell.text),
                                |t|t.text_muted,None,Leading::Times(1.3),
                            ));
                            if cell.toggle_to.is_some() {
                                column=column.with_fixed(action_button(
                                    "Изменить флажок локально".into(),
                                    Action::NotionBasesChange(cell.row_id.clone(),cell.property_id.clone(),false),bindings,
                                ));
                            }
                            if cell.revert {
                                column=column.with_fixed(action_button(
                                    "Вернуть исходное значение".into(),
                                    Action::NotionBasesChange(cell.row_id.clone(),cell.property_id.clone(),true),bindings,
                                ));
                            }
                        }
                    }
                    if bases.row_start>0 {
                        column=column.with_fixed(action_button(
                            "← Предыдущие строки".into(),
                            Action::NotionBasesPage(bases.row_start.saturating_sub(10),bases.column_start),bindings,
                        ));
                    }
                    if bases.row_start+bases.rows.len()<bases.total_rows {
                        column=column.with_fixed(action_button(
                            "Следующие строки →".into(),
                            Action::NotionBasesPage(bases.row_start+10,bases.column_start),bindings,
                        ));
                    }
                    if bases.column_start>0 {
                        column=column.with_fixed(action_button(
                            "← Предыдущие колонки".into(),
                            Action::NotionBasesPage(bases.row_start,bases.column_start.saturating_sub(6)),bindings,
                        ));
                    }
                    if bases.column_start+bases.columns.len()<bases.total_columns {
                        column=column.with_fixed(action_button(
                            "Следующие колонки →".into(),
                            Action::NotionBasesPage(bases.row_start,bases.column_start+6),bindings,
                        ));
                    }
                    column=column.with_fixed(action_button(
                        "Перечитать локальные изменения".into(),
                        Action::NotionBasesReload,bindings,
                    ));
                }
            } else if bases.stage==crate::notion_bases::Stage::NeedsReview {
                column=column.with_fixed(action_button(
                    "Проверить локальную базу повторно".into(),Action::NotionBasesReload,bindings,
                ));
            }
        }
    }

    column
}
