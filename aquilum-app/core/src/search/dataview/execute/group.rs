use std::collections::HashMap;

use super::{evaluate, note_object, Entry, Expr, Value, ROWS};

const INDEX_AFTER_UNIQUE_GROUPS: usize = 32;

#[derive(Default)]
struct Groups {
    values: Vec<(Value, Vec<Value>)>,
    by_text: Option<HashMap<String, Vec<usize>>>,
}

fn folded(value: &Value) -> String {
    value.text().to_lowercase()
}

impl Groups {
    fn push(&mut self, key: Value, item: Value) {
        // A small GROUP BY (including the home dashboard's one-group summary)
        // stays on the original linear path without allocating a hash index.
        if self.by_text.is_none() && self.values.len() >= INDEX_AFTER_UNIQUE_GROUPS {
            let mut index: HashMap<String, Vec<usize>> = HashMap::with_capacity(self.values.len());
            for (position, (existing, _)) in self.values.iter().enumerate() {
                index.entry(folded(existing)).or_default().push(position);
            }
            self.by_text = Some(index);
        }

        // The folded display text is only a candidate filter: Value::equals
        // uses typed equality for numbers/dates/lists and cross-type text for
        // other cases. Preserve the first matching key in insertion order.
        let found = if let Some(index) = self.by_text.as_ref() {
            let signature = folded(&key);
            index.get(&signature).and_then(|positions| {
                positions.iter().copied().find(|&position| self.values[position].0.equals(&key))
            })
        } else {
            self.values.iter().position(|(existing, _)| existing.equals(&key))
        };

        match found {
            Some(position) => self.values[position].1.push(item),
            None => {
                if let Some(index) = self.by_text.as_mut() {
                    index.entry(folded(&key)).or_default().push(self.values.len());
                }
                self.values.push((key, vec![item]));
            }
        }
    }

    fn entries(self, name: &str) -> Vec<Entry> {
        self.values
            .into_iter()
            .map(|(key, items)| Entry {
                note: None,
                vars: vec![(name.to_owned(), key), (ROWS.to_owned(), Value::List(items))],
            })
            .collect()
    }
}

pub(super) fn group(entries: Vec<Entry>, value: &Expr, name: &str) -> Result<Vec<Entry>, String> {
    let mut groups = Groups::default();
    for entry in entries {
        let key = evaluate(value, &entry.scope())?;
        let item = match &entry.note {
            Some(note) => note_object(note),
            None => Value::Object(entry.vars.clone()),
        };
        groups.push(key, item);
    }
    Ok(groups.entries(name))
}

#[cfg(test)]
mod tests;
