//! Local JSON Canvas operation session. No transport, no UI and no background writes.
//! The original disk revision stays owned by StructuredDocument/File Gate.
use std::fmt;
use std::path::Path;

use serde_json::{Value, json};

use crate::app_core::Core;
use super::{StructuredDocument, StructuredError};

const HISTORY_LIMIT: usize = 64;
const EDIT_COORD_LIMIT: i64 = 100_000_000;

#[derive(Debug)]
pub enum CanvasSessionError {
    Document(StructuredError),
    MissingNode(String),
    NotTextNode(String),
    CoordinateOutOfRange,
}

impl From<StructuredError> for CanvasSessionError {
    fn from(value: StructuredError) -> Self { Self::Document(value) }
}

impl fmt::Display for CanvasSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => write!(f, "{error}"),
            Self::MissingNode(id) => write!(f, "Canvas node does not exist: {id}"),
            Self::NotTextNode(id) => write!(f, "Canvas node is not text: {id}"),
            Self::CoordinateOutOfRange => f.write_str("Canvas position is outside the supported editable range"),
        }
    }
}

impl std::error::Error for CanvasSessionError {}

/// Per-document state that must outlive a Canvas widget/tab reconstruction.
/// Undo and redo are full validated JSON snapshots in this first local editor;
/// unknown vendor properties remain untouched by object-level operations.
pub struct CanvasSession {
    document: StructuredDocument,
    value: Value,
    saved: Value,
    undo: Vec<Value>,
    redo: Vec<Value>,
}

impl CanvasSession {
    /// Caller must authorize the path against the open vault before opening it.
    pub fn open(path: &Path) -> Result<Self, CanvasSessionError> {
        let document = StructuredDocument::open(path)?;
        let value = document.canvas()?;
        Ok(Self {
            document,
            saved: value.clone(),
            value,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    /// When an inactive, clean tab is reopened, detect external changes using
    /// the full source digest, not size or modification time.
    pub fn refresh_if_clean(&mut self) -> Result<bool, CanvasSessionError> {
        if self.dirty() { return Ok(false); }
        let latest = StructuredDocument::open(&self.document.path)?;
        if latest.original_hash == self.document.original_hash { return Ok(false); }
        let value = latest.canvas()?;
        self.document = latest;
        self.saved = value.clone();
        self.value = value;
        self.undo.clear();
        self.redo.clear();
        Ok(true)
    }

    pub fn value(&self) -> &Value { &self.value }
    pub fn dirty(&self) -> bool { self.value != self.saved }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }

    fn stage(&mut self, next: Value) -> Result<bool, CanvasSessionError> {
        if next == self.value { return Ok(false); }
        // Validate and check legacy lossless-writing restrictions BEFORE
        // publishing the new value or modifying either history stack.
        self.document.replace_canvas(&next)?;
        if self.undo.len() == HISTORY_LIMIT { self.undo.remove(0); }
        self.undo.push(std::mem::replace(&mut self.value, next));
        self.redo.clear();
        Ok(true)
    }

    fn node_mut<'a>(value: &'a mut Value, id: &str) -> Result<&'a mut Value, CanvasSessionError> {
        value.get_mut("nodes")
            .and_then(Value::as_array_mut)
            .and_then(|nodes| nodes.iter_mut().find(|node| node.get("id").and_then(Value::as_str) == Some(id)))
            .ok_or_else(|| CanvasSessionError::MissingNode(id.to_owned()))
    }

    fn edit_coordinates(x: i64, y: i64) -> Result<(), CanvasSessionError> {
        if !(-EDIT_COORD_LIMIT..=EDIT_COORD_LIMIT).contains(&x)
            || !(-EDIT_COORD_LIMIT..=EDIT_COORD_LIMIT).contains(&y)
        {
            return Err(CanvasSessionError::CoordinateOutOfRange);
        }
        Ok(())
    }

    /// Move X and Y atomically as one undoable action. Never splice
    /// X from one participant with Y from another participant.
    pub fn move_node(&mut self, id: &str, x: i64, y: i64) -> Result<bool, CanvasSessionError> {
        Self::edit_coordinates(x, y)?;
        let mut next = self.value.clone();
        let node = Self::node_mut(&mut next, id)?;
        node["x"] = json!(x);
        node["y"] = json!(y);
        self.stage(next)
    }

    pub fn set_text(&mut self, id: &str, text: &str) -> Result<bool, CanvasSessionError> {
        let mut next = self.value.clone();
        let node = Self::node_mut(&mut next, id)?;
        if node.get("type").and_then(Value::as_str) != Some("text") {
            return Err(CanvasSessionError::NotTextNode(id.to_owned()));
        }
        node["text"] = json!(text);
        self.stage(next)
    }

    /// Append an ordinary JSON Canvas text card. Existing node order remains
    /// unchanged, so source z-order continues to be authoritative.
    pub fn add_text(&mut self, text: &str, x: i64, y: i64) -> Result<String, CanvasSessionError> {
        Self::edit_coordinates(x, y)?;
        let mut next = self.value.clone();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let object = next.as_object_mut().expect("validated Canvas root is an object");
        let nodes = object.entry("nodes").or_insert_with(|| json!([]))
            .as_array_mut().expect("validated Canvas nodes is an array");
        nodes.push(json!({
            "id": id,
            "type": "text",
            "x": x,
            "y": y,
            "width": 240,
            "height": 120,
            "text": text,
        }));
        self.stage(next)?;
        Ok(id)
    }

    /// Removing a card removes only edges referencing that card. Unknown root
    /// properties, remaining nodes and unrelated extension edges survive.
    pub fn remove_node(&mut self, id: &str) -> Result<bool, CanvasSessionError> {
        let mut next = self.value.clone();
        let nodes = next.get_mut("nodes").and_then(Value::as_array_mut)
            .ok_or_else(|| CanvasSessionError::MissingNode(id.to_owned()))?;
        let Some(index) = nodes.iter().position(|node| node.get("id").and_then(Value::as_str) == Some(id)) else {
            return Err(CanvasSessionError::MissingNode(id.to_owned()));
        };
        nodes.remove(index);
        if let Some(edges) = next.get_mut("edges").and_then(Value::as_array_mut) {
            edges.retain(|edge| {
                edge.get("fromNode").and_then(Value::as_str) != Some(id)
                    && edge.get("toNode").and_then(Value::as_str) != Some(id)
            });
        }
        self.stage(next)
    }

    pub fn undo(&mut self) -> Result<bool, CanvasSessionError> {
        let Some(previous) = self.undo.last().cloned() else { return Ok(false); };
        self.document.replace_canvas(&previous)?;
        let before = std::mem::replace(&mut self.value, previous);
        self.undo.pop();
        self.redo.push(before);
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, CanvasSessionError> {
        let Some(next) = self.redo.last().cloned() else { return Ok(false); };
        self.document.replace_canvas(&next)?;
        let before = std::mem::replace(&mut self.value, next);
        self.redo.pop();
        self.undo.push(before);
        Ok(true)
    }

    /// On File Gate conflict, the edited Value and both history stacks survive
    /// unchanged for explicit user recovery. No silent overwrite or retry.
    pub fn save(&mut self, core: &Core) -> Result<bool, CanvasSessionError> {
        if !self.dirty() { return Ok(false); }
        let saved = self.document.save(core)?;
        if saved { self.saved = self.value.clone(); }
        Ok(saved)
    }
}
