//! Bounded, one-action Canvas text card resizing.
//! StructuredDocument and File Gate validate before staging or persisting.
use super::*;

const MIN_WIDTH: i64 = 48;
const MIN_HEIGHT: i64 = 32;
const MAX_DIMENSION: i64 = 10_000_000;

impl CanvasSession {
    pub fn resize_node(
        &mut self, id: &str, width: i64, height: i64,
    ) -> Result<bool, CanvasSessionError> {
        if !(MIN_WIDTH..=MAX_DIMENSION).contains(&width)
            || !(MIN_HEIGHT..=MAX_DIMENSION).contains(&height)
        {
            return Err(CanvasSessionError::InvalidDimensions);
        }
        let mut next = self.value.clone();
        let node = Self::node_mut(&mut next, id)?;
        if node.get("type").and_then(Value::as_str) != Some("text") {
            return Err(CanvasSessionError::NotTextNode(id.to_owned()));
        }
        node["width"] = json!(width);
        node["height"] = json!(height);
        self.stage(next)
    }
}
