//! Explicit QA-only Akula column schema editor.
//! Never exposes rename on original Notion columns.
use super::*;
use aquilum_core::notion_api_probe::bases_edits::LocalKind;
use crate::notion_bases::{SchemaDraftView, SchemaMode, Stage};
use crate::ui::{editor::TextEditor, style};

pub(super) fn new_column_buttons(
    mut column: Flex, bindings: &mut HashMap<WidgetId, Binding>,
) -> Flex {
    for (kind,label) in [
        (LocalKind::Text,"Добавить текстовую колонку Akula"),
        (LocalKind::Number,"Добавить числовую колонку Akula"),
        (LocalKind::Checkbox,"Добавить колонку-флажок Akula"),
    ] {
        column=column.with_fixed(action_button(
            label.into(),Action::NotionBasesNewColumn(kind),bindings,
        ));
    }
    column
}
pub(super) fn append(
    mut column: Flex, draft: &SchemaDraftView, stage: Stage,
    bindings: &mut HashMap<WidgetId, Binding>,
) -> Flex {
    let operation=match &draft.mode {
        SchemaMode::Create{..}=>"Новая локальная колонка Akula",
        SchemaMode::Rename{..}=>"Переименование локальной колонки",
    };
    column=column.with_fixed(selectable(
        &format!("{} · тип: {}",operation,draft.kind_label()),
        |t|t.text_primary,None,Leading::Times(1.4),
    ));
    column=column.with_fixed(selectable(
        "Исходная схема Notion не меняется. Новое имя сохраняется только через File Gate CAS.",
        |t|t.text_muted,None,Leading::Times(1.3),
    ));
    if stage==Stage::Ready && !draft.frozen {
        let field=NewWidget::new(
            TextEditor::single_line(&draft.name)
                .styled(||style::ui_text(size::INPUT_CONTENT_HEIGHT as f32))
        ).with_class(style::FIELD);
        bindings.insert(field.id(),Binding::BasesSchemaDraft);
        column=column.with_fixed(field);
        column=column.with_fixed(action_button(
            if draft.is_new(){"Создать колонку Akula".into()}
                else{"Сохранить новое имя".into()},
            Action::NotionBasesSaveSchema,bindings,
        ));
    } else {
        column=column.with_fixed(selectable(
            "После конфликта сохранение заблокировано. Скопируйте имя и перечитайте базу.",
            |t|t.text_secondary,None,Leading::Times(1.3),
        ));
        column=column.with_fixed(selectable(
            &draft.name,|t|t.text_primary,None,Leading::Times(1.3),
        ));
    }
    column.with_fixed(action_button(
        "Отменить изменение схемы".into(),Action::NotionBasesCancelSchema,bindings,
    ))
}
