//! User-invoked offline Notion reader. All buttons stay read-only.
//! Verified source loading happens in NotionSession's worker, not paint.
use super::*;
use crate::notion_offline::{LoadState, PAGE_BATCH, TEXT_CHARS};

fn action_button(
    title: String, action: Action, bindings: &mut HashMap<WidgetId, Binding>,
) -> NewWidget<TextButton> {
    let button = NewWidget::new(TextButton::new(title, Variant::Ghost));
    bindings.insert(button.id(), Binding::Button(action));
    button
}

pub(super) fn body(
    ctx: &Context, bindings: &mut HashMap<WidgetId, Binding>,
) -> NewWidget<Flex> {
    let mut column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(selectable(
            "Notion — сохранённые источники (офлайн, только чтение)",
            |t| t.text_primary, None, Leading::Times(1.3),
        ))
        .with_fixed(selectable(
            "Notion остаётся владельцем данных. Эта тестовая функция не подключает аккаунт, не обновляет источники и не изменяет Vault.",
            |t| t.text_muted, None, Leading::Times(1.4),
        ));
    if let Some(reason) = ctx.notion_offline.error {
        column = column.with_fixed(selectable(
            &format!("Кэш требует проверки: {reason}"),
            |t| t.text_secondary, None, Leading::Times(1.4),
        ));
    } else if ctx.notion_offline.roots.is_empty() {
        column = column.with_fixed(selectable(
            "Нет сохранённых снимков Notion. Первичную загрузку пока выполняет отдельный API-инструмент.",
            |t| t.text_secondary, None, Leading::Times(1.4),
        ));
    } else {
        for source in &ctx.notion_offline.roots {
            let chosen = ctx.notion_selection.root.as_deref() == Some(&source.id);
            let label = format!("{} {} · объектов: {} · связей: {}",
                if chosen { "●" } else { "○" }, &source.id[..8],
                source.objects, source.links);
            column = column.with_fixed(action_button(
                label, Action::NotionOpenRoot(source.id.clone()), bindings,
            ));
        }
    }

    let status = &ctx.notion_selection;
    match status.state {
        LoadState::Idle => {}
        LoadState::Loading => {
            column = column.with_fixed(selectable(
                "Проверяю контрольную сумму и читаю страницы в отдельном потоке...",
                |t| t.text_secondary, None, Leading::Times(1.4),
            ));
        }
        LoadState::NeedsReview => {
            column = column.with_fixed(selectable(
                "Снимок не прошёл проверку целостности или недоступен. Содержимое скрыто.",
                |t| t.text_secondary, None, Leading::Times(1.4),
            ));
        }
        LoadState::Ready => {
            column = column.with_fixed(selectable(
                &format!("Проверенный снимок: {} страниц. Выберите страницу.", status.page_count),
                |t| t.text_primary, None, Leading::Times(1.3),
            ));
            for page in &status.displayed_pages {
                let selected = status.chosen.as_ref().is_some_and(|p| p.id == page.id);
                column = column.with_fixed(action_button(
                    format!("{} {}", if selected { "●" } else { "○" }, page.title),
                    Action::NotionOpenPage(page.id.clone()), bindings,
                ));
            }
            if status.page_start > 0 {
                column = column.with_fixed(action_button(
                    "← Предыдущие страницы".into(),
                    Action::NotionPageBatch(status.page_start.saturating_sub(PAGE_BATCH)), bindings,
                ));
            }
            if status.page_start + status.displayed_pages.len() < status.page_count {
                column = column.with_fixed(action_button(
                    "Следующие страницы →".into(),
                    Action::NotionPageBatch(status.page_start + PAGE_BATCH), bindings,
                ));
            }
            if let Some(page) = &status.chosen {
                column = column.with_fixed(selectable(
                    &format!("Чтение: {}", page.title),
                    |t| t.text_primary, None, Leading::Times(1.3),
                ));
                column = column.with_fixed(selectable(
                    &page.body, |t| t.text_secondary, None, Leading::Times(1.45),
                ));
                if page.more_before {
                    column = column.with_fixed(action_button(
                        "← Предыдущая часть текста".into(),
                        Action::NotionTextChunk(page.start_char.saturating_sub(TEXT_CHARS)), bindings,
                    ));
                }
                if page.more_after {
                    column = column.with_fixed(action_button(
                        "Следующая часть текста →".into(),
                        Action::NotionTextChunk(page.start_char + TEXT_CHARS), bindings,
                    ));
                }
                column = column.with_fixed(selectable(
                    "Текст из сохранённого Markdown, не полноценный рендер Notion. Данные только для чтения.",
                    |t| t.text_muted, None, Leading::Times(1.4),
                ));
            }
        }
    }
    NewWidget::new(column).with_props(PropertySet::new()
        .with(Gap::new(Length::px(size::GAP_LG)))
        .with(Padding::all(Length::px(size::GAP_XL))))
}
