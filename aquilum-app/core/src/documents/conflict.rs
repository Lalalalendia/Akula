use crate::search::note_date::to_civil_at;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictCause {
    Diverged,
    NoSyncPoint,
    Displaced,
    Truncated,
    Reverted,
}

struct Explanation {
    title: &'static str,
    why: &'static str,
}

fn explanation(cause: ConflictCause) -> Explanation {
    match cause {
        ConflictCause::Diverged => Explanation {
            title: "файл и документ изменились одновременно",
            why: "И файл на диске, и открытый в редакторе документ изменились после последней сверки. \
                  Свести их без потери было нельзя, поэтому в заметке осталась версия с диска, \
                  а версия из редактора сохранена здесь.",
        },
        ConflictCause::NoSyncPoint => Explanation {
            title: "точка сверки недоступна",
            why: "Хранилище точек синхронизации не ответило, поэтому сравнить версии было нечем. \
                  Приложение не стало молча выбирать сторону: в заметке осталась версия с диска, \
                  а версия из редактора сохранена здесь.",
        },
        ConflictCause::Displaced => Explanation {
            title: "строки не поместились при слиянии",
            why: "В заметку приехала внешняя правка, и её сведение с тем, что набиралось в редакторе, \
                  вытеснило строки ниже. Остальной текст заметки цел.",
        },
        ConflictCause::Reverted => Explanation {
            title: "изменения версии отменены поверх более поздних правок",
            why: "Отмена изменений одной версии из истории задела строки, которые правились позже. \
                  В заметке эти места вернулись к виду до той версии, а более поздний текст этих строк \
                  сохранён здесь.",
        },
        ConflictCause::Truncated => Explanation {
            title: "заметка обнулена при сохранении",
            why: "Сохранение записало в заметку пустой текст, хотя до этого в ней был текст. \
                  Прежнее содержимое сохранено здесь до того, как файл был обнулён. \
                  Если вы очистили заметку сами, эту копию можно просто удалить.",
        },
    }
}

pub struct ConflictReport {
    pub cause: ConflictCause,
    pub disk_hash: Option<String>,
    pub synced_hash: Option<String>,
}

pub struct LocalMinute {
    pub date: String,
    pub time: String,
}

impl LocalMinute {
    pub fn at(unix_nanos: i64, offset_seconds: i64) -> Self {
        let (year, month, day, inside) = to_civil_at(unix_nanos, offset_seconds);
        Self {
            date: format!("{year:04}-{month:02}-{day:02}"),
            time: format!("{:02}:{:02}", inside / 3600, (inside % 3600) / 60),
        }
    }
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn copy_stem(stem: &str, moment: &LocalMinute) -> String {
    format!("{stem} (конфликт {} {})", moment.date, moment.time.replace(':', "-"))
}

pub fn copy_content(
    note_path: &str,
    stem: &str,
    report: &ConflictReport,
    moment: &LocalMinute,
    body: &str,
) -> Option<String> {
    // Whitespace is user data too; only an actually empty body can be omitted.
    if body.is_empty() {
        return None;
    }
    let explanation = explanation(report.cause);
    let stamp = format!("{} {}", moment.date, moment.time);
    let mut lines = vec![
        "---".to_owned(),
        format!("конфликт: {stamp}"),
        format!("заметка: {}", quoted(stem)),
        format!("причина: {}", quoted(explanation.title)),
        "---".to_owned(),
        String::new(),
        format!("> Это конфликтная копия. Текст ниже не попал в заметку «{stem}» — он сохранён здесь, чтобы не пропасть."),
        ">".to_owned(),
        format!("> **Почему.** {}", explanation.why),
        ">".to_owned(),
        format!("> **Когда.** {stamp}"),
        format!("> **Заметка.** `{note_path}`"),
    ];
    if let Some(hash) = &report.disk_hash {
        lines.push(format!("> **Отпечаток файла на диске.** `{}`", one_line(hash)));
    }
    if let Some(hash) = &report.synced_hash {
        lines.push(format!("> **Отпечаток последней сверки.** `{}`", one_line(hash)));
    }
    lines.extend([String::new(), "## Текст, который не попал в заметку".to_owned(), String::new()]);
    let mut content = lines.join("\n");
    content.push_str(body);
    if !body.ends_with('\n') {
        content.push('\n');
    }
    Some(content)
}

#[cfg(test)]
#[path = "conflict_tests.rs"]
mod tests;
