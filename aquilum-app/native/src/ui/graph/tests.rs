use super::*;

fn synthetic(count: usize) -> Graph {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let side = (count as f64).sqrt() * 0.6;
    let mut positions: Vec<[f32; 2]> = Vec::with_capacity(count);
    let mut edges = Vec::new();
    for node in 0..count {
        if node % 50 == 0 {
            positions.push([(next() * side) as f32, (next() * side) as f32]);
            continue;
        }
        let other = node - 1 - (next() * (node % 50) as f64) as usize;
        let [x, y] = positions[other];
        positions.push([x + (next() - 0.5) as f32 * 3.0, y + (next() - 0.5) as f32 * 3.0]);
        edges.push([other as u32, node as u32]);
        if next() < 0.3 {
            edges.push([(node - 1) as u32, node as u32]);
        }
    }
    let mut degrees = vec![0u32; count];
    for &[a, b] in &edges {
        degrees[a as usize] += 1;
        degrees[b as usize] += 1;
    }
    let days = vec![20_000.0f32; count];
    let paths = (0..count).map(|i| format!("Заметка {i}.md")).collect();
    Graph::new(positions, days.clone(), days, degrees, edges, paths)
}

#[test]
#[ignore]
fn million_notes_frame() {
    aq_trace::start();
    for count in [10_000usize, 1_000_000] {
        let started = Instant::now();
        let graph = Arc::new(synthetic(count));
        eprintln!("{count}: граф собран за {:?}", started.elapsed());
        let mut view = GraphView::build(Some(graph), Status::Ready, Display::default(), None, None);
        view.size = Size::new(1400.0, 900.0);
        view.scale_factor = 1.5;
        view.camera.jump_to(view.bounds(), view.size.width, view.size.height);
        for (name, zoom) in [("целиком", 1.0), ("ближе", 40.0)] {
            view.camera.move_to(view.camera.center_x, view.camera.center_y, view.camera.scale * zoom);
            let started = Instant::now();
            let frames = 5;
            for _ in 0..frames {
                view.camera.pan_by(3.0, 2.0);
                view.step(1.0 / 60.0);
                assert!(view.scene().is_some());
            }
            eprintln!("{count} {name}: кадр {:?}", started.elapsed() / frames);
            let started = Instant::now();
            let hit = view.node_at(Point::new(700.0, 450.0));
            eprintln!("{count} {name}: поиск узла {:?} -> {hit:?}", started.elapsed());
        }
    }
    if let Ok(out) = std::env::var("AQ_TRACE") {
        aq_trace::write(std::path::Path::new(&out)).unwrap();
    }
}

#[test]
fn graph_scene_smoke_across_zoom_and_dpi() {
    // Exercise the real GraphView.scene -> Frame.draw -> Band::disc path, not
    // only the standalone raster benchmark. Keep the fixture cheap in CI.
    let graph = Arc::new(synthetic(640));
    let mut view = GraphView::build(Some(graph), Status::Ready, Display::default(), None, None);
    view.size = Size::new(720.0, 480.0);
    for dpi in [1.0f64, 1.5] {
        view.scale_factor = dpi;
        view.camera.jump_to(view.bounds(), view.size.width, view.size.height);
        for zoom in [1.0, 5.0, 12.0] {
            view.camera.move_to(view.camera.center_x, view.camera.center_y, view.camera.scale * zoom);
            let (brush, scale) = view.scene().expect("graph frame should render");
            assert_eq!(scale, dpi);
            assert_eq!(brush.image.width, (view.size.width * dpi).round() as u32);
            assert_eq!(brush.image.height, (view.size.height * dpi).round() as u32);
            let pixels = view.surfaces.snapshot_rgba_for_test().expect("graph surface pixels");
            assert_eq!(pixels.len(), brush.image.width as usize * brush.image.height as usize * 4);
            if zoom == 1.0 {
                assert!(
                    pixels.chunks_exact(4).any(|pixel| pixel != &pixels[0..4]),
                    "the fitted graph must contain visible nodes and/or edges"
                );
            }
            if let Some(dir) = std::env::var_os("AQ_GRAPH_SCENE_DIR") {
                std::fs::create_dir_all(&dir).expect("create screenshot directory");
                let path = std::path::PathBuf::from(&dir).join(
                    format!("graph-dpi-{dpi:.1}-zoom-{zoom:.0}.png")
                );
                image::save_buffer(
                    &path, &pixels, brush.image.width, brush.image.height,
                    image::ColorType::Rgba8,
                )
                .expect("write graph scene screenshot");
            }
        }
        // Exercise the reduced-resolution frame path used while interacting.
        view.draft = true;
        let (brush, scale) = view.scene().expect("draft graph frame should render");
        assert_eq!(scale, dpi * DRAFT_SCALE);
        assert_eq!(brush.image.width, (view.size.width * scale).round() as u32);
        view.draft = false;
    }
}
