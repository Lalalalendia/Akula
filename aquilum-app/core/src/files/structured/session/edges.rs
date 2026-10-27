//! Standard JSON Canvas edge creation with one validated Undo/Redo action.
use super::*;

impl CanvasSession {
    pub fn add_edge(&mut self, from: &str, to: &str) -> Result<String, CanvasSessionError> {
        let nodes = self.value.get("nodes").and_then(Value::as_array);
        for id in [from, to] {
            if !nodes.is_some_and(|items| items.iter().any(|node| {
                node.get("id").and_then(Value::as_str) == Some(id)
            })) {
                return Err(CanvasSessionError::MissingNode(id.to_owned()));
            }
        }
        let mut next = self.value.clone();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let edges = next.as_object_mut().expect("validated JSON Canvas object")
            .entry("edges").or_insert_with(|| json!([]))
            .as_array_mut().expect("validated edge array");
        edges.push(json!({"id":id, "fromNode":from, "toNode":to, "toEnd":"arrow"}));
        self.stage(next)?;
        Ok(id)
    }
}
