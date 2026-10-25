//! Explicit draft editor: no Enter-to-save, no auto-commit, no Notion writes.
use super::*;
use crate::ui::{editor::TextEditor, style};
use crate::notion_bases::{DraftView, EditKind, Stage};

pub(super) fn append(
    mut column: Flex, draft: &DraftView, stage: Stage,
    bindings: &mut HashMap<WidgetId, Binding>,
) -> Flex {
    let label=match draft.kind {
        EditKind::Text => "Текст: локальная правка без форматирования Notion",
        EditKind::Number => "Число: десятичный разделитель — точка",
    };
    column=column.with_fixed(selectable(
        &format!("Редактирование: {}", draft.column_name),
        |t|t.text_primary,None,Leading::Times(1.3),
    ));
    column=column.with_fixed(selectable(
        label, |t|t.text_muted,None,Leading::Times(1.3),
    ));
    if stage==Stage::Ready && !draft.frozen {
        let field=NewWidget::new(
            TextEditor::single_line(&draft.value)
                .styled(||style::ui_text(size::INPUT_CONTENT_HEIGHT as f32))
        ).with_class(style::FIELD);
        bindings.insert(field.id(),Binding::BasesDraft);
        column=column.with_fixed(field);
        column=column.with_fixed(action_button(
            "Сохранить локальную правку".into(),
            Action::NotionBasesSaveDraft,bindings,
        ));
    } else {
        column=column.with_fixed(selectable(
            "Сохранение заблокировано: ревизия могла измениться. Скопируйте текст и перечитайте базу перед новой правкой.",
            |t|t.text_secondary,None,Leading::Times(1.3),
        ));
        column=column.with_fixed(selectable(
            &draft.value,|t|t.text_primary,None,Leading::Times(1.3),
        ));
    }
    column.with_fixed(action_button(
        "Отменить черновик".into(),Action::NotionBasesCancelDraft,bindings,
    ))
}
