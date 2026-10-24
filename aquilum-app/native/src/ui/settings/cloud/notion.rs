//! Offline-only native Notion source catalog.
//! No credentials, network, sync, or File Gate permissions live in this UI.
use super::*;

pub(super) fn body(ctx: &Context) -> NewWidget<Flex> {
    let mut column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(selectable(
            "Notion — локальные копии (только чтение)",
            |t| t.text_primary, None, Leading::Times(1.3),
        ))
        .with_fixed(selectable(
            "Это просмотр ранее сохранённых данных, не подключение аккаунта. Автообновление и запись в Notion здесь отключены.",
            |t| t.text_muted, None, Leading::Times(1.4),
        ));
    if let Some(reason) = ctx.notion_offline.error {
        column = column.with_fixed(selectable(
            &format!("Кэш требует проверки: {reason}"),
            |t| t.text_secondary, None, Leading::Times(1.4),
        ));
    } else if ctx.notion_offline.roots.is_empty() {
        column = column.with_fixed(selectable(
            "Локальных снимков пока нет. Первую загрузку выполняет отдельный тестовый Notion API-инструмент.",
            |t| t.text_secondary, None, Leading::Times(1.4),
        ));
    } else {
        for entry in &ctx.notion_offline.roots {
            column = column.with_fixed(selectable(
                &format!("Источник: {} · объектов: {} · связей: {}",
                    entry.id, entry.objects, entry.links),
                |t| t.text_primary, None, Leading::Times(1.4),
            ));
            for title in &entry.titles {
                column = column.with_fixed(selectable(
                    &format!("    {}", title),
                    |t| t.text_secondary, None, Leading::Times(1.4),
                ));
            }
        }
        column = column.with_fixed(selectable(
            "Показаны только названия первых страниц из локального каталога. Целостность полного содержимого и онлайн-доступ этим экраном не подтверждаются.",
            |t| t.text_muted, None, Leading::Times(1.4),
        ));
    }
    NewWidget::new(column).with_props(PropertySet::new()
        .with(Gap::new(Length::px(size::GAP_LG)))
        .with(Padding::all(Length::px(size::GAP_XL))))
}
