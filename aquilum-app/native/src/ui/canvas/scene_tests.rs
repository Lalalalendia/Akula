//! Real native widget paint smoke: test the JSON Canvas scene, not an older graph.
use std::sync::Arc;

use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
use masonry::dpi::PhysicalSize;
use serde_json::{json, Value};

use super::CanvasPage;
use crate::render::Renderer;
use crate::ui::theme;

const WIDTH: u32 = 720;
const HEIGHT: u32 = 440;

fn render_board(value: &Value, name: &str) -> image::RgbaImage {
    let options = RenderRootOptions {
        default_properties: Arc::new(theme::default_properties()),
        use_system_fonts: true,
        size_policy: WindowSizePolicy::User,
        size: PhysicalSize::new(WIDTH, HEIGHT),
        scale_factor: 1.0,
        test_font: None,
    };
    let page = CanvasPage::new(value);
    let mut root = RenderRoot::new(page.erased(), |_| {}, options);
    theme::register_fonts(&mut root);
    let (layers, _) = root.redraw();

    let mut renderer = Renderer::new(0);
    renderer
        .render(&layers, WIDTH as u16, HEIGHT as u16, 1.0, theme::background())
        .expect("native Canvas frame must render");
    let png = renderer.png();
    if let Some(dir) = std::env::var_os("AQ_CANVAS_SCENE_DIR") {
        std::fs::create_dir_all(&dir).expect("create Canvas screenshot directory");
        std::fs::write(std::path::PathBuf::from(dir).join(format!("{name}.png")), &png)
            .expect("write Canvas screenshot");
    }
    image::load_from_memory(&png)
        .expect("PNG produced by native renderer")
        .to_rgba8()
}

#[test]
fn native_canvas_paints_cards_connections_and_unknown_nodes() {
    let fixture = json!({
        "nodes": [
            {"id":"group","type":"group","x":0,"y":0,"width":430,"height":280,
             "label":"Project"},
            {"id":"text","type":"text","x":30,"y":40,"width":200,"height":100,
             "text":"# Task\\nSome notes","color":"4"},
            {"id":"link","type":"link","x":265,"y":145,"width":135,"height":80,
             "url":"https://example.com","color":"#3478a9"},
            {"id":"plugin","type":"future.shape","x":35,"y":175,"width":130,"height":60,
             "vendor":{"keep":true}}
        ],
        "edges": [
            {"id":"edge","fromNode":"text","toNode":"link","fromSide":"right",
             "toSide":"left","toEnd":"arrow"}
        ],
        "vendor":{"opaque":"do-not-dereference"}
    });
    let original = fixture.clone();
    let populated = render_board(&fixture, "canvas-populated");
    let empty = render_board(&json!({}), "canvas-empty");
    let mut without_edges = fixture.clone();
    without_edges["edges"] = json!([]);
    let no_edges = render_board(&without_edges, "canvas-no-edges");
    let mut without_plugin = fixture.clone();
    without_plugin["nodes"].as_array_mut().unwrap().pop();
    let no_plugin = render_board(&without_plugin, "canvas-no-plugin");

    assert_eq!(populated.dimensions(), (WIDTH, HEIGHT));
    assert_eq!(empty.dimensions(), (WIDTH, HEIGHT));
    assert_eq!(fixture, original, "painting never modifies the source document");

    // Exclude the small status label at the top: actual board contents must
    // create visible output below it, not merely a different node counter.
    let changed_pixels = |actual: &image::RgbaImage, control: &image::RgbaImage| {
        let start = WIDTH as usize * 4 * 75;
        let actual = actual.as_raw();
        let control = control.as_raw();
        (start..actual.len())
            .step_by(4)
            .filter(|&i| actual[i..i + 4] != control[i..i + 4])
            .count()
    };
    let cards = changed_pixels(&populated, &empty);
    let edges = changed_pixels(&populated, &no_edges);
    let placeholders = changed_pixels(&populated, &no_plugin);
    assert!(cards > 1_500, "native Canvas looks blank: {cards} changed pixels");
    assert!(edges > 20, "Canvas connection or arrow was not visible: {edges} pixels");
    assert!(placeholders > 100, "unknown node is missing its visible placeholder: {placeholders} pixels");
}

#[test]
fn native_canvas_empty_board_renders_without_graph_or_vault() {
    let blank = render_board(&json!({"nodes":[],"edges":[]}), "canvas-blank");
    assert_eq!(blank.dimensions(), (WIDTH, HEIGHT));
}


/// Route a real file through the production Core format boundary before the
/// native renderer. Opening/rendering is read-only, including vendor data.
#[test]
fn native_canvas_real_disk_file_passes_core_validation_and_paint() {
    use aquilum_core::files::structured::StructuredDocument;
    use std::fs;

    let dir = tempfile::tempdir().expect("temporary Canvas workspace");
    let path = dir.path().join("vendor-board.canvas");
    let raw = concat!(
        "{\r\n",
        r#"  "nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"A real disk document","plugin":{"opaque":[1,2,3]}}],"#,
        "\r\n",
        r#"  "edges":[],"thirdParty":{"keep":true}"#,
        "\r\n}\r\n"
    );
    fs::write(&path, raw.as_bytes()).expect("write fixture");

    let document = StructuredDocument::open(&path).expect("open with production schema");
    let json = document.canvas().expect("validated Canvas JSON");
    assert_eq!(json["thirdParty"]["keep"], true);
    let image = render_board(&json, "canvas-from-disk");
    assert_eq!(image.dimensions(), (WIDTH, HEIGHT));
    assert_eq!(fs::read(&path).expect("source bytes"), raw.as_bytes());

    // Reopen on the same path to prove no accidental source rewrite or session
    // dependence introduced by creating/dropping a render widget.
    drop(document);
    assert!(StructuredDocument::open(&path).is_ok());
    assert_eq!(fs::read(&path).unwrap(), raw.as_bytes());
}

#[test]
fn native_canvas_invalid_disk_file_fails_closed_without_rewrite() {
    use aquilum_core::files::structured::StructuredDocument;
    use std::fs;

    let dir = tempfile::tempdir().expect("temporary Canvas workspace");
    let path = dir.path().join("invalid.canvas");
    let raw = br#"{ "nodes": {"wrong": "object"}, "edges": [] }"#;
    fs::write(&path, raw).expect("write malformed fixture");
    assert!(StructuredDocument::open(&path).is_err());
    assert_eq!(fs::read(&path).expect("existing source unchanged"), raw);
}
