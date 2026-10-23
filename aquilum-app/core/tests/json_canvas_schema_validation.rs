//! AK-DOC-C1b: structural JSON Canvas 1.0 compatibility, not Canvas UI.
use aquilum_core::files::structured::StructuredDocument;
use serde_json::json;
use std::fs;

fn open(text: &str) -> Result<StructuredDocument, String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.canvas");
    fs::write(&path, text).unwrap();
    let result = StructuredDocument::open(&path).map_err(|err| err.to_string());
    assert_eq!(fs::read_to_string(&path).unwrap(), text, "invalid files are never repaired");
    result
}

#[test]
fn all_official_node_kinds_and_edges_preserve_array_order_and_extra_properties() {
    let board = json!({
        "nodes": [
            {"id":"a","type":"text","x":-8,"y":0,"width":200,"height":100,
             "text":"# Hello","color":"1","custom":{"keep":true}},
            {"id":"b","type":"file","x":0,"y":100,"width":50,"height":60,
             "file":"Photos/image.png","subpath":"#heading"},
            {"id":"c","type":"link","x":42,"y":12,"width":80,"height":80,
             "url":"https://example.com","color":"#123ABC"},
            {"id":"d","type":"group","x":0,"y":0,"width":400,"height":300,
             "label":"Group","background":"background.png","backgroundStyle":"ratio"},
            {"id":"e","type":"obsidium.shape","x":7,"y":5,"width":100,"height":40,
             "shape":{"curve":true},"vendor":{"new":9}}
        ],
        "edges": [
            {"id":"e-1","fromNode":"a","toNode":"d","fromSide":"right",
             "toSide":"left","fromEnd":"none","toEnd":"arrow","color":"6",
             "label":"depends"}
        ],
        "unrecognized": {"preserve": [1, 2, 3]}
    });
    let doc = open(&board.to_string()).unwrap();
    let value = doc.canvas().unwrap();
    assert_eq!(value["nodes"][0]["id"], "a");
    assert_eq!(value["nodes"][4]["id"], "e");
    assert_eq!(value["nodes"][4]["vendor"]["new"], 9);
    assert_eq!(value["nodes"][0]["custom"]["keep"], true);
    assert_eq!(value["edges"][0]["fromNode"], "a");
    assert_eq!(value["unrecognized"]["preserve"], json!([1, 2, 3]));
}

#[test]
fn broken_geometry_and_known_node_fields_are_rejected_without_modification() {
    let cases = [
        r#"{"nodes":[null]}"#,
        r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":3,"height":4}]}"#,
        r#"{"nodes":[{"id":"a","type":"file","x":0,"y":0,"width":3,"height":4}]}"#,
        r#"{"nodes":[{"id":"a","type":"link","x":0,"y":0,"width":3,"height":4}]}"#,
        r#"{"nodes":[{"id":"a","type":"group","x":0,"y":0,"width":3,"height":4,"backgroundStyle":"unknown"}]}"#,
        r#"{"nodes":[{"id":"a","type":"text","x":"0","y":0,"width":3,"height":4,"text":"a"}]}"#,
        r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":-3,"height":4,"text":"a"}]}"#,
        r#"{"nodes":[{"id":"a","type":"file","x":0,"y":0,"width":3,"height":4,"file":"a.png","subpath":"bad"}]}"#,
        r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":3,"height":4,"text":1}]}"#,
        r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":3,"height":4,"text":"x","color":"7"}]}"#,
        r#"{"nodes":[{"type":"text","x":0,"y":0,"width":3,"height":4,"text":"x"}]}"#,
        r#"{"nodes":[{"id":"a","x":0,"y":0,"width":3,"height":4,"text":"x"}]}"#,
    ];
    for (i, board) in cases.iter().enumerate() {
        assert!(open(board).is_err(), "invalid node {i}: {board}");
    }
}

#[test]
fn duplicate_ids_orphan_references_and_invalid_edges_are_rejected() {
    let node = r#"{"id":"a","type":"text","x":0,"y":0,"width":10,"height":10,"text":""}"#;
    let cases = [
        format!(r#"{{"nodes":[{node},{node}]}}"#),
        format!(r#"{{"nodes":[{node}],"edges":[{{"id":"e","fromNode":"a","toNode":"missing"}}]}}"#),
        format!(r#"{{"nodes":[{node}],"edges":[{{"id":"e","fromNode":"a","toNode":"a"}},{{"id":"e","fromNode":"a","toNode":"a"}}]}}"#),
        format!(r#"{{"nodes":[{node}],"edges":[{{"id":"e","fromNode":"a"}}]}}"#),
        format!(r#"{{"nodes":[{node}],"edges":[{{"id":"e","fromNode":"a","toNode":"a","toSide":"diagonal"}}]}}"#),
        format!(r#"{{"nodes":[{node}],"edges":[{{"id":"e","fromNode":"a","toNode":"a","fromEnd":"diamond"}}]}}"#),
        r#"{"edges":[{"id":"e","fromNode":"a","toNode":"b"}]}"#.to_owned(),
    ];
    for (i, board) in cases.iter().enumerate() {
        assert!(open(board).is_err(), "invalid edge {i}: {board}");
    }
}

#[test]
fn legacy_fractional_geometry_is_readable_but_never_silently_rewritten() {
    let board = r#"{"nodes":[{"id":"a","type":"text","x":12.125,"y":0,"width":33.75,"height":70,"text":"hello"}],"vendor":"keep"}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.canvas");
    fs::write(&path, board).unwrap();
    let mut doc = StructuredDocument::open(&path).expect("old finite coordinates remain readable");
    let mut value = doc.canvas().unwrap();
    assert_eq!(value["nodes"][0]["x"], json!(12.125));
    value["nodes"][0]["text"] = json!("changed");
    assert!(doc.replace_canvas(&value).is_err(), "lossy legacy rewrite must be refused");
    assert_eq!(fs::read_to_string(&path).unwrap(), board);
}

#[test]
fn integer_geometry_and_unknown_properties_allow_guarded_edit_in_memory() {
    let board = r#"{"nodes":[{"id":"a","type":"text","x":12,"y":7,"width":33,"height":70,"text":"hello","plugin":{"keep":true}}]}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.canvas");
    fs::write(&path, board).unwrap();
    let mut doc = StructuredDocument::open(&path).unwrap();
    let mut value = doc.canvas().unwrap();
    value["nodes"][0]["text"] = json!("changed");
    doc.replace_canvas(&value).unwrap();
    assert_eq!(doc.canvas().unwrap()["nodes"][0]["plugin"]["keep"], true);
    assert_eq!(fs::read_to_string(&path).unwrap(), board);
}
