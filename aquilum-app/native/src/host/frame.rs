use super::*;

impl Host {
    pub(super) fn redraw(&mut self) {
        let _zone = aq_trace::zone("кадр");
        let t0 = Instant::now();
        let (Some(window), Some(output), Some(root)) = (self.window.clone(), self.output.as_mut(), self.root.as_mut()) else { return };
        let size = window.surface_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if let Err(err) = output.resize(size.width, size.height) {
            report(&err);
            return;
        }
        #[cfg(feature = "memory-probe")]
        crate::memory_probe::reset_frame();
        let (layers, access_update) = {
            let _zone = aq_trace::zone("сцена");
            root.run_rewrite_passes();
            self.app.restore_view(root);
            let painted = root.redraw();
            self.signals.borrow_mut().retain(|signal| !matches!(signal, RenderRootSignal::RequestRedraw));
            painted
        };
        let scene = t0.elapsed();
        if let (Some(update), Some(accessibility)) = (access_update, self.accessibility.as_mut()) {
            if self.args.trace {
                eprintln!("доступность: обновление дерева, узлов {}", update.nodes.len());
            }
            accessibility.update(update);
        }
        if self.app.holding() {
            return;
        }
        let (width, height) = (size.width.min(u32::from(u16::MAX)) as u16, size.height.min(u32::from(u16::MAX)) as u16);
        let scale = window.scale_factor() * self.app.scale();
        window.pre_present_notify();
        let background = ui::theme::background();
        if self.background != Some(background) {
            self.background = Some(background);
            set_background(&*window, background);
            let rgba = background.to_rgba8();
            output.set_background(u32::from_be_bytes([rgba.a, rgba.r, rgba.g, rgba.b]));
        }
        let full = std::mem::take(&mut self.full_next) || output.fresh();
        // Developer-only A/B falsifier: force rasterization rather than reuse
        // copied BGRA rows during a live replay. Normal and installed frames
        // do not check this environment variable.
        #[cfg(not(feature = "installed"))]
        let full = full || {
            static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            let enabled = *ENABLED.get_or_init(|| {
                std::env::var("AKULA_EXPERIMENTAL_IMAGE_FORCE_FULL_REDRAW").ok().as_deref() == Some("1")
            });
            let active = enabled && self.replay.as_ref().is_some_and(crate::replay::Replay::running);
            if active {
                static FIRST: std::sync::Once = std::sync::Once::new();
                FIRST.call_once(|| eprintln!("AKULA_IMAGE_REPLAY_FULL_REDRAW_APPLIED"));
            }
            active
        };
        #[cfg(all(target_os = "windows", not(feature = "installed")))]
        output.begin_output_stages(crate::render::Renderer::diagnostic_stages_enabled());
        let raster_start = Instant::now();
        let drawn = match self.renderer.draw(&layers, width, height, scale, background, full, output) {
            Ok(drawn) => drawn,
            Err(err) => {
                report(&format!("кадр не нарисован: {err}"));
                self.full_next = true;
                return;
            }
        };
        let raster = raster_start.elapsed();
        let present_start = Instant::now();
        #[cfg(feature = "memory-probe")]
        let presented = !drawn.is_empty();
        #[cfg(all(any(target_os = "macos", target_os = "linux"), feature = "installed"))]
        let startup_frame_presented = !drawn.is_empty();
        if !drawn.is_empty() {
            output.present(&drawn);
        }
        // Release the mutable output borrow before calling methods on Host.
        // The native timing snapshot belongs to the frame just submitted.
        #[cfg(all(target_os = "windows", not(feature = "installed")))]
        let native_output_stages = output.take_output_stages();
        self.layers = Some(layers);
        self.schedule_live();
        self.reveal();
        // ACK only after a nonempty frame is presented AND the native window
        // has been revealed; scheduling it before reveal could acknowledge a
        // hidden app which never reached its visible GUI startup path.
        #[cfg(all(any(target_os = "macos", target_os = "linux"), feature = "installed"))]
        if startup_frame_presented {
            crate::updater::on_first_presented_frame();
        }
        let present = present_start.elapsed();
        #[cfg(feature = "memory-probe")]
        if presented && crate::memory_probe::take_text_painted()
            && let Some(sample) = self.reader_frame.take()
        {
            let ms = |time: Duration| time.as_secs_f64() * 1000.0;
            let line = sample.finish(ms(scene), ms(raster), ms(present));
            eprintln!("{line}");
            if let Some(probe) = &self.probe {
                probe.append_frame(&line);
            }
        }
        // Capture whole-process frame stages (including the first paint) and
        // replay-only stages from the same snapshot. Both paths use identical
        // stage clocks in baseline and mip modes when explicitly enabled.
        #[cfg(not(feature = "installed"))]
        let diagnostic_stage = if crate::render::Renderer::diagnostic_stages_enabled() {
            let mut stage = self.renderer.diagnostic_stages();
            stage.scene_ms = scene.as_secs_f64() * 1000.0;
            stage.renderer_draw_ms = raster.as_secs_f64() * 1000.0;
            stage.output_present_ms = present.as_secs_f64() * 1000.0;
            #[cfg(target_os = "windows")]
            {
                let native = native_output_stages;
                stage.output_map_ms = native.map_ms;
                stage.output_unmap_ms = native.unmap_ms;
                stage.output_copy_submit_ms = native.copy_submit_ms;
                stage.output_present_call_ms = native.present_call_ms;
                stage.output_copy_bytes = native.copy_bytes;
                stage.output_copy_rects = native.copy_rects;
                stage.output_map_calls = native.map_calls;
            }
            Some(stage)
        } else {
            None
        };
        // Replay sample = a non-empty CPU frame handed to the output surface.
        // No-op redraws must not inflate the diagnostic "present" count.
        #[cfg(not(feature = "installed"))]
        if !drawn.is_empty() {
            if let Some(replay) = &mut self.replay {
                if let Some(stage) = diagnostic_stage {
                    replay.frame_with_stage(Instant::now(), t0.elapsed(), stage);
                } else {
                    replay.frame(Instant::now(), t0.elapsed());
                }
            }
        }
        if self.metrics.frame(self.started, [scene, raster, present, t0.elapsed()]) {
            self.show_title();
        }
        #[cfg(not(feature = "installed"))]
        if let Some(stage) = diagnostic_stage {
            self.metrics.record_stage(stage);
        }
        if self.metrics.frames == 1 {
            self.metrics.write();
        }
    }

    /// Кадр живых слоёв: без сцены Masonry, только анимация поверх кэшей.
    pub(super) fn live_tick(&mut self) {
        let (Some(output), Some(layers)) = (self.output.as_mut(), self.layers.as_ref()) else { return };
        match self.renderer.live(layers, output) {
            Ok(areas) if !areas.is_empty() => output.present(&areas),
            Ok(_) => {}
            Err(err) => {
                report(&format!("живой слой не нарисован: {err}"));
                self.full_next = true;
            }
        }
        self.schedule_live();
    }

    fn schedule_live(&mut self) {
        let next = self.renderer.next_live().filter(|_| !self.occluded);
        self.live_at = next.map(|wait| Instant::now() + wait);
    }

    /// Анимационный тик: проходы обновления Masonry без растеризации.
    pub(super) fn anim_tick(&mut self, event_loop: &dyn ActiveEventLoop, vblank: Instant, previous: Option<Instant>) {
        let _zone = aq_trace::zone("тик анимации");
        let Some(root) = &mut self.root else { return };
        let since = |t: Instant| vblank.saturating_duration_since(t);
        let elapsed = match self.last_anim.take() {
            Some(t) => since(t),
            None => previous.map_or(Duration::ZERO, |t| since(t).min(Duration::from_micros(16_667))),
        };
        root.handle_window_event(MasonryWindowEvent::AnimFrame(elapsed));
        self.last_anim = root.needs_anim().then_some(vblank);
        self.metrics.anim_ticks += 1;
        self.handle_signals(event_loop);
    }
}
