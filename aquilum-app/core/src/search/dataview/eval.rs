use super::ast::{BinaryOp, Expr, UnaryOp};
use super::constants::FILE_IDENTIFIER;
use super::duration;
use super::functions;
use super::rows::Row;
use super::span;
use super::value::{Link, Value};
use crate::search::fields::FieldKind;
use crate::search::note_date;

pub struct Local<'a> {
    pub name: &'a str,
    pub value: &'a Value,
    pub outer: Option<&'a Local<'a>>,
}

pub struct Scope<'a> {
    pub note: Option<&'a Row>,
    pub vars: &'a [(String, Value)],
    pub local: Option<&'a Local<'a>>,
}

impl<'a> Scope<'a> {
    fn inner(&'a self, local: &'a Local<'a>) -> Scope<'a> {
        Scope { note: self.note, vars: self.vars, local: Some(local) }
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        let mut local = self.local;
        while let Some(found) = local {
            if found.name.eq_ignore_ascii_case(name) {
                return Some(found.value.clone());
            }
            local = found.outer;
        }
        if let Some((_, value)) = self.vars.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)) {
            return Some(value.clone());
        }
        self.note.and_then(|note| note.field(name)).map(field_value)
    }
}

pub fn evaluate(expr: &Expr, scope: &Scope<'_>) -> Result<Value, String> {
    match expr {
        Expr::Literal(value) => Ok(value.clone()),
        Expr::Variable(name) => Ok(variable(name, scope)),
        Expr::Property { base, name } => property(base, name, scope),
        Expr::Index { base, index } => {
            let base = evaluate(base, scope)?;
            let index = evaluate(index, scope)?;
            Ok(at_index(&base, &index))
        }
        Expr::Call { name, arguments } => call(name, arguments, scope),
        Expr::Unary { operator, operand } => {
            let value = evaluate(operand, scope)?;
            Ok(match operator {
                UnaryOp::Not => Value::Bool(!value.truthy()),
                UnaryOp::Negate => match value {
                    Value::Duration(length) => Value::Duration(length.negate()),
                    other => match other.as_number() {
                        Some(number) => Value::Number(-number),
                        None => Value::Null,
                    },
                },
            })
        }
        Expr::Binary { operator, left, right } => binary(*operator, left, right, scope),
        Expr::Lambda { .. } => Err(
            "Стрелочная функция бывает только аргументом «filter», «map», «any» или «all»".to_owned(),
        ),
    }
}

pub fn note_object(row: &Row) -> Value {
    let mut fields = vec![(
        FILE_IDENTIFIER.to_owned(),
        Value::Object(vec![
            ("link".to_owned(), note_link(row)),
            ("path".to_owned(), Value::Text(row.relative.clone())),
            ("name".to_owned(), Value::Text(row.name.clone())),
            ("folder".to_owned(), Value::Text(row.folder.clone())),
            ("ctime".to_owned(), Value::Date(row.created)),
            ("mtime".to_owned(), Value::Date(row.modified)),
            ("size".to_owned(), Value::Number(row.size as f64)),
        ]),
    )];
    fields.extend(row.fields.iter().map(|field| (field.key.clone(), field_value(field))));
    Value::Object(fields)
}

fn variable(name: &str, scope: &Scope<'_>) -> Value {
    if name.eq_ignore_ascii_case(FILE_IDENTIFIER) {
        return scope.note.map(note_link).unwrap_or(Value::Null);
    }
    if let Some(value) = scope.lookup(name) {
        return value;
    }
    if name.eq_ignore_ascii_case("today") {
        return Value::Date(note_date::start_of_day(note_date::now()));
    }
    if name.eq_ignore_ascii_case("now") {
        return Value::Date(note_date::now());
    }
    if let Some(period) = span::builtin(name) {
        return period;
    }
    Value::Null
}

fn field_value(field: &crate::search::fields::Field) -> Value {
    match field.kind {
        FieldKind::List => {
            Value::List(field.items.iter().cloned().map(Value::Text).collect::<Vec<_>>())
        }
        FieldKind::Number => match field.text.trim().parse::<f64>() {
            Ok(number) => Value::Number(number),
            Err(_) => Value::Text(field.text.clone()),
        },
        FieldKind::Bool => Value::Bool(field.text.trim().eq_ignore_ascii_case("true")),
        FieldKind::Text => {
            if field.text.trim().is_empty() {
                return Value::Null;
            }
            match note_date::parse(field.text.trim()) {
                Some(nanos) => Value::Date(nanos),
                None => Value::Text(field.text.clone()),
            }
        }
    }
}

fn property(base: &Expr, name: &str, scope: &Scope<'_>) -> Result<Value, String> {
    let file_of_note = matches!(base, Expr::Variable(found) if found.eq_ignore_ascii_case(FILE_IDENTIFIER))
        && scope.lookup(FILE_IDENTIFIER).is_none();
    if file_of_note {
        if let Some(note) = scope.note {
            return file_property(name, note);
        }
    }
    let value = evaluate(base, scope)?;
    Ok(member(&value, name))
}

// Built-in property names are ASCII, but Unicode case conversion can produce
// ASCII (e.g. U+212A KELVIN SIGN). Preserve full str::to_lowercase for those
// uncommon inputs without allocating for the common ASCII property names.
fn builtin_property_name(name: &str) -> std::borrow::Cow<'_, str> {
    if name.is_ascii() {
        std::borrow::Cow::Borrowed(name)
    } else {
        std::borrow::Cow::Owned(name.to_lowercase())
    }
}

fn member(value: &Value, name: &str) -> Value {
    let property = builtin_property_name(name);
    match value {
        Value::Link(link) if property.eq_ignore_ascii_case("path") => Value::Text(link.target.clone()),
        Value::Link(link) if property.eq_ignore_ascii_case("display") => match &link.display {
            Some(display) => Value::Text(display.clone()),
            None => Value::Null,
        },
        Value::List(items) if property.eq_ignore_ascii_case("length") => Value::Number(items.len() as f64),
        Value::List(items) => Value::List(items.iter().map(|item| member(item, name)).collect()),
        // Object member resolution deliberately retains exact-first key priority.
        Value::Object(fields) => fields
            .iter()
            .find(|(key, _)| key == name)
            .or_else(|| fields.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)))
            .map(|(_, found)| found.clone())
            .unwrap_or(Value::Null),
        Value::Date(nanos) if property.eq_ignore_ascii_case("year") => {
            Value::Number(note_date::to_civil(*nanos).0 as f64)
        }
        Value::Date(nanos) if property.eq_ignore_ascii_case("month") => {
            Value::Number(note_date::to_civil(*nanos).1 as f64)
        }
        Value::Date(nanos) if property.eq_ignore_ascii_case("day") => {
            Value::Number(note_date::to_civil(*nanos).2 as f64)
        }
        Value::Date(nanos) if property.eq_ignore_ascii_case("weekday") => {
            Value::Number(note_date::weekday(*nanos) as f64)
        }
        _ => Value::Null,
    }
}

fn file_property(name: &str, row: &Row) -> Result<Value, String> {
    let property = builtin_property_name(name);
    let value = if property.eq_ignore_ascii_case("link") {
        note_link(row)
    } else if property.eq_ignore_ascii_case("path") {
        Value::Text(row.relative.clone())
    } else if property.eq_ignore_ascii_case("name") {
        Value::Text(row.name.clone())
    } else if property.eq_ignore_ascii_case("folder") {
        Value::Text(row.folder.clone())
    } else if property.eq_ignore_ascii_case("ctime") || property.eq_ignore_ascii_case("cday") {
        Value::Date(row.created)
    } else if property.eq_ignore_ascii_case("mtime") || property.eq_ignore_ascii_case("mday") {
        Value::Date(row.modified)
    } else if property.eq_ignore_ascii_case("size") {
        Value::Number(row.size as f64)
    } else if property.eq_ignore_ascii_case("tags") {
        match row.field("tags") {
            Some(field) => field_value(field),
            None => Value::List(Vec::new()),
        }
    } else {
        // Keep the original lowercase diagnostic for unknown properties.
        let other = name.to_lowercase();
        return Err(format!(
            "У файла нет поля «{other}»: есть link, path, name, folder, ctime, mtime, size, tags"
        ));
    };
    Ok(value)
}

fn note_link(row: &Row) -> Value {
    Value::Link(Link { target: row.relative.clone(), display: None })
}

fn at_index(base: &Value, index: &Value) -> Value {
    match (base, index) {
        (Value::List(items), Value::Number(position)) => {
            let position = *position;
            if position < 0.0 {
                return Value::Null;
            }
            items.get(position as usize).cloned().unwrap_or(Value::Null)
        }
        (Value::Object(_), Value::Text(name)) => member(base, name),
        _ => Value::Null,
    }
}

fn call(name: &str, arguments: &[Expr], scope: &Scope<'_>) -> Result<Value, String> {
    if let Some(Expr::Lambda { parameters, body }) =
        arguments.iter().find(|argument| matches!(argument, Expr::Lambda { .. }))
    {
        let over = match arguments.first() {
            Some(first) => evaluate(first, scope)?,
            None => Value::Null,
        };
        let items = match over {
            Value::List(items) => items,
            Value::Null => Vec::new(),
            single => vec![single],
        };
        return apply_lambda(name, parameters, body, items, scope);
    }

    let mut values = Vec::with_capacity(arguments.len());
    for argument in arguments {
        values.push(evaluate(argument, scope)?);
    }
    functions::call(name, values)
}

fn apply_lambda(
    name: &str,
    parameters: &[String],
    body: &Expr,
    items: Vec<Value>,
    scope: &Scope<'_>,
) -> Result<Value, String> {
    let Some(parameter) = parameters.first() else {
        return Err(format!("У стрелочной функции в «{name}» нет параметра"));
    };
    let mut results = Vec::with_capacity(items.len());
    for item in &items {
        let local = Local { name: parameter, value: item, outer: scope.local };
        results.push(evaluate(body, &scope.inner(&local))?);
    }

    Ok(match name {
        "filter" => Value::List(
            items
                .into_iter()
                .zip(results)
                .filter(|(_, kept)| kept.truthy())
                .map(|(item, _)| item)
                .collect(),
        ),
        "map" => Value::List(results),
        "any" => Value::Bool(results.iter().any(Value::truthy)),
        "all" => Value::Bool(results.iter().all(Value::truthy)),
        other => {
            return Err(format!(
                "Функция «{other}» не принимает стрелочную функцию: её ждут filter, map, any, all"
            ))
        }
    })
}

fn binary(operator: BinaryOp, left: &Expr, right: &Expr, scope: &Scope<'_>) -> Result<Value, String> {
    if operator == BinaryOp::And {
        let left = evaluate(left, scope)?;
        return Ok(Value::Bool(left.truthy() && evaluate(right, scope)?.truthy()));
    }
    if operator == BinaryOp::Or {
        let left = evaluate(left, scope)?;
        return Ok(Value::Bool(left.truthy() || evaluate(right, scope)?.truthy()));
    }

    let left = evaluate(left, scope)?;
    let right = evaluate(right, scope)?;
    Ok(match operator {
        BinaryOp::Equal => Value::Bool(left.equals(&right)),
        BinaryOp::NotEqual => Value::Bool(!left.equals(&right)),
        BinaryOp::Less => Value::Bool(left.compare(&right).is_lt()),
        BinaryOp::LessOrEqual => Value::Bool(left.compare(&right).is_le()),
        BinaryOp::Greater => Value::Bool(left.compare(&right).is_gt()),
        BinaryOp::GreaterOrEqual => Value::Bool(left.compare(&right).is_ge()),
        BinaryOp::Add => add(left, right),
        BinaryOp::Subtract => subtract(left, right),
        BinaryOp::Multiply => multiply(left, right),
        BinaryOp::Divide => arithmetic(left, right, |l, r| l / r),
        BinaryOp::Modulo => arithmetic(left, right, |l, r| l % r),
        BinaryOp::And | BinaryOp::Or => unreachable!("обработаны выше"),
    })
}

fn add(left: Value, right: Value) -> Value {
    match (&left, &right) {
        (Value::Date(moment), Value::Duration(length))
        | (Value::Duration(length), Value::Date(moment)) => {
            Value::Date(duration::shift(*moment, *length))
        }
        (Value::Duration(first), Value::Duration(second)) => Value::Duration(first.add(*second)),
        (Value::List(left), Value::List(right)) => {
            let mut items = left.clone();
            items.extend(right.iter().cloned());
            Value::List(items)
        }
        (Value::Text(_), _) | (_, Value::Text(_)) => {
            Value::Text(format!("{}{}", left.text(), right.text()))
        }
        _ => arithmetic(left, right, |l, r| l + r),
    }
}

fn subtract(left: Value, right: Value) -> Value {
    match (&left, &right) {
        (Value::Date(moment), Value::Duration(length)) => {
            Value::Date(duration::shift(*moment, length.negate()))
        }
        (Value::Date(later), Value::Date(earlier)) => {
            Value::Duration(duration::between(*later, *earlier))
        }
        (Value::Duration(first), Value::Duration(second)) => {
            Value::Duration(first.add(second.negate()))
        }
        _ => arithmetic(left, right, |l, r| l - r),
    }
}

fn multiply(left: Value, right: Value) -> Value {
    match (&left, &right) {
        (Value::Duration(length), other) | (other, Value::Duration(length)) => {
            match other.as_number() {
                Some(factor) => Value::Duration(length.scale(factor)),
                None => Value::Null,
            }
        }
        _ => arithmetic(left, right, |l, r| l * r),
    }
}

fn arithmetic(left: Value, right: Value, apply: fn(f64, f64) -> f64) -> Value {
    match (left.as_number(), right.as_number()) {
        (Some(left), Some(right)) => Value::Number(apply(left, right)),
        _ => Value::Null,
    }
}
