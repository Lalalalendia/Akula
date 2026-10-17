//! AK-PUB-MASONRY-PROBE-01: bounded geometry-only experiment.
//!
//! This consumes the serialized Chaptera PageRenderPlanV1 geometry boundary.
//! It DOES NOT implement text, images, tables, transformed shapes, editing,
//! hit testing, IME, or a Chaptera-vs-egui performance comparison.
//! Any such field is rejected rather than silently rendered incorrectly.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
use masonry::core::{NewWidget, WidgetTag};
use masonry::dpi::PhysicalSize;
use masonry::imaging::Painter;
use masonry::kurbo::Rect;
use masonry::peniko::Color;
use masonry::theme::default_property_set;
use masonry::widgets::Canvas;
use serde::Deserialize;

use crate::render::Renderer;

const EMU_PER_CSS_PX: f64 = 9_525.0;
const VIEW_WIDTH: u16 = 720;
const VIEW_HEIGHT: u16 = 480;
const PAGE_BACKGROUND: Color = Color::from_rgb8(255, 255, 255);
const BACKGROUND: Color = Color::from_rgb8(17, 19, 23);
const TAG: WidgetTag<Canvas> = WidgetTag::named("ak-pub-geometry-canvas");

#[derive(Debug, Deserialize)]
struct Plan {
    schema_version: String,
    page_id: String,
    page_size: Extent,
    nodes: Vec<PlanNode>,
}

#[derive(Debug, Deserialize)]
struct Extent {
    width: i64,
    height: i64,
}

#[derive(Debug, Deserialize)]
struct Bounds {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Debug, Deserialize)]
struct Transform {
    a: String,
    b: String,
    c: String,
    d: String,
    tx: i64,
    ty: i64,
}

impl Transform {
    fn is_identity(&self) -> bool {
        self.a == "1"
            && self.b == "0"
            && self.c == "0"
            && self.d == "1"
            && self.tx == 0
            && self.ty == 0
    }
}

#[derive(Debug, Deserialize)]
struct PlanNode {
    node_id: String,
    bounds: Bounds,
    transform: Transform,
    solid_fill_rgb: Option<[u8; 3]>,
    #[serde(flatten)]
    other_fields: BTreeMap<String, serde_json::Value>,
}

impl Plan {
    fn validate_geometry_only(&self) -> Result<(), String> {
        if self.schema_version != "chaptera.page-render-plan.v1" {
            return Err(format!("wrong Chaptera render-plan schema: {}", self.schema_version));
        }
        if self.page_id.is_empty() || self.page_size.width <= 0 || self.page_size.height <= 0 {
            return Err("missing page id or nonpositive page extent".to_owned());
        }

        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            if !ids.insert(&node.node_id) {
                return Err(format!("duplicate node id: {}", node.node_id));
            }
            if node.bounds.width <= 0 || node.bounds.height <= 0 {
                return Err(format!("nonpositive bounds for {}", node.node_id));
            }
            if !node.transform.is_identity() {
                return Err(format!("transformed node {} is not implemented", node.node_id));
            }
            if node.solid_fill_rgb.is_none() {
                return Err(format!("node {} has no admitted solid fill", node.node_id));
            }
            if let Some((name, _)) = node
                .other_fields
                .iter()
                .find(|(_, value)| !value.is_null())
            {
                return Err(format!(
                    "node {} has unimplemented plan field {}",
                    node.node_id, name
                ));
            }
        }
        Ok(())
    }
}

fn synthetic_plan_json(count: usize) -> String {
    let nodes: Vec<_> = (0..count)
        .map(|index| {
            let x = (10 + (index % 25) * 27) as i64 * 9_525;
            let y = (10 + ((index / 25) % 20) * 22) as i64 * 9_525;
            serde_json::json!({
                "node_id": format!("00000000-0000-0000-0000-{:012x}", index + 1),
                "bounds": {
                    "x": x, "y": y, "width": 16 * 9_525, "height": 12 * 9_525
                },
                "transform": {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0},
                "solid_fill_rgb": [
                    (20 + (index * 17 % 220)) as u8,
                    (40 + (index * 13 % 190)) as u8,
                    (60 + (index * 11 % 170)) as u8
                ]
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": "chaptera.page-render-plan.v1",
        "page_id": "00000000-0000-0000-0000-000000000001",
        "page_size": {
            "width": i64::from(VIEW_WIDTH) * 9_525,
            "height": i64::from(VIEW_HEIGHT) * 9_525
        },
        "nodes": nodes
    })
    .to_string()
}

fn emu_to_px(value: i64, zoom: f64) -> f64 {
    value as f64 / EMU_PER_CSS_PX * zoom
}

fn render_one(plan: &Plan, dpi: f64, zoom: f64, label: &str) -> serde_json::Value {
    plan.validate_geometry_only().expect("refuse unsupported render-plan semantics");
    let width = (f64::from(VIEW_WIDTH) * dpi).round() as u16;
    let height = (f64::from(VIEW_HEIGHT) * dpi).round() as u16;
    let options = RenderRootOptions {
        default_properties: Arc::new(default_property_set()),
        use_system_fonts: true,
        size_policy: WindowSizePolicy::User,
        size: PhysicalSize::new(u32::from(width), u32::from(height)),
        scale_factor: dpi,
        test_font: None,
    };
    let mut root = RenderRoot::new(
        NewWidget::new(Canvas::default()).with_tag(TAG).erased(),
        |_| (),
        options,
    );

    let scene_started = Instant::now();
    root.edit_widget_with_tag(TAG, |mut canvas| {
        Canvas::update_scene(&mut canvas, |_ctx, scene, _size| {
            let mut painter = Painter::new(scene);
            painter.fill_rect(
                Rect::new(
                    0.0,
                    0.0,
                    emu_to_px(plan.page_size.width, zoom),
                    emu_to_px(plan.page_size.height, zoom),
                ),
                PAGE_BACKGROUND,
            );
            for node in &plan.nodes {
                let bounds = &node.bounds;
                let rect = Rect::new(
                    emu_to_px(bounds.x, zoom),
                    emu_to_px(bounds.y, zoom),
                    emu_to_px(bounds.x + bounds.width, zoom),
                    emu_to_px(bounds.y + bounds.height, zoom),
                );
                let [r, g, b] = node.solid_fill_rgb.expect("geometry validated");
                painter.fill_rect(rect, Color::from_rgb8(r, g, b));
            }
        });
    });
    let scene_ms = scene_started.elapsed().as_secs_f64() * 1_000.0;
    let redraw_started = Instant::now();
    let (layers, _) = root.redraw();
    let redraw_ms = redraw_started.elapsed().as_secs_f64() * 1_000.0;

    let mut renderer = Renderer::new(0);
    let raster_started = Instant::now();
    renderer
        .draw_frame(&layers, width, height, dpi, BACKGROUND, true)
        .expect("real vello_cpu full frame");
    let raster_ms = raster_started.elapsed().as_secs_f64() * 1_000.0;

    let pixels = renderer.frame_pixels().to_vec();
    assert_eq!(pixels.len(), usize::from(width) * usize::from(height));
    assert!(
        pixels.iter().any(|pixel| *pixel != pixels[0]),
        "the Canvas must not be an empty/flat frame"
    );
    renderer
        .draw_frame(&layers, width, height, dpi, BACKGROUND, false)
        .expect("partial frame");
    assert_eq!(
        renderer.frame_pixels(),
        pixels.as_slice(),
        "unchanged plan: partial and full pixels differ"
    );

    if let Some(dir) = std::env::var_os("AQ_PUB_PROBE_DIR") {
        std::fs::create_dir_all(&dir).expect("create PNG evidence directory");
        let name = format!("{label}-dpi-{dpi:.1}-zoom-{zoom:.2}.png");
        std::fs::write(std::path::PathBuf::from(&dir).join(name), renderer.png())
            .expect("write renderer PNG");
    }

    serde_json::json!({
        "scope": "geometry-only; synthetic unless supplied externally",
        "plan_schema": plan.schema_version,
        "objects": plan.nodes.len(),
        "dpi": dpi,
        "zoom": zoom,
        "physical_width": width,
        "physical_height": height,
        "scene_build_ms_single": scene_ms,
        "render_root_redraw_ms_single": redraw_ms,
        "vello_cpu_draw_frame_ms_single": raster_ms,
        "full_partial_pixel_equal_unchanged": true,
        "not_proven": [
            "text", "fonts", "rotation", "images", "tables", "hit_test",
            "mouse_drag", "IME", "egui_pair", "p95", "60fps"
        ]
    })
}

fn save_receipt(name: &str, records: &[serde_json::Value]) {
    let receipt = serde_json::json!({
        "task": "AK-PUB-MASONRY-PROBE-01",
        "phase": "S0-geometry",
        "head_checkout_sha": std::env::var("GITHUB_SHA").ok(),
        "records": records
    });
    eprintln!("{}", serde_json::to_string_pretty(&receipt).unwrap());
    if let Some(dir) = std::env::var_os("AQ_PUB_PROBE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            std::path::PathBuf::from(dir).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .expect("write receipt");
    }
}

#[test]
fn pub_canvas_probe_s0_geometry_only() {
    let plan: Plan = serde_json::from_str(&synthetic_plan_json(100)).unwrap();
    let mut records = Vec::new();
    for dpi in [1.0, 1.5, 2.0] {
        for zoom in [0.25, 1.0, 4.0] {
            records.push(render_one(&plan, dpi, zoom, "s0"));
        }
    }
    assert_eq!(records.len(), 9);
    save_receipt("s0-geometry", &records);
}

#[test]
fn pub_canvas_probe_rejects_unsupported_content() {
    let mut json: serde_json::Value =
        serde_json::from_str(&synthetic_plan_json(1)).unwrap();
    json["nodes"][0]["text"] = serde_json::json!({"text": "Привет"});
    let plan: Plan = serde_json::from_value(json).unwrap();
    assert!(
        plan.validate_geometry_only().unwrap_err().contains("text"),
        "geometry-only probe must fail closed on text"
    );
}

#[test]
#[ignore = "manual geometry-only size sweep; not an S1-S4/Publisher acceptance"]
fn pub_canvas_probe_geometry_only_size_sweep() {
    let mut records = Vec::new();
    for count in [100, 500, 1_000, 5_000] {
        let plan: Plan = serde_json::from_str(&synthetic_plan_json(count)).unwrap();
        records.push(render_one(&plan, 1.0, 1.0, &format!("geometry-{count}")));
    }
    save_receipt("geometry-size-sweep", &records);
}

#[test]
#[ignore = "requires AK_PUB_RENDER_PLAN_JSON; refuses real plans with text/unsupported primitives"]
fn pub_canvas_probe_external_geometry_plan() {
    let path = std::env::var("AK_PUB_RENDER_PLAN_JSON")
        .expect("provide a Chaptera PageRenderPlanV1 JSON export");
    let data = std::fs::read_to_string(path).expect("read externally exported plan");
    let plan: Plan = serde_json::from_str(&data).expect("valid plan wire shape");
    save_receipt(
        "external-geometry-plan",
        &[render_one(&plan, 1.0, 1.0, "external")],
    );
}
