//! JSON Canvas 1.0 group creation. Insert behind existing nodes to keep
//! their drawing order and leave their geometry, edges and extensions intact.
use super::*;

const GROUP_WIDTH: i64 = 420;
const GROUP_HEIGHT: i64 = 260;

impl CanvasSession {
    pub fn add_group(
        &mut self, label: &str, x: i64, y: i64,
    ) -> Result<String, CanvasSessionError> {
        Self::edit_coordinates(x, y)?;
        let mut next = self.value.clone();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let nodes = next.as_object_mut().expect("validated Canvas object")
            .entry("nodes").or_insert_with(|| json!([]))
            .as_array_mut().expect("validated Canvas nodes");
        // A group is a visual enclosure, not ownership: moving/creating it
        // must never reparent or modify overlapping text/file/link nodes.
        nodes.insert(0, json!({
            "id": id, "type": "group", "x": x, "y": y,
            "width": GROUP_WIDTH, "height": GROUP_HEIGHT, "label": label,
        }));
        self.stage(next)?;
        Ok(id)
    }
}
