mod group;

use super::ast::{Clause, Column, Direction, Expr, Query, Shape, Source};
use super::eval::{evaluate, note_object, Scope};
use super::rows::Row;
use super::value::Value;
use serde::Serialize;
use std::cmp::Ordering;
use std::rc::Rc;

use super::constants::{
    MAX_OUTPUT_ROWS as MAX_ROWS, NOTE_COLUMN, ROWS_IDENTIFIER as ROWS, TASK_DONE, TASK_LINE,
    TASK_TEXT,
};

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CellPart {
    Text { text: String },
    Link { target: String, text: String },
    Progress { percent: f64 },
    Check { done: bool, target: String, line: usize },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub parts: Vec<CellPart>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOutput {
    pub shape: &'static str,
    pub title: Option<String>,
    pub refresh_seconds: Option<u32>,
    pub columns: Vec<String>,
    pub rows: Vec<Cell>,
    pub width: usize,
    pub total: usize,
    pub truncated: bool,
}

struct Entry {
    note: Option<Rc<Row>>,
    vars: Vec<(String, Value)>,
}

impl Entry {
    fn scope(&self) -> Scope<'_> {
        Scope { note: self.note.as_deref(), vars: &self.vars, local: None }
    }

    fn var(&self, name: &str) -> Option<&Value> {
        self.vars.iter().find(|(key, _)| key == name).map(|(_, value)| value)
    }

    fn title_cell(&self) -> Cell {
        match &self.note {
            Some(note) => Cell {
                parts: vec![CellPart::Link {
                    target: note.relative.clone(),
                    text: note.name.clone(),
                }],
            },
            None => match self.vars.first() {
                Some((_, key)) => cell(key.clone()),
                None => cell(Value::Null),
            },
        }
    }
}

pub fn run(query: &Query, rows: Vec<Row>) -> Result<QueryOutput, String> {
    let entries: Vec<Entry> = if matches!(query.source, Some(Source::Once)) {
        vec![Entry { note: None, vars: Vec::new() }]
    } else {
        rows.into_iter()
            .map(|row| Entry { note: Some(Rc::new(row)), vars: Vec::new() })
            .collect()
    };
    let entries = if matches!(query.shape, Shape::Tasks { .. }) { tasks_of(entries) } else { entries };
    let mut entries = apply_clauses(query, entries)?;
    let total = entries.len();
    let truncated = total > MAX_ROWS;
    entries.truncate(MAX_ROWS);

    let mut output = match &query.shape {
        Shape::Table { columns, show_path } => {
            table(&entries, columns, *show_path, total, truncated)
        }
        Shape::List { value, show_path } => {
            list(&entries, value.as_ref(), *show_path, total, truncated)
        }
        Shape::Tasks { show_path } => Ok(task_list(&entries, *show_path, total, truncated)),
    }?;
    output.title = query.title.clone();
    output.refresh_seconds = query.refresh;
    Ok(output)
}

fn tasks_of(entries: Vec<Entry>) -> Vec<Entry> {
    let mut spread = Vec::new();
    for entry in entries {
        let Some(note) = entry.note.clone() else { continue };
        for task in &note.tasks {
            spread.push(Entry {
                note: Some(note.clone()),
                vars: vec![
                    (TASK_TEXT.to_owned(), Value::Text(task.text.clone())),
                    (TASK_DONE.to_owned(), Value::Bool(task.done)),
                    (TASK_LINE.to_owned(), Value::Number(task.line as f64)),
                ],
            });
        }
    }
    spread
}

fn task_list(entries: &[Entry], show_path: bool, total: usize, truncated: bool) -> QueryOutput {
    let mut cells = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut parts = vec![CellPart::Check {
            done: entry.var(TASK_DONE).is_some_and(|value| value.truthy()),
            target: entry.note.as_ref().map(|note| note.relative.clone()).unwrap_or_default(),
            line: entry.var(TASK_LINE).and_then(|value| value.as_number()).unwrap_or(0.0) as usize,
        }];
        let text = entry.var(TASK_TEXT).map(|value| value.text()).unwrap_or_default();
        match entry.note.as_ref().filter(|_| show_path) {
            Some(note) => parts.push(CellPart::Link {
                target: note.relative.clone(),
                text,
            }),
            None => parts.push(CellPart::Text { text }),
        }
        cells.push(Cell { parts });
    }
    QueryOutput {
        shape: "tasks",
        title: None,
        refresh_seconds: None,
        columns: Vec::new(),
        rows: cells,
        width: 1,
        total,
        truncated,
    }
}

fn apply_clauses(query: &Query, mut entries: Vec<Entry>) -> Result<Vec<Entry>, String> {
    for clause in &query.clauses {
        match clause {
            Clause::Where(condition) => {
                let mut kept = Vec::with_capacity(entries.len());
                for entry in entries {
                    if evaluate(condition, &entry.scope())?.truthy() {
                        kept.push(entry);
                    }
                }
                entries = kept;
            }
            Clause::Sort(keys) => {
                let mut keyed = Vec::with_capacity(entries.len());
                for entry in entries {
                    let mut values = Vec::with_capacity(keys.len());
                    {
                        let scope = entry.scope();
                        for key in keys {
                            values.push(evaluate(&key.value, &scope)?);
                        }
                    }
                    keyed.push((values, entry));
                }
                keyed.sort_by(|left, right| {
                    for (index, key) in keys.iter().enumerate() {
                        let (left, right) = (&left.0[index], &right.0[index]);
                        if let Some(ordering) = empty_last(left, right) {
                            return ordering;
                        }
                        let ordering = match key.direction {
                            Direction::Ascending => left.compare(right),
                            Direction::Descending => left.compare(right).reverse(),
                        };
                        if !ordering.is_eq() {
                            return ordering;
                        }
                    }
                    Ordering::Equal
                });
                entries = keyed.into_iter().map(|(_, entry)| entry).collect();
            }
            Clause::Limit(limit) => entries.truncate(*limit),
            Clause::GroupBy { value, name } => entries = group::group(entries, value, name)?,
            Clause::Flatten { value, name } => entries = flatten(entries, value, name)?,
        }
    }
    Ok(entries)
}

fn flatten(entries: Vec<Entry>, value: &Expr, name: &str) -> Result<Vec<Entry>, String> {
    let mut spread = Vec::with_capacity(entries.len());
    for entry in entries {
        let computed = evaluate(value, &entry.scope())?;
        let items = match computed {
            Value::List(items) => items,
            single => vec![single],
        };
        for item in items {
            let mut vars = entry.vars.clone();
            match vars.iter_mut().find(|(key, _)| key == name) {
                Some((_, existing)) => *existing = item,
                None => vars.push((name.to_owned(), item)),
            }
            spread.push(Entry { note: entry.note.clone(), vars });
        }
    }
    Ok(spread)
}

fn empty_last(left: &Value, right: &Value) -> Option<Ordering> {
    match (matches!(left, Value::Null), matches!(right, Value::Null)) {
        (true, false) => Some(Ordering::Greater),
        (false, true) => Some(Ordering::Less),
        _ => None,
    }
}

fn table(
    entries: &[Entry],
    columns: &[Column],
    show_path: bool,
    total: usize,
    truncated: bool,
) -> Result<QueryOutput, String> {
    let show_note = show_path || columns.is_empty();
    let mut titles = Vec::with_capacity(columns.len() + 1);
    if show_note {
        titles.push(NOTE_COLUMN.to_owned());
    }
    titles.extend(columns.iter().map(|column| column.title.clone()));

    let width = titles.len();
    let mut cells = Vec::with_capacity(entries.len() * width);
    for entry in entries {
        if show_note {
            cells.push(entry.title_cell());
        }
        let scope = entry.scope();
        for column in columns {
            cells.push(cell(evaluate(&column.value, &scope)?));
        }
    }

    Ok(QueryOutput {
        shape: "table",
        title: None,
        refresh_seconds: None,
        columns: titles,
        rows: cells,
        width,
        total,
        truncated,
    })
}

fn list(
    entries: &[Entry],
    value: Option<&Expr>,
    show_path: bool,
    total: usize,
    truncated: bool,
) -> Result<QueryOutput, String> {
    let mut cells = Vec::with_capacity(entries.len());
    for entry in entries {
        cells.push(match value {
            Some(expression) => {
                let computed = cell(evaluate(expression, &entry.scope())?);
                if show_path {
                    let mut parts = entry.title_cell().parts;
                    parts.push(CellPart::Text { text: ": ".to_owned() });
                    parts.extend(computed.parts);
                    Cell { parts }
                } else {
                    computed
                }
            }
            None => entry.title_cell(),
        });
    }
    Ok(QueryOutput {
        shape: "list",
        title: None,
        refresh_seconds: None,
        columns: Vec::new(),
        rows: cells,
        width: 1,
        total,
        truncated,
    })
}

fn cell(value: Value) -> Cell {
    let mut parts = Vec::new();
    push_parts(&value, &mut parts);
    if parts.is_empty() {
        parts.push(CellPart::Text { text: String::new() });
    }
    Cell { parts }
}

fn push_parts(value: &Value, parts: &mut Vec<CellPart>) {
    match value {
        Value::Link(link) => parts.push(CellPart::Link {
            target: link.target.clone(),
            text: link.label(),
        }),
        Value::Progress(percent) => parts.push(CellPart::Progress { percent: *percent }),
        Value::List(items) => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    parts.push(CellPart::Text { text: ", ".to_owned() });
                }
                push_parts(item, parts);
            }
        }
        other => parts.push(CellPart::Text { text: other.text() }),
    }
}
