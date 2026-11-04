//! Pure, bounded projection of a validated JSON Canvas into native paint data.
//! Source JSON remains owned by StructuredDocument; this view never rewrites it.
use std::collections::HashMap;

use masonry::kurbo::Rect;
use serde_json::Value;

pub(super) const MAX_PAINT_NODES: usize = 2_000;
pub(super) const MAX_PAINT_EDGES: usize = 4_000;
const MAX_COORD: f64 = 100_000_000.0;
const MAX_SIZE: f64 = 10_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side { Top, Right, Bottom, Left }

impl Side {
    fn parse(value: Option<&Value>) -> Option<Self> {
        match value.and_then(Value::as_str) {
            Some("top") => Some(Self::Top),
            Some("right") => Some(Self::Right),
            Some("bottom") => Some(Self::Bottom),
            Some("left") => Some(Self::Left),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind { Text, File, Link, Group, Unsupported }

pub(super) struct Card {
    pub id: String,
    pub kind: Kind,
    pub rect: Rect,
    pub lines: Vec<String>,
    pub color: Option<String>,
}

pub(super) struct Connection {
    pub from: usize,
    pub to: usize,
    pub from_side: Option<Side>,
    pub to_side: Option<Side>,
    pub from_arrow: bool,
    pub to_arrow: bool,
    pub color: Option<String>,
}

pub(super) struct Board {
    pub cards: Vec<Card>,
    pub connections: Vec<Connection>,
    pub bounds: Rect,
    pub total_cards: usize,
    pub total_connections: usize,
}

fn coord(node: &Value, key: &str) -> Option<f64> {
    node.get(key)?.as_f64().filter(|n| n.is_finite() && n.abs() <= MAX_COORD)
}

fn dimension(node: &Value, key: &str) -> Option<f64> {
    node.get(key)?.as_f64().filter(|n| n.is_finite() && (0.0..=MAX_SIZE).contains(n))
}

fn short_line(input: &str) -> String {
    input.chars().take(180).collect()
}

fn card_lines(node: &Value, kind: Kind) -> Vec<String> {
    let text = |key| node.get(key).and_then(Value::as_str).unwrap_or("");
    let mut lines: Vec<String> = match kind {
        Kind::Text => text("text").lines().take(8).map(short_line).collect(),
        Kind::File => vec![format!("File: {}", text("file").rsplit(['/', '\\']).next().unwrap_or(""))],
        Kind::Link => vec![short_line(text("url"))],
        Kind::Group => vec![short_line(text("label"))],
        Kind::Unsupported => vec![format!("Unsupported node: {}", short_line(text("type")))],
    };
    if lines.is_empty() || lines.iter().all(String::is_empty) {
        lines = vec![match kind {
            Kind::Text => "Empty text".to_owned(),
            Kind::Group => "Group".to_owned(),
            _ => "Untitled".to_owned(),
        }];
    }
    lines
}

impl Board {
    pub fn from_json(value: &Value) -> Self {
        let source_cards = value.get("nodes").and_then(Value::as_array);
        let source_edges = value.get("edges").and_then(Value::as_array);
        let mut cards = Vec::new();
        let mut by_id = HashMap::new();
        for node in source_cards.into_iter().flatten() {
            if cards.len() >= MAX_PAINT_NODES { break; }
            let (Some(id), Some(x), Some(y), Some(w), Some(h)) = (
                node.get("id").and_then(Value::as_str),
                coord(node, "x"), coord(node, "y"),
                dimension(node, "width"), dimension(node, "height"),
            ) else { continue };
            if by_id.contains_key(id) { continue; }
            let kind = match node.get("type").and_then(Value::as_str) {
                Some("text") => Kind::Text,
                Some("file") => Kind::File,
                Some("link") => Kind::Link,
                Some("group") => Kind::Group,
                _ => Kind::Unsupported,
            };
            let card = Card {
                id: id.to_owned(),
                kind,
                rect: Rect::new(x, y, x + w.max(1.0), y + h.max(1.0)),
                lines: card_lines(node, kind),
                color: node.get("color").and_then(Value::as_str).map(str::to_owned),
            };
            by_id.insert(card.id.clone(), cards.len());
            cards.push(card);
        }

        let mut connections = Vec::new();
        for edge in source_edges.into_iter().flatten() {
            if connections.len() >= MAX_PAINT_EDGES { break; }
            let Some(from) = edge.get("fromNode").and_then(Value::as_str).and_then(|s| by_id.get(s)).copied() else { continue };
            let Some(to) = edge.get("toNode").and_then(Value::as_str).and_then(|s| by_id.get(s)).copied() else { continue };
            connections.push(Connection {
                from,
                to,
                from_side: Side::parse(edge.get("fromSide")),
                to_side: Side::parse(edge.get("toSide")),
                from_arrow: edge.get("fromEnd").and_then(Value::as_str) == Some("arrow"),
                to_arrow: edge.get("toEnd").and_then(Value::as_str) != Some("none"),
                color: edge.get("color").and_then(Value::as_str).map(str::to_owned),
            });
        }

        let bounds = if let Some(first) = cards.first() {
            cards.iter().skip(1).fold(first.rect, |b, card| Rect::new(
                b.x0.min(card.rect.x0), b.y0.min(card.rect.y0),
                b.x1.max(card.rect.x1), b.y1.max(card.rect.y1),
            ))
        } else {
            Rect::new(-200.0, -120.0, 200.0, 120.0)
        };
        Self {
            total_cards: source_cards.map_or(0, Vec::len),
            total_connections: source_edges.map_or(0, Vec::len),
            cards, connections, bounds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canvas_nodes_keep_source_z_order_and_unknown_types_get_placeholders() {
        let board = Board::from_json(&json!({
            "nodes": [
                {"id":"back","type":"group","x":0,"y":0,"width":300,"height":200},
                {"id":"front","type":"future.figure","x":30,"y":15,"width":90,"height":50,
                 "vendor":{"do_not_change":true}}
            ],
            "edges":[{"id":"e","fromNode":"front","toNode":"back","toEnd":"none"}]
        }));
        assert_eq!(board.cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["back", "front"]);
        assert_eq!(board.cards[1].kind, Kind::Unsupported);
        assert_eq!(board.connections.len(), 1);
        assert_eq!(board.connections[0].from, 1);
        assert!(!board.connections[0].to_arrow);
    }

    #[test]
    fn empty_and_partial_root_arrays_render_safely() {
        for value in [json!({}), json!({"nodes":[]}), json!({"edges":[]})] {
            let board = Board::from_json(&value);
            assert!(board.cards.is_empty());
            assert!(board.connections.is_empty());
            assert!(board.bounds.width() > 0.0);
        }
    }

    #[test]
    fn huge_geometry_and_untrusted_links_are_never_followed() {
        let board = Board::from_json(&json!({
            "nodes": [
                {"id":"huge","type":"text","x":1e150,"y":0,"width":10,"height":20,"text":"x"},
                {"id":"link","type":"link","x":0,"y":0,"width":180,"height":80,
                 "url":"file:///private/secret"}
            ]
        }));
        assert_eq!(board.total_cards, 2);
        assert_eq!(board.cards.len(), 1);
        assert_eq!(board.cards[0].lines[0], "file:///private/secret");
    }
}
