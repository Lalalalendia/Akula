//! Lossless read-only opening and guarded edits for JSON Canvas and Bases.
//! This is a Core format primitive, not a Canvas/Bases renderer.
//! Paths passed here must already be authorized by the caller's vault resolver.
use crate::app_core::Core;
use crate::files::document::read_file_snapshot_impl;
use crate::files::error::FileCommandError;
use crate::files::gate;
use crate::history::Source;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Canvas,
    Base,
}

#[derive(Debug)]
pub enum StructuredError {
    File(FileCommandError),
    InvalidFormat(String),
}

impl From<FileCommandError> for StructuredError {
    fn from(value: FileCommandError) -> Self { Self::File(value) }
}

impl std::fmt::Display for StructuredError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(e) => write!(fmt, "{e}"),
            Self::InvalidFormat(e) => fmt.write_str(e),
        }
    }
}

pub struct StructuredDocument {
    path: PathBuf,
    kind: Format,
    original_hash: String,
    original_text: String,
    edited_text: Option<String>,
}

fn kind_for(path: &Path) -> Result<Format, StructuredError> {
    match path.extension().and_then(|v| v.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("canvas") => Ok(Format::Canvas),
        Some(ext) if ext.eq_ignore_ascii_case("base") => Ok(Format::Base),
        _ => Err(StructuredError::InvalidFormat("expected .canvas or .base file".into())),
    }
}

mod schema; pub mod session;
use schema::validate_canvas;

impl StructuredDocument {
    /// Opening never changes the input file, including whitespace and unknown keys.
    pub fn open(path: &Path) -> Result<Self, StructuredError> {
        let kind = kind_for(path)?;
        let snapshot = read_file_snapshot_impl(path)?;
        if kind == Format::Canvas { validate_canvas(&snapshot.content)?; }
        Ok(Self {
            path: path.to_path_buf(),
            kind,
            original_hash: snapshot.hash,
            original_text: snapshot.content,
            edited_text: None,
        })
    }

    pub fn format(&self) -> Format { self.kind }
    pub fn text(&self) -> &str {
        self.edited_text.as_deref().unwrap_or(&self.original_text)
    }
    pub fn canvas(&self) -> Result<Value, StructuredError> {
        if self.kind != Format::Canvas {
            return Err(StructuredError::InvalidFormat("document is not JSON Canvas".into()));
        }
        validate_canvas(self.text())
    }

    /// Unknown JSON fields survive because the caller edits the complete Value.
    pub fn replace_canvas(&mut self, value: &Value) -> Result<(), StructuredError> {
        if self.kind != Format::Canvas {
            return Err(StructuredError::InvalidFormat("document is not JSON Canvas".into()));
        }
        let text = serde_json::to_string_pretty(value)
            .map_err(|e| StructuredError::InvalidFormat(e.to_string()))?;
        validate_canvas(&text)?;
        // Older Canvas files may contain fractional geometry. Reading is safe,
        // but rewriting with Value could silently change numeric precision.
        if schema::has_legacy_geometry(&validate_canvas(self.text())?)
            || schema::has_legacy_geometry(value)
        {
            return Err(StructuredError::InvalidFormat(
                "legacy fractional Canvas geometry is read-only to prevent rounding".into(),
            ));
        }
        self.edited_text = (text != self.original_text).then_some(text);
        Ok(())
    }

    /// Bases YAML is intentionally opaque until a preserving parser exists.
    /// No claims about evaluating filters/formulas or modifying properties.
    pub fn replace_base_text(&mut self, content: &str) -> Result<(), StructuredError> {
        if self.kind != Format::Base {
            return Err(StructuredError::InvalidFormat("document is not a .base file".into()));
        }
        self.edited_text = (content != self.original_text).then(|| content.to_owned());
        Ok(())
    }

    /// No-op preserves original bytes; changed files use the Core File Gate.
    /// On conflict the edit remains in memory for explicit recovery.
    pub fn save(&mut self, core: &Core) -> Result<bool, StructuredError> {
        let Some(text) = self.edited_text.as_deref() else { return Ok(false) };
        let result = gate::write(core, &self.path, text, Some(&self.original_hash), Source::Me, None)?;
        self.original_hash = result.hash;
        self.original_text = text.to_owned();
        self.edited_text = None;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::CoreEvent;
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn canvas_open_close_preserves_raw_bytes_and_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.canvas");
        let bytes = b"{\r\n  \"nodes\": [], \"edges\": [], \"custom\": {\"x\": 7}\r\n}\r\n";
        fs::write(&path, bytes).unwrap();
        let document = StructuredDocument::open(&path).unwrap();
        assert_eq!(document.canvas().unwrap()["custom"]["x"], 7);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn invalid_canvas_does_not_rewrite_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.canvas");
        fs::write(&path, "{\"nodes\": {}, \"edges\": []}").unwrap();
        assert!(StructuredDocument::open(&path).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "{\"nodes\": {}, \"edges\": []}");
    }

    #[test]
    fn base_open_close_is_byte_exact_and_opaque() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.base");
        let bytes = b"views:\r\n  - type: unknown\r\n    formula: custom(42)\r\n";
        fs::write(&path, bytes).unwrap();
        let mut doc = StructuredDocument::open(&path).unwrap();
        assert_eq!(doc.format(), Format::Base);
        assert!(doc.text().contains("custom(42)"));
        assert!(doc.replace_canvas(&serde_json::json!({"nodes": [], "edges": []})).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn canvas_guarded_save_preserves_unknown_fields_and_refuses_external_write() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let data = dir.path().join("data");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&data).unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();

        let path = vault.join("board.canvas");
        fs::write(&path, r#"{"nodes":[],"edges":[],"plugin":{"keep":true}}"#).unwrap();
        let mut doc = StructuredDocument::open(&path).unwrap();
        let mut value = doc.canvas().unwrap();
        value["plugin"]["new"] = serde_json::json!(1);
        doc.replace_canvas(&value).unwrap();
        fs::write(&path, r#"{"nodes":[],"edges":[],"external":true}"#).unwrap();
        assert!(matches!(doc.save(&core), Err(StructuredError::File(FileCommandError::Conflict { .. }))));
        assert!(fs::read_to_string(&path).unwrap().contains("external"));
        assert!(doc.text().contains("new"));

        let mut fresh = StructuredDocument::open(&path).unwrap();
        let mut v = fresh.canvas().unwrap();
        v["saved"] = serde_json::json!(true);
        fresh.replace_canvas(&v).unwrap();
        assert!(fresh.save(&core).unwrap());
        assert_eq!(StructuredDocument::open(&path).unwrap().canvas().unwrap()["external"], true);
        assert_eq!(StructuredDocument::open(&path).unwrap().canvas().unwrap()["saved"], true);
        assert!(!fresh.save(&core).unwrap());
    }
}
