use super::*;

impl Host {
    pub(super) fn wake(&self) -> Option<Instant> {
        #[cfg(not(feature = "installed"))]
        if let Some(at) = self.replay.as_ref().and_then(crate::replay::Replay::wake) {
            return Some(at);
        }
        #[cfg(not(feature = "installed"))]
        if self.replay.as_ref().is_some_and(|r| !r.running()) {
            return Some(self.started + Duration::from_millis(2100));
        }
        #[cfg(feature = "memory-probe")]
        if let Some(at) = self.probe.as_ref().and_then(crate::memory_probe::Probe::wake) {
            return Some(at);
        }
        None
    }

    #[cfg(not(feature = "installed"))]
    pub(super) fn replay_scroll(&mut self, event_loop: &dyn ActiveEventLoop, now: Instant) {
        let settling = self.last_anim.is_some() || self.started.elapsed() < Duration::from_secs(2);
        if self.metrics.frames == 0 || settling && self.replay.as_ref().is_some_and(|r| !r.running()) {
            return;
        }
        let graph = self.replay.as_ref().is_some_and(crate::replay::Replay::is_graph);
        if graph {
            let Some(root) = &mut self.root else { return };
            if !self.app.graph_ready() {
                self.app.open_graph_for_replay(root);
                return;
            }
        }
        let Some(step) = self.replay.as_mut().and_then(|r| r.step(now)) else {
            if let Some(replay) = self.replay.as_ref().filter(|r| r.finished()) {
                replay.write();
                self.close(event_loop);
            }
            return;
        };
        let _zone = aq_trace::zone("прогон: колесо");
        let scale = self.scale();
        let target = if graph { self.app.graph_view() } else { self.app.editor_id() };
        let center = self.root.as_ref().zip(target).and_then(|(root, id)| root.get_widget(id)).map(|body| body.ctx().bounding_box().center());
        if let Some(center) = center {
            let height = f64::from(self.window().surface_size().height);
            self.pointer.position = winit::dpi::PhysicalPosition::new(center.x * scale * 0.8, (center.y * scale).clamp(height * 0.2, height * 0.8));
        }
        let state = self.pointer_state();
        match step {
            crate::replay::Step::Wheel(lines) => {
                // Developer-only image workload: a high-frequency smooth wheel
                // (and even a fixed PixelDelta) traversed only 3 distinct PNG
                // contents in a 10-source note. Seek to byte anchors of real
                // image paragraphs through the existing note layout instead.
                // The same deterministic dwell schedule is used for Eager and
                // Selective, with no installed feature path.
                static ANCHORS: std::sync::OnceLock<Option<Vec<usize>>> = std::sync::OnceLock::new();
                let anchors = ANCHORS.get_or_init(|| {
                    let raw = std::env::var("AKULA_EXPERIMENTAL_IMAGE_SCAN_BYTES").ok()?;
                    let values = raw.split(',').map(str::parse::<usize>)
                        .collect::<Result<Vec<_>, _>>().ok()?;
                    if values.is_empty() || values.len() > 64
                        || values.windows(2).any(|pair| pair[0] >= pair[1]) {
                        return None;
                    }
                    Some(values)
                });
                if let Some(anchors) = anchors {
                    // A transition-only seek was stale after asynchronous
                    // photo decode changed image paragraph heights. On every
                    // actual 8ms native replay tick, recheck the *active*
                    // anchor; App::image_scan_reveal is a cheap no-op when
                    // the latest layout places its top visibly inside the
                    // viewport with a 64px raster band. This handles anchors
                    // that cannot center at the start of a document. Never
                    // advance the 6s sweep clock.
                    let transition = self.replay.as_mut()
                        .and_then(|replay| replay.image_scan_target(now, anchors.len()));
                    let active = self.replay.as_ref()
                        .and_then(|replay| replay.image_scan_active(anchors.len()));
                    if let Some(index) = active {
                        if let Some(root) = self.root.as_mut() {
                            if let Some((from, to)) = self.app.image_scan_reveal(root, anchors[index]) {
                                static FIRST: std::sync::Once = std::sync::Once::new();
                                FIRST.call_once(|| eprintln!(
                                    "AKULA_IMAGE_ANCHOR_SCAN_APPLIED count={} schedule=forward-reverse",
                                    anchors.len()
                                ));
                                if transition.is_some() && (index % 4 == 0 || index + 1 == anchors.len()) {
                                    eprintln!(
                                        "AKULA_IMAGE_ANCHOR_SCAN_SEEK index={} byte={} from={:.0} to={:.0}",
                                        index, anchors[index], from, to
                                    );
                                }
                            }
                        }
                    }
                    // Each scheduled 8ms fling tick must request another
                    // native frame even while the same image anchor is held.
                    // Otherwise only the two anchor transitions render in
                    // the one-image case, and neither replay CPU metrics nor
                    // 32-frame cache diagnostics are meaningful. Keep source
                    // decoders and painting asynchronous, but render during
                    // the entire per-image dwell interval.
                    self.request_redraw();
                } else {
                    self.pointer_event(PointerEvent::Scroll(PointerScrollEvent {
                        pointer: PRIMARY_MOUSE, delta: ScrollDelta::LineDelta(0.0, lines), state
                    }));
                }
            }
            crate::replay::Step::Pinch(delta) => self.pinch(f64::from(delta)),
            crate::replay::Step::Focus => {
                if let Some(root) = &mut self.root {
                    self.app.toggle_focus_mode(root);
                }
            }
            crate::replay::Step::Pan(dx, dy) => {
                let delta = ScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(f64::from(dx), f64::from(dy)));
                self.pointer_event(PointerEvent::Scroll(PointerScrollEvent { pointer: PRIMARY_MOUSE, delta, state }));
            }
        }
        self.handle_signals(event_loop);
    }

    #[cfg(feature = "memory-probe")]
    pub(super) fn probe_step(&mut self, event_loop: &dyn ActiveEventLoop, now: Instant) {
        if self.metrics.frames == 0 {
            return;
        }
        let Some(probe) = &mut self.probe else { return };
        if probe.finished() {
            self.close(event_loop);
            return;
        }
        let Some(root) = &mut self.root else { return };
        if let Some(step) = probe.step(now).cloned() {
            if let crate::memory_probe::Step::Minimize(minimized) = step {
                self.window().set_minimized(minimized);
                return;
            }
            let opening_reader = matches!(&step, crate::memory_probe::Step::Reader(_));
            if opening_reader {
                self.reader_frame = Some(crate::memory_probe::ReaderFrame::begin());
            }
            self.app.probe(root, &step);
            if opening_reader {
                if self.app.probe_reader_active() {
                    if let Some(frame) = &mut self.reader_frame {
                        frame.opened();
                    }
                } else {
                    self.reader_frame = None;
                    eprintln!("READER_GUI_ERROR: Reader did not open; no first-frame metric");
                }
            }
            if matches!(&step, crate::memory_probe::Step::CloseReader) {
                self.reader_frame = None;
            }
            if let crate::memory_probe::Step::Mark(name) | crate::memory_probe::Step::Evict(name) = &step {
                probe.record(name, &self.app.probe_stats(root));
                // Compact machine-readable rows, avoiding costly Windows
                // memory breakdown on each intermediate navigation step.
                let state = self.app.probe_reader_state(root);
                probe.append_frame(&crate::memory_probe::navigation_sample(name, state.as_deref()));
            }
            self.handle_signals(event_loop);
            self.request_redraw();
        }
    }
}
