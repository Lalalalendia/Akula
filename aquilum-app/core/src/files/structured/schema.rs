//! JSON Canvas 1.0 structural validation at the untrusted file boundary.
//! Unknown properties and unknown node kinds remain in the original JSON.
//! This validator does not dereference file nodes or load external URLs.
use std::collections::HashSet;

use serde_json::{Map, Value};

use super::StructuredError;

fn invalid(reason: impl Into<String>) -> StructuredError {
    StructuredError::InvalidFormat(reason.into())
}

fn optional_array<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a [Value], StructuredError> {
    match value {
        None => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        _ => Err(invalid(format!("JSON Canvas {name} must be an array"))),
    }
}

fn object<'a>(value: &'a Value, name: &str) -> Result<&'a Map<String, Value>, StructuredError> {
    value
        .as_object()
        .ok_or_else(|| invalid(format!("JSON Canvas {name} must be an object")))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    at: &str,
) -> Result<&'a str, StructuredError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("JSON Canvas {at}.{name} must be a string")))
}

fn unique_id<'a>(
    object: &'a Map<String, Value>,
    seen: &mut HashSet<&'a str>,
    at: &str,
) -> Result<&'a str, StructuredError> {
    let id = required_string(object, "id", at)?;
    if id.is_empty() || !seen.insert(id) {
        return Err(invalid(format!("JSON Canvas {at} has an empty or duplicate id: {id}")));
    }
    Ok(id)
}

fn optional_string(object: &Map<String, Value>, name: &str, at: &str) -> Result<(), StructuredError> {
    if object.contains_key(name) {
        required_string(object, name, at)?;
    }
    Ok(())
}

fn optional_enum(
    object: &Map<String, Value>,
    name: &str,
    variants: &[&str],
    at: &str,
) -> Result<(), StructuredError> {
    if object.contains_key(name) {
        let value = required_string(object, name, at)?;
        if !variants.contains(&value) {
            return Err(invalid(format!("JSON Canvas {at}.{name} has invalid value: {value}")));
        }
    }
    Ok(())
}

fn canvas_color(object: &Map<String, Value>, at: &str) -> Result<(), StructuredError> {
    if let Some(color) = object.get("color") {
        let value = color
            .as_str()
            .ok_or_else(|| invalid(format!("JSON Canvas {at}.color must be a string")))?;
        let preset = matches!(value, "1" | "2" | "3" | "4" | "5" | "6");
        let hex = value.strip_prefix('#').is_some_and(|hex| {
            matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        if !preset && !hex {
            return Err(invalid(format!("JSON Canvas {at}.color is not a valid canvasColor")));
        }
    }
    Ok(())
}

fn geometry_number(object: &Map<String, Value>, key: &str, at: &str) -> Result<(), StructuredError> {
    let number = object
        .get(key)
        .filter(|value| value.is_number())
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| invalid(format!("JSON Canvas {at}.{key} must be a finite number")))?;
    if matches!(key, "width" | "height") && number < 0.0 {
        return Err(invalid(format!("JSON Canvas {at}.{key} must not be negative")));
    }
    Ok(())
}

fn nodes<'a>(items: &'a [Value]) -> Result<HashSet<&'a str>, StructuredError> {
    let mut ids = HashSet::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = format!("nodes[{index}]");
        let object = object(item, &at)?;
        unique_id(object, &mut ids, &at)?;
        let kind = required_string(object, "type", &at)?;
        for key in ["x", "y", "width", "height"] {
            geometry_number(object, key, &at)?;
        }
        canvas_color(object, &at)?;
        match kind {
            "text" => {
                required_string(object, "text", &at)?;
            }
            "file" => {
                required_string(object, "file", &at)?;
                if let Some(subpath) = object.get("subpath") {
                    if !subpath.as_str().is_some_and(|value| value.starts_with('#')) {
                        return Err(invalid(format!("JSON Canvas {at}.subpath must start with '#'")));
                    }
                }
            }
            "link" => {
                required_string(object, "url", &at)?;
            }
            "group" => {
                optional_string(object, "label", &at)?;
                optional_string(object, "background", &at)?;
                optional_enum(object, "backgroundStyle", &["cover", "ratio", "repeat"], &at)?;
            }
            _ => {} // Preserve future/custom node types for a safe UI placeholder.
        }
    }
    Ok(ids)
}

fn edges(items: &[Value], nodes: &HashSet<&str>) -> Result<(), StructuredError> {
    let mut ids = HashSet::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = format!("edges[{index}]");
        let object = object(item, &at)?;
        unique_id(object, &mut ids, &at)?;
        for field in ["fromNode", "toNode"] {
            let node_id = required_string(object, field, &at)?;
            if !nodes.contains(node_id) {
                return Err(invalid(format!("JSON Canvas {at}.{field} references missing node: {node_id}")));
            }
        }
        for field in ["fromSide", "toSide"] {
            optional_enum(object, field, &["top", "right", "bottom", "left"], &at)?;
        }
        for field in ["fromEnd", "toEnd"] {
            optional_enum(object, field, &["none", "arrow"], &at)?;
        }
        optional_string(object, "label", &at)?;
        canvas_color(object, &at)?;
    }
    Ok(())
}

/// Lossless read compatibility: older files with finite floating point
/// geometry remain readable, but are not silently rounded and rewritten.
pub(super) fn has_legacy_geometry(value: &Value) -> bool {
    value
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|node| {
            ["x", "y", "width", "height"].into_iter().any(|field| {
                node.get(field).is_some_and(|value| {
                    value.is_number() && value.as_i64().is_none() && value.as_u64().is_none()
                })
            })
        })
}

pub(super) fn validate_canvas(text: &str) -> Result<Value, StructuredError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| invalid(format!("invalid JSON Canvas: {error}")))?;
    let root = object(&value, "root")?;
    let node_ids = nodes(optional_array(root.get("nodes"), "nodes")?)?;
    edges(optional_array(root.get("edges"), "edges")?, &node_ids)?;
    // The ID set borrows from `value`; release it before returning the Value.
    drop(node_ids);
    Ok(value)
}
