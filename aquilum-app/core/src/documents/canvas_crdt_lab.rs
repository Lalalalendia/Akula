//! Offline-only, feature-gated proof of a two-peer JSON Canvas CRDT projection.
//!
//! The original JSON Canvas schema remains authoritative for file I/O. This module
//! does not write files, open sockets, grant permissions, or replace CanvasSession.
//! Do not feed untrusted network traffic here until a real authenticated transport,
//! complete Yrs decoder hardening, and production resource limits are accepted.

use std::collections::HashSet;

use serde_json::{json, Value};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, ClientID, Doc, GetString, Map, MapPrelim, MapRef, OffsetKind, Options, Out, ReadTxn,
    StateVector, Text, TextPrelim, Transact,
};

use crate::documents::merge::TextEdit;
use super::{update_guard, utf16_boundary};

const MAX_NODES: usize = 2_000;
const MAX_UPDATE_BYTES: usize = 4 * 1024 * 1024;
const MAX_VECTOR_BYTES: usize = 128 * 1024;
const MAX_COORD: f64 = 100_000_000.0;
const MAX_SIZE: f64 = 10_000_000.0;

/// A standalone in-memory replica. A distinct, stable client_id is required per
/// participant; both clients start from ONE seeded CRDT state, not two imports.
pub struct CanvasCrdtLab {
    doc: Doc,
    nodes: MapRef,
    meta: MapRef,
}

impl CanvasCrdtLab {
    pub fn new(client_id: ClientID) -> Self {
        let doc = Doc::with_options(Options {
            client_id,
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        });
        let nodes = doc.get_or_insert_map("canvas.nodes");
        let meta = doc.get_or_insert_map("canvas.meta");
        Self { doc, nodes, meta }
    }

    /// Seed once from the validated local document. Other replicas must receive
    /// this state as a Yrs update; importing the same JSON twice creates conflicts.
    pub fn seeded(client_id: ClientID, source: &Value) -> Result<Self, String> {
        if !source.is_object() {
            return Err("JSON Canvas root must be an object".into());
        }
        if source.get("edges").is_some_and(|v| !v.is_array()) {
            return Err("JSON Canvas edges must be an array".into());
        }
        let empty = Vec::new();
        let items = match source.get("nodes") {
            None => &empty,
            Some(Value::Array(items)) => items,
            _ => return Err("JSON Canvas nodes must be an array".into()),
        };
        if items.len() > MAX_NODES {
            return Err("too many Canvas nodes for the offline laboratory".into());
        }
        let mut ids = HashSet::new();
        for node in items {
            let id = node.get("id").and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("Canvas node lacks a nonempty id")?;
            if !ids.insert(id) {
                return Err("duplicate Canvas node id".into());
            }
            let _ = geometry(node)?;
            if node.get("type").and_then(Value::as_str) == Some("text")
                && node.get("text").and_then(Value::as_str).is_none()
            {
                return Err("text node lacks text".into());
            }
        }
        let result = Self::new(client_id);
        {
            let mut txn = result.doc.transact_mut_with("canvas-seed");
            result.meta.insert(&mut txn, "root", source.to_string());
            for (index, node) in items.iter().enumerate() {
                let id = node["id"].as_str().expect("prevalidated id");
                let child: MapRef =
                    result.nodes.insert(&mut txn, id, MapPrelim::default());
                child.insert(&mut txn, "raw", node.to_string());
                child.insert(&mut txn, "geometry", json!(geometry(node)?).to_string());
                child.insert(&mut txn, "order", index as f64);
                if node["type"].as_str() == Some("text") {
                    let text = node["text"].as_str().expect("prevalidated text");
                    child.insert(&mut txn, "text", TextPrelim::new(text));
                }
            }
        }
        Ok(result)
    }

    pub fn state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    pub fn state_vector_bytes(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    pub fn diff_for(&self, peer_vector: &[u8]) -> Result<Vec<u8>, String> {
        if peer_vector.len() > MAX_VECTOR_BYTES {
            return Err("Canvas state vector too large".into());
        }
        let vector = StateVector::decode_v1(peer_vector).map_err(|e| e.to_string())?;
        Ok(self.doc.transact().encode_state_as_update_v1(&vector))
    }

    /// Stage every remote update against a disposable replica and validate its
    /// projection before touching the live in-memory state. This is NOT a full
    /// malicious-Yrs-input sandbox and intentionally has no network entrypoint.
    pub fn apply_remote_update(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_UPDATE_BYTES {
            return Err("Canvas CRDT update exceeds the laboratory size limit".into());
        }
        let staged = Self::new(self.doc.client_id());
        staged.apply_checked(&self.state())?;
        staged.apply_checked(bytes)?;
        staged.snapshot()?;
        self.apply_checked(bytes)
    }

    fn apply_checked(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_UPDATE_BYTES {
            return Err("Canvas CRDT update exceeds the laboratory size limit".into());
        }
        let update = update_guard::decode(bytes).map_err(|e| e.to_string())?;
        self.doc.transact_mut_with("canvas-remote")
            .apply_update(update).map_err(|e| e.to_string())
    }

    /// One CRDT register for the X/Y pair. A concurrent drag can win or lose,
    /// but cannot combine X from one user with Y from the other.
    pub fn move_node(&self, id: &str, x: i64, y: i64) -> Result<Vec<u8>, String> {
        if (x as f64).abs() > MAX_COORD || (y as f64).abs() > MAX_COORD {
            return Err("Canvas coordinates are out of bounds".into());
        }
        let mut txn = self.doc.transact_mut_with("canvas-local");
        let Out::YMap(child) = self.nodes.get(&txn, id).ok_or("Canvas node not found")?
        else {
            return Err("Canvas node record is not a Y.Map".into());
        };
        let raw = string_field(&child, &txn, "geometry")?;
        let mut coords: Vec<Value> =
            serde_json::from_str(&raw).map_err(|_| "invalid Canvas geometry")?;
        validate_geometry(&coords)?;
        coords[0] = json!(x);
        coords[1] = json!(y);
        child.insert(&mut txn, "geometry", Value::Array(coords).to_string());
        Ok(txn.encode_update_v1())
    }

    /// UTF-16 edits on a dedicated nested Y.Text. Geometry and text never
    /// overwrite each other. Reject split-surrogate or invalid edit intervals.
    pub fn edit_text(&self, id: &str, edits: &[TextEdit]) -> Result<Vec<u8>, String> {
        let mut txn = self.doc.transact_mut_with("canvas-local");
        let Out::YMap(child) = self.nodes.get(&txn, id).ok_or("Canvas node not found")?
        else {
            return Err("Canvas node record is not a Y.Map".into());
        };
        let Out::YText(text) = child.get(&txn, "text").ok_or("not a Canvas text node")?
        else {
            return Err("Canvas text field is not Y.Text".into());
        };
        utf16_boundary::validate_utf16_edits(&text.get_string(&txn), edits)
            .map_err(|e| e.to_string())?;
        for edit in edits.iter().rev() {
            if edit.to > edit.from {
                text.remove_range(&mut txn, edit.from as u32, (edit.to - edit.from) as u32);
            }
            if !edit.insert.is_empty() {
                text.insert(&mut txn, edit.from as u32, &edit.insert);
            }
        }
        Ok(txn.encode_update_v1())
    }

    /// Build JSON Canvas without throwing away unfamiliar node, edge or root
    /// properties. Original node array order is retained for existing nodes.
    pub fn snapshot(&self) -> Result<Value, String> {
        let txn = self.doc.transact();
        let raw = string_field(&self.meta, &txn, "root")?;
        let mut root: Value =
            serde_json::from_str(&raw).map_err(|_| "invalid Canvas root record")?;
        let Some(object) = root.as_object_mut() else {
            return Err("invalid Canvas root object".into());
        };

        let mut ordered = Vec::new();
        for (id, entry) in self.nodes.iter(&txn) {
            let Out::YMap(child) = entry else {
                return Err("invalid Canvas node record".into());
            };
            let raw = string_field(&child, &txn, "raw")?;
            let mut node: Value =
                serde_json::from_str(&raw).map_err(|_| "invalid Canvas node JSON")?;
            if node.get("id").and_then(Value::as_str) != Some(id) {
                return Err("Canvas node id does not match CRDT key".into());
            }
            let coords: Vec<Value> = serde_json::from_str(
                &string_field(&child, &txn, "geometry")?
            ).map_err(|_| "invalid Canvas geometry")?;
            validate_geometry(&coords)?;
            let fields = node.as_object_mut().ok_or("invalid Canvas node object")?;
            for (key, value) in ["x", "y", "width", "height"].iter().zip(coords) {
                fields.insert((*key).to_owned(), value);
            }
            if fields.get("type").and_then(Value::as_str) == Some("text") {
                let Out::YText(text) =
                    child.get(&txn, "text").ok_or("text node lacks CRDT text")?
                else {
                    return Err("Canvas text field is not Y.Text".into());
                };
                fields.insert("text".into(), Value::String(text.get_string(&txn)));
            }
            let order = match child.get(&txn, "order") {
                Some(Out::Any(Any::Number(order))) => order.as_f64()
                    .filter(|value| value.is_finite()).ok_or("invalid Canvas node order")?,
                _ => return Err("invalid Canvas node order".into()),
            };
            ordered.push((order, id.to_owned(), node));
        }
        if ordered.len() > MAX_NODES {
            return Err("too many Canvas nodes".into());
        }
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        if object.contains_key("nodes") || !ordered.is_empty() {
            object.insert("nodes".into(),
                Value::Array(ordered.into_iter().map(|(_, _, node)| node).collect()));
        }
        Ok(root)
    }
}

fn string_field<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> Result<String, String> {
    match map.get(txn, key) {
        Some(Out::Any(Any::String(value))) => Ok(value.to_string()),
        _ => Err(format!("Canvas CRDT field missing or invalid: {key}")),
    }
}

fn geometry(node: &Value) -> Result<[Value; 4], String> {
    let mut result = Vec::with_capacity(4);
    for key in ["x", "y", "width", "height"] {
        result.push(node.get(key)
            .ok_or_else(|| format!("Canvas node lacks {key}"))?.clone());
    }
    validate_geometry(&result)?;
    Ok(result.try_into().expect("four geometry values"))
}

fn validate_geometry(values: &[Value]) -> Result<(), String> {
    if values.len() != 4 {
        return Err("Canvas geometry requires x/y/width/height".into());
    }
    for (index, value) in values.iter().enumerate() {
        let number = value.as_f64().filter(|number| number.is_finite())
            .ok_or("Canvas geometry is not a finite number")?;
        if index < 2 && number.abs() > MAX_COORD {
            return Err("Canvas position exceeds bounds".into());
        }
        if index >= 2 && !(0.0..=MAX_SIZE).contains(&number) {
            return Err("Canvas size exceeds bounds".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEXT_ID: &str = "59e896bc8da20699";
    const OFFICIAL: &str = include_str!("../../tests/fixtures/jsoncanvas/official-sample.canvas");

    fn seed() -> (CanvasCrdtLab, CanvasCrdtLab) {
        let original: Value = serde_json::from_str(OFFICIAL).unwrap();
        let first = CanvasCrdtLab::seeded(ClientID::new(101), &original).unwrap();
        let second = CanvasCrdtLab::new(ClientID::new(202));
        second.apply_remote_update(&first.state()).unwrap();
        (first, second)
    }

    fn converge(a: &CanvasCrdtLab, b: &CanvasCrdtLab) {
        let from_a = a.state();
        let from_b = b.state();
        a.apply_remote_update(&from_b).unwrap();
        b.apply_remote_update(&from_a).unwrap();
        assert_eq!(a.snapshot().unwrap(), b.snapshot().unwrap());
    }

    #[test]
    fn official_sample_roundtrip_preserves_order_edges_and_unknowns() {
        let mut original: Value = serde_json::from_str(OFFICIAL).unwrap();
        original["nodes"][3]["custom.vendor"] = json!({"keep": ["all", 3]});
        original["root-extension"] = json!(true);
        let replica = CanvasCrdtLab::seeded(ClientID::new(21), &original).unwrap();
        assert_eq!(replica.snapshot().unwrap(), original);
        let peer = CanvasCrdtLab::new(ClientID::new(22));
        peer.apply_remote_update(&replica.state()).unwrap();
        assert_eq!(peer.snapshot().unwrap(), original);
    }

    #[test]
    fn concurrent_move_and_text_edit_merge_without_lost_fields() {
        let (a, b) = seed();
        a.move_node(TEXT_ID, 717, -815).unwrap();
        b.edit_text(TEXT_ID, &[TextEdit {
            from: 0, to: 0, insert: "Remote: ".into(),
        }]).unwrap();
        converge(&a, &b);
        let json = a.snapshot().unwrap();
        let node = json["nodes"].as_array().unwrap().iter()
            .find(|node| node["id"] == TEXT_ID).unwrap();
        assert_eq!(node["x"], 717);
        assert_eq!(node["y"], -815);
        assert!(node["text"].as_str().unwrap().starts_with("Remote: "));
        assert_eq!(json["edges"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn conflicting_drags_never_mix_the_two_coordinates() {
        let (a, b) = seed();
        a.move_node(TEXT_ID, 10, 20).unwrap();
        b.move_node(TEXT_ID, 300, 400).unwrap();
        converge(&a, &b);
        let json = a.snapshot().unwrap();
        let n = json["nodes"].as_array().unwrap().iter()
            .find(|node| node["id"] == TEXT_ID).unwrap();
        let xy = (n["x"].as_i64().unwrap(), n["y"].as_i64().unwrap());
        assert!(xy == (10, 20) || xy == (300, 400), "mixed X/Y: {xy:?}");
    }

    #[test]
    fn diff_and_rejoin_converge_after_offline_work() {
        let (a, b) = seed();
        a.move_node(TEXT_ID, 12, 34).unwrap();
        let vector = b.state_vector_bytes();
        let missing = a.diff_for(&vector).unwrap();
        b.apply_remote_update(&missing).unwrap();
        assert_eq!(a.snapshot().unwrap(), b.snapshot().unwrap());
    }

    #[test]
    fn invalid_utf16_and_garbage_updates_do_not_mutate_replica() {
        let source = json!({
            "nodes": [{
                "id":"t","type":"text","text":"a😀b",
                "x":0,"y":0,"width":120,"height":80,"vendor":{"keep":1}
            }],
            "edges": []
        });
        let replica = CanvasCrdtLab::seeded(ClientID::new(1), &source).unwrap();
        assert!(replica.edit_text("t", &[TextEdit {
            from: 2, to: 2, insert: "x".into(),
        }]).is_err());
        assert!(replica.apply_remote_update(&[255, 0, 19]).is_err());
        assert!(replica.apply_remote_update(&vec![0; MAX_UPDATE_BYTES + 1]).is_err());
        assert_eq!(replica.snapshot().unwrap(), source);
    }

    #[test]
    fn invalid_seeds_are_rejected_without_loading_files() {
        assert!(CanvasCrdtLab::seeded(ClientID::new(1), &json!({"nodes":{}})).is_err());
        assert!(CanvasCrdtLab::seeded(ClientID::new(1), &json!({
            "nodes": [
                {"id":"x","x":0,"y":0,"width":10,"height":10},
                {"id":"x","x":1,"y":1,"width":10,"height":10}
            ]
        })).is_err());
        assert!(CanvasCrdtLab::seeded(ClientID::new(1), &json!({
            "nodes":[{"id":"x","x":0,"y":0,"width":-1,"height":10}]
        })).is_err());
    }
}
