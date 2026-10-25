//! Opt-in Notion read-only connection and verified offline page browser.
//! HTTP and checksum work run on workers, not inside the UI event loop.
use super::*;
use crate::notion_offline::{LoadState, PAGE_BATCH, TEXT_CHARS};
mod bases;

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
            "Notion — подключение, просмотр и перенос в Акулу",
            |t| t.text_primary, None, Leading::Times(1.3),
        ))
        .with_fixed(selectable(
            "Notion — источник данных. Подключение и обновление только по запросу; импорт в выбранную базу Акулы — после отдельного подтверждения.",
            |t| t.text_muted, None, Leading::Times(1.4),
        ));
    if !ctx.notion_sync.busy {
        column = column.with_fixed(action_button(
            "Подключить Notion (UUID страницы и ключ в системном окне)".into(),
            Action::NotionConnect, bindings,
        ));
    }
    column = column.with_fixed(selectable(
        ctx.notion_sync.status.label(),
        |t| t.text_secondary, None, Leading::Times(1.4),
    ));
    let selected_root = ctx.notion_selection.root.as_ref()
        .or(ctx.notion_sync.root.as_ref());
    if let Some(root) = selected_root
        && !ctx.notion_sync.busy
    {
        column = column.with_fixed(action_button(
            "Обновить выбранный источник через Notion API".into(),
            Action::NotionRefresh(root.clone()), bindings,
        ));
        column = column.with_fixed(action_button(
            "Удалить сохранённый ключ (локальная копия останется)".into(),
            Action::NotionForget(root.clone()), bindings,
        ));
    }
    if let Some(reason) = ctx.notion_offline.error {
        column = column.with_fixed(selectable(
            &format!("Кэш требует проверки: {reason}"),
            |t| t.text_secondary, None, Leading::Times(1.4),
        ));
    } else if ctx.notion_offline.roots.is_empty() {
        column = column.with_fixed(selectable(
            "Локальной копии нет. Подключите Notion, затем дождитесь подтверждённой загрузки.",
            |t| t.text_secondary, None, Leading::Times(1.4),
        ));
    } else {
        for source in &ctx.notion_offline.roots {
            let chosen = ctx.notion_selection.root.as_deref() == Some(source.id.as_str());
            let label = format!("{} {} · объектов: {} · связей: {}",
                if chosen { "●" } else { "○" }, &source.id[..8],
                source.objects, source.links);
            column = column.with_fixed(action_button(
                label, Action::NotionOpenRoot(source.id.clone()), bindings,
            ));
        }
    }


    column=bases::append(column,ctx,bindings,selected_root);
    let status = &ctx.notion_selection;
    if let Some(root) = selected_root
        && status.state == LoadState::Ready
        && status.root.as_deref() == Some(root.as_str())
    {
        let importing = &ctx.notion_import;
        if let Some(workspace) = ctx.workspace.as_deref() {
            column = column.with_fixed(selectable(
                &format!("Целевая база Акулы: {}", workspace.display()),
                |t| t.text_secondary, None, Leading::Times(1.3),
            ));
            if !importing.busy && !ctx.notion_sync.busy {
                column = column.with_fixed(action_button(
                    "Проверить снимок для независимого импорта".into(),
                    Action::NotionReviewImport(root.clone()), bindings,
                ));
            }
            if importing.ready_for(root, workspace) && !ctx.notion_sync.busy {
                column = column.with_fixed(selectable(
                    &format!("Проверено: {} страниц, {} связей; снимок {}",
                        importing.pages, importing.links,
                        importing.digest.as_deref().unwrap_or("").chars().take(12).collect::<String>()),
                    |t| t.text_primary, None, Leading::Times(1.3),
                ));
                column = column.with_fixed(selectable(
                    &format!("Модель баз: {} источников, {} записей, {} представлений; проверить: {}",
                        importing.base_schemas, importing.base_rows,
                        importing.base_views, importing.base_issues),
                    |t| t.text_secondary, None, Leading::Times(1.3),
                ));
                column = column.with_fixed(action_button(
                    "Сохранить заметки, исходные JSON и граф ID в Акулу".into(),
                    Action::NotionPublishImport(root.clone()), bindings,
                ));
            }
        } else {
            column = column.with_fixed(selectable(
                "Для импорта выберите рабочую базу Акулы.",
                |t| t.text_secondary, None, Leading::Times(1.3),
            ));
        }
        if importing.root.as_deref() == Some(root.as_str()) {
            column = column.with_fixed(selectable(
                importing.stage.label(),
                |t| t.text_secondary, None, Leading::Times(1.3),
            ));
            if importing.created > 0 || importing.conflicts > 0 {
                column = column.with_fixed(selectable(
                    &format!("Создано: {}; прежние: {}; конфликты: {}; JSON: {}; граф ID: {}; модель: {}; проверить: {}",
                        importing.created, importing.existing, importing.conflicts,
                        if importing.structured_saved { "сохранён" } else { "не подтверждён" },
                        if importing.identities_saved { "сохранён" } else { "не подтверждён" },
                        if importing.bases_reconstructible { "читается офлайн" }
                            else { "не подтверждена" },
                        importing.identities_need_review + importing.base_issues),
                    |t| t.text_secondary, None, Leading::Times(1.3),
                ));
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notion_offline::{CachedRoot, LoadState, PageEntry};
    use crate::ui::settings::schema::Context;

    #[test]
    fn verified_api_root_exposes_explicit_import_only_for_selected_workspace() {
        let mut ctx = Context::new(1.0);
        let source = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned();
        let workspace = std::path::PathBuf::from("selected-vault");
        ctx.workspace = Some(workspace.clone());
        ctx.notion_selection.root = Some(source.clone());
        ctx.notion_selection.state = LoadState::Ready;
        ctx.notion_import.root = Some(source.clone());
        ctx.notion_import.target = Some(workspace.clone());
        ctx.notion_import.stage = crate::notion_import::Stage::Ready;
        ctx.notion_import.digest = Some("a".repeat(64));
        ctx.notion_import.pages = 2;
        ctx.notion_import.links = 1;
        let mut bindings = HashMap::new();
        let _ = body(&ctx, &mut bindings);
        assert!(bindings.values().any(|b| matches!(b,
            Binding::Button(Action::NotionReviewImport(id)) if *id == source)));
        assert!(bindings.values().any(|b| matches!(b,
            Binding::Button(Action::NotionPublishImport(id)) if *id == source)));

        ctx.workspace = Some(std::path::PathBuf::from("different-vault"));
        let mut other = HashMap::new();
        let _ = body(&ctx, &mut other);
        assert!(!other.values().any(|b|
            matches!(b, Binding::Button(Action::NotionPublishImport(_)))));
        ctx.workspace = Some(workspace);
        ctx.notion_import.busy = true;
        ctx.notion_import.stage = crate::notion_import::Stage::Publishing;
        let mut busy = HashMap::new();
        let _ = body(&ctx, &mut busy);
        assert!(!busy.values().any(|b|
            matches!(b, Binding::Button(Action::NotionPublishImport(_)))));
    }

    #[test]
    fn notion_source_and_page_buttons_are_read_only_selections() {
        let mut ctx = Context::new(1.0);
        let root = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned();
        let page = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_owned();
        ctx.notion_offline.roots.push(CachedRoot {
            id: root.clone(), objects: 4, links: 1, titles: vec!["Alpha".into()],
        });
        ctx.notion_selection.root = Some(root.clone());
        ctx.notion_selection.state = LoadState::Ready;
        ctx.notion_selection.page_count = 1;
        ctx.notion_selection.displayed_pages.push(PageEntry {
            id: page.clone(), title: "Alpha".into(),
        });
        let mut bindings = HashMap::new();
        let _widget = body(&ctx, &mut bindings);
        let actions = bindings.values().filter_map(|b| match b {
            Binding::Button(action) => Some(action),
            _ => None,
        }).collect::<Vec<_>>();
        assert!(actions.contains(&&Action::NotionOpenRoot(root)));
        assert!(actions.contains(&&Action::NotionOpenPage(page)));
        assert!(actions.contains(&&Action::NotionConnect));
        assert!(!actions.contains(&&Action::NotionReviewImport(
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into())));
        assert!(actions.contains(&&Action::NotionRefresh(
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string())));
        assert!(actions.iter().all(|action| matches!(action,
            Action::NotionConnect | Action::NotionRefresh(_) | Action::NotionForget(_)
            | Action::NotionOpenRoot(_) | Action::NotionOpenPage(_))));
    }
}
